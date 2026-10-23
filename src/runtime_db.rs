use crate::config::{TideConfig, sqlite_url_creates_file};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use tideorm::internal::{ConnectionTrait, OrmStatement as Statement};
use tideorm::prelude::Database;

/// The ledger TideORM's `Seeder` records runs in, so `db check` creates the
/// table seeding actually writes.
pub const DEFAULT_SEEDERS_TABLE: &str = "_seeds";

#[derive(Debug, Clone)]
pub struct ColumnDetails {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub key: Option<String>,
    pub default: Option<String>,
    pub extra: Option<String>,
}

#[derive(Debug, Clone)]
pub struct IndexDetails {
    pub name: String,
    pub columns: Vec<String>,
    pub unique: bool,
}

#[derive(Debug, Clone)]
pub struct ForeignKeyDetails {
    pub column: String,
    pub references_table: String,
    pub references_column: String,
}

pub async fn connect(config: &TideConfig) -> Result<Database, String> {
    create_sqlite_file(config).await?;
    connect_with_url(&config.database.connection_url()).await
}

/// Connect with a pool of one connection, so every statement shares one
/// session: `SET FOREIGN_KEY_CHECKS` and `PRAGMA foreign_keys` hold only on
/// the connection they ran on, and a larger pool may run the next statement
/// on another.
pub(crate) async fn connect_single_session(config: &TideConfig) -> Result<Database, String> {
    create_sqlite_file(config).await?;
    let url = config.database.connection_url();
    Database::builder()
        .url(&url)
        .max_connections(1)
        .build()
        .await
        .map_err(|error| crate::utils::masked_connection_error(error, &url))
}

/// A SQLite database is a file, which has to exist before a connection opens; an
/// in-memory one has none.
async fn create_sqlite_file(config: &TideConfig) -> Result<(), String> {
    if config.database.driver_name() == "sqlite"
        && config
            .database
            .url
            .as_deref()
            .is_none_or(sqlite_url_creates_file)
        && let Some(sqlite_path) = config.database.sqlite_file()
    {
        create_database(config, &sqlite_path).await?;
    }
    Ok(())
}

pub async fn ping(config: &TideConfig) -> Result<(), String> {
    if config.database.driver_name() == "sqlite"
        && let Some(path) = config.database.sqlite_file()
        && !Path::new(&path).exists()
    {
        return Err(format!("SQLite database file does not exist: {path}"));
    }

    let db = connect_with_url(&config.database.try_connection_url()?).await?;
    db.ping()
        .await
        .map(|_| ())
        .map_err(|error| error.to_string())
}

pub async fn query_json(config: &TideConfig, sql: &str) -> Result<Vec<Value>, String> {
    let db = connect(config).await?;
    query_json_on_db(&db, sql).await
}

pub async fn execute_on_db(db: &Database, sql: &str) -> Result<u64, String> {
    let connection = db
        .__internal_connection()
        .map_err(|error| error.to_string())?;

    connection
        .execute_unprepared(sql)
        .await
        .map(|result| result.rows_affected())
        .map_err(|error| error.to_string())
}

pub async fn query_json_on_db(db: &Database, sql: &str) -> Result<Vec<Value>, String> {
    let connection = db
        .__internal_connection()
        .map_err(|error| error.to_string())?;
    let backend = connection.get_database_backend();
    let statement = Statement::from_string(backend, sql.to_string());
    let rows = connection
        .query_all_raw(statement)
        .await
        .map_err(|error| error.to_string())?;

    let mut values = Vec::new();
    for row in rows {
        let mut object = serde_json::Map::new();

        for column_name in row.column_names() {
            // Integers must be probed before booleans: the SQLite bool decoder
            // (and MySQL's, for TINYINT) accepts any integer, so a leading bool
            // probe turns every integer column into `true`/`false`.
            let json_value = if let Ok(value) = row.try_get::<Option<i64>>("", &column_name) {
                value.map(Value::from).unwrap_or(Value::Null)
            } else if let Ok(value) = row.try_get::<Option<bool>>("", &column_name) {
                value.map(Value::from).unwrap_or(Value::Null)
            } else if let Ok(value) = row.try_get::<Option<f64>>("", &column_name) {
                value.map(Value::from).unwrap_or(Value::Null)
            } else if let Ok(value) = row.try_get::<Option<String>>("", &column_name) {
                value.map(Value::from).unwrap_or(Value::Null)
            } else {
                Value::Null
            };

            object.insert(column_name.to_string(), json_value);
        }

        values.push(Value::Object(object));
    }

    Ok(values)
}

pub async fn list_tables(config: &TideConfig) -> Result<Vec<String>, String> {
    list_names(config, &tables_sql(config.database.driver_name())?).await
}

/// Every view in the database. `wipe_tables` drops them with `DROP VIEW`: MySQL refuses
/// `DROP TABLE` on a view, and SQLite and MySQL keep a view whose tables are dropped.
async fn list_views(config: &TideConfig) -> Result<Vec<String>, String> {
    list_names(config, &views_sql(config.database.driver_name())?).await
}

/// The sequences of the current database that no table owns: PostgreSQL's
/// and MariaDB's. A table's own sequence goes with it.
async fn list_sequences(config: &TideConfig) -> Result<Vec<String>, String> {
    match config.database.driver_name() {
        "postgres" => list_names(config, &postgres_relations_sql("'S'")).await,
        "mysql" => {
            list_names(
                config,
                "SELECT table_name AS table_name FROM information_schema.tables WHERE table_schema = DATABASE() AND table_type = 'SEQUENCE' ORDER BY table_name",
            )
            .await
        }
        _ => Ok(Vec::new()),
    }
}

/// The base tables of the current database, as `table_name`: MariaDB's
/// system-versioned ones too.
///
/// Aliased: MySQL 8 labels information_schema columns in upper case, and an unmatched
/// label made every listing - and so every wipe - empty.
fn tables_sql(driver: &str) -> Result<String, String> {
    match driver {
        "sqlite" => Ok(
            "SELECT name AS table_name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name".to_string(),
        ),
        "postgres" => Ok(postgres_relations_sql("'r', 'p'")),
        "mysql" => Ok(
            "SELECT table_name AS table_name FROM information_schema.tables WHERE table_schema = DATABASE() AND table_type IN ('BASE TABLE', 'SYSTEM VERSIONED') ORDER BY table_name".to_string(),
        ),
        driver => Err(format!("Unsupported database driver: {}", driver)),
    }
}

/// The views of the current database, as `table_name`.
fn views_sql(driver: &str) -> Result<String, String> {
    match driver {
        "sqlite" => Ok(
            "SELECT name AS table_name FROM sqlite_master WHERE type = 'view' ORDER BY name"
                .to_string(),
        ),
        "postgres" => Ok(postgres_relations_sql("'v'")),
        "mysql" => Ok(
            "SELECT table_name AS table_name FROM information_schema.views WHERE table_schema = DATABASE() ORDER BY table_name".to_string(),
        ),
        driver => Err(format!("Unsupported database driver: {}", driver)),
    }
}

/// The user-schema relations of `kinds` (`pg_class.relkind`), as
/// `table_name`, but an extension's: PostgreSQL refuses to drop what an
/// extension installed (`pg_stat_statements`, PostGIS's `spatial_ref_sys`)
/// except with the extension. Public-schema names stay unqualified for display;
/// names in other schemas are returned as `schema.name` for inspection and wipe.
fn postgres_relations_sql(kinds: &str) -> String {
    format!(
        "SELECT CASE WHEN n.nspname = 'public' THEN c.relname ELSE n.nspname || '.' || c.relname END AS table_name FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE n.nspname NOT IN ('pg_catalog', 'information_schema') AND left(n.nspname, 3) <> 'pg_' AND c.relkind IN ({kinds}) AND NOT {} ORDER BY n.nspname, c.relname",
        postgres_extension_member("pg_class", "c.oid")
    )
}

/// Whether the object `oid` of the catalog `catalog` belongs to an extension.
fn postgres_extension_member(catalog: &str, oid: &str) -> String {
    format!(
        "EXISTS (SELECT 1 FROM pg_depend d WHERE d.classid = '{catalog}'::regclass AND d.objid = {oid} AND d.deptype = 'e')"
    )
}

async fn list_names(config: &TideConfig, sql: &str) -> Result<Vec<String>, String> {
    let rows = query_json(config, sql).await?;
    Ok(rows
        .into_iter()
        .filter_map(|row| string_field(&row, &["table_name"]))
        .collect())
}

pub async fn table_columns(
    config: &TideConfig,
    table_name: &str,
) -> Result<Vec<ColumnDetails>, String> {
    let rows = query_json(config, &columns_sql(config, table_name)?).await?;
    let mut columns = rows
        .into_iter()
        .map(|row| column_from_row(config, &row))
        .collect::<Vec<_>>();

    if columns.is_empty() {
        return Err(format!("Table or view not found: {table_name}"));
    }

    if config.database.driver_name() == "sqlite" {
        let foreign_keys = table_foreign_keys(config, table_name).await?;
        for column in &mut columns {
            if column.key.as_deref() != Some("PRI")
                && foreign_keys.iter().any(|fk| fk.column == column.name)
            {
                column.key = Some("FK".to_string());
            }
        }
    }

    Ok(columns)
}

pub async fn table_indexes(
    config: &TideConfig,
    table_name: &str,
) -> Result<Vec<IndexDetails>, String> {
    match config.database.driver_name() {
        "sqlite" => sqlite_indexes(config, table_name).await,
        "postgres" => postgres_indexes(config, table_name).await,
        "mysql" => mysql_indexes(config, table_name).await,
        driver => Err(format!("Unsupported database driver: {}", driver)),
    }
}

pub async fn table_foreign_keys(
    config: &TideConfig,
    table_name: &str,
) -> Result<Vec<ForeignKeyDetails>, String> {
    let sql = match config.database.driver_name() {
        "sqlite" => format!(
            "PRAGMA foreign_key_list({})",
            quoted_identifier(config, table_name)
        ),
        "postgres" => format!(
            "SELECT kcu.column_name, ccu.table_name AS references_table, ccu.column_name AS references_column \
             FROM information_schema.table_constraints tc \
             JOIN information_schema.key_column_usage kcu \
               ON tc.constraint_name = kcu.constraint_name \
              AND tc.table_schema = kcu.table_schema \
             JOIN information_schema.constraint_column_usage ccu \
               ON ccu.constraint_name = tc.constraint_name \
              AND ccu.table_schema = tc.table_schema \
             WHERE tc.constraint_type = 'FOREIGN KEY' \
               AND {} \
             ORDER BY kcu.ordinal_position",
            postgres_table_filter("tc", table_name)
        ),
        "mysql" => format!(
            "SELECT COLUMN_NAME AS column_name, REFERENCED_TABLE_NAME AS references_table, REFERENCED_COLUMN_NAME AS references_column \
             FROM information_schema.KEY_COLUMN_USAGE \
             WHERE TABLE_SCHEMA = DATABASE() \
               AND TABLE_NAME = {} \
               AND REFERENCED_TABLE_NAME IS NOT NULL \
             ORDER BY ORDINAL_POSITION",
            sql_string(table_name)
        ),
        driver => return Err(format!("Unsupported database driver: {}", driver)),
    };

    let rows = query_json(config, &sql).await?;
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            Some(ForeignKeyDetails {
                column: string_field(&row, &["column_name", "from"])?,
                references_table: string_field(&row, &["references_table", "table"])?,
                references_column: string_field(&row, &["references_column", "to"])?,
            })
        })
        .collect())
}

pub async fn wipe_tables(config: &TideConfig, drop_types: bool) -> Result<(), String> {
    let db = connect_single_session(config).await?;
    let views = list_views(config).await?;
    let tables = list_tables(config).await?;
    let sequences = list_sequences(config).await?;
    let joined = |names: &[String]| {
        names
            .iter()
            .map(|name| quoted_relation_identifier(config, name))
            .collect::<Vec<_>>()
            .join(", ")
    };

    match config.database.driver_name() {
        "sqlite" => {
            execute_on_db(&db, "PRAGMA foreign_keys = OFF;").await?;
            for view in views {
                execute_on_db(
                    &db,
                    &format!("DROP VIEW IF EXISTS {}", quoted_identifier(config, &view)),
                )
                .await?;
            }
            for table in tables {
                execute_on_db(
                    &db,
                    &format!("DROP TABLE IF EXISTS {}", quoted_identifier(config, &table)),
                )
                .await?;
            }
            let _ = execute_on_db(&db, "DELETE FROM sqlite_sequence;").await;
            execute_on_db(&db, "PRAGMA foreign_keys = ON;").await?;
        }
        "postgres" => {
            // CASCADE takes the views built on a table along, but not one that reads
            // no table.
            if !views.is_empty() {
                execute_on_db(
                    &db,
                    &format!("DROP VIEW IF EXISTS {} CASCADE", joined(&views)),
                )
                .await?;
            }
            if !tables.is_empty() {
                execute_on_db(
                    &db,
                    &format!("DROP TABLE IF EXISTS {} CASCADE", joined(&tables)),
                )
                .await?;
            }
            if !sequences.is_empty() {
                execute_on_db(
                    &db,
                    &format!("DROP SEQUENCE IF EXISTS {} CASCADE", joined(&sequences)),
                )
                .await?;
            }

            if drop_types {
                let types = query_json(
                    config,
                    &format!(
                        "SELECT CASE WHEN n.nspname = 'public' THEN t.typname ELSE n.nspname || '.' || t.typname END AS type_name FROM pg_type t JOIN pg_namespace n ON n.oid = t.typnamespace WHERE n.nspname NOT IN ('pg_catalog', 'information_schema') AND left(n.nspname, 3) <> 'pg_' AND t.typtype = 'e' AND NOT {} ORDER BY n.nspname, t.typname",
                        postgres_extension_member("pg_type", "t.oid")
                    ),
                )
                .await?;

                for row in types {
                    if let Some(type_name) = string_field(&row, &["type_name"]) {
                        execute_on_db(
                            &db,
                            &format!(
                                "DROP TYPE IF EXISTS {} CASCADE",
                                quoted_relation_identifier(config, &type_name)
                            ),
                        )
                        .await?;
                    }
                }
            }
        }
        "mysql" => {
            execute_on_db(&db, "SET FOREIGN_KEY_CHECKS = 0").await?;
            if !views.is_empty() {
                execute_on_db(&db, &format!("DROP VIEW IF EXISTS {}", joined(&views))).await?;
            }
            if !tables.is_empty() {
                execute_on_db(&db, &format!("DROP TABLE IF EXISTS {}", joined(&tables))).await?;
            }
            if !sequences.is_empty() {
                execute_on_db(
                    &db,
                    &format!("DROP SEQUENCE IF EXISTS {}", joined(&sequences)),
                )
                .await?;
            }
            execute_on_db(&db, "SET FOREIGN_KEY_CHECKS = 1").await?;
        }
        driver => return Err(format!("Unsupported database driver: {}", driver)),
    }

    Ok(())
}

/// Create the target database.
///
/// For SQLite `database_name` is the database *file path* to create; callers
/// decide whether that is a user-supplied target or the configured
/// `sqlite_path`.
pub async fn create_database(config: &TideConfig, database_name: &str) -> Result<(), String> {
    match config.database.driver_name() {
        "sqlite" => {
            let path = sqlite_target_path(database_name)?;
            if let Some(parent) = Path::new(path).parent()
                && !parent.as_os_str().is_empty()
            {
                fs::create_dir_all(parent).map_err(|error| error.to_string())?;
            }
            let _file = fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(false)
                .open(path)
                .map_err(|error| error.to_string())?;
            Ok(())
        }
        "postgres" | "mysql" => {
            let admin_db = connect_with_url(&admin_connection_url(config)?).await?;
            execute_on_db(
                &admin_db,
                &format!(
                    "CREATE DATABASE {}",
                    quoted_identifier(config, database_name)
                ),
            )
            .await
            .map(|_| ())
        }
        driver => Err(format!("Unsupported database driver: {}", driver)),
    }
}

pub async fn ensure_metadata_tables(
    config: &TideConfig,
    migration_table: &str,
) -> Result<(), String> {
    let db = connect(config).await?;
    ensure_metadata_tables_on_db(&db, migration_table).await
}

pub async fn ensure_metadata_tables_on_db(
    db: &Database,
    migration_table: &str,
) -> Result<(), String> {
    ensure_migration_table_on_db(db, migration_table).await?;
    ensure_seeders_table_on_db(db).await
}

pub async fn ensure_migration_table(
    config: &TideConfig,
    migration_table: &str,
) -> Result<(), String> {
    let db = connect(config).await?;
    ensure_migration_table_on_db(&db, migration_table).await
}

/// Create the migrations ledger if it is missing, as TideORM's migrator does.
pub async fn ensure_migration_table_on_db(
    db: &Database,
    migration_table: &str,
) -> Result<(), String> {
    tideorm::migration::__ensure_migration_ledger(db, migration_table)
        .await
        .map_err(|error| error.to_string())
}

/// Create the seed ledger if it is missing, as TideORM's seeder does.
pub async fn ensure_seeders_table_on_db(db: &Database) -> Result<(), String> {
    tideorm::migration::__ensure_seed_ledger(db)
        .await
        .map_err(|error| error.to_string())
}

/// Forget every seed the ledger records, so the next `db seed` runs them all:
/// no migration owns the ledger, so it outlives a reset of the tables it
/// describes.
pub async fn clear_seed_ledger(config: &TideConfig) -> Result<(), String> {
    let db = connect(config).await?;
    ensure_seeders_table_on_db(&db).await?;
    execute_on_db(
        &db,
        &format!(
            "DELETE FROM {}",
            quoted_identifier(config, DEFAULT_SEEDERS_TABLE)
        ),
    )
    .await?;
    Ok(())
}

/// Drop the target database.
///
/// For SQLite `database_name` is the database *file path* to delete; callers
/// decide whether that is a user-supplied target or the configured
/// `sqlite_path`.
pub async fn drop_database(config: &TideConfig, database_name: &str) -> Result<(), String> {
    match config.database.driver_name() {
        "sqlite" => {
            let path = sqlite_target_path(database_name)?;
            if Path::new(path).exists() {
                fs::remove_file(path).map_err(|error| error.to_string())?;
            }
            Ok(())
        }
        "postgres" => {
            let admin_db = connect_with_url(&admin_connection_url(config)?).await?;
            let terminate_sql = format!(
                "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = {} AND pid <> pg_backend_pid()",
                sql_string(database_name)
            );
            let _ = execute_on_db(&admin_db, &terminate_sql).await;
            execute_on_db(
                &admin_db,
                &format!(
                    "DROP DATABASE IF EXISTS {}",
                    quoted_identifier(config, database_name)
                ),
            )
            .await
            .map(|_| ())
        }
        "mysql" => {
            let admin_db = connect_with_url(&admin_connection_url(config)?).await?;
            execute_on_db(
                &admin_db,
                &format!(
                    "DROP DATABASE IF EXISTS {}",
                    quoted_identifier(config, database_name)
                ),
            )
            .await
            .map(|_| ())
        }
        driver => Err(format!("Unsupported database driver: {}", driver)),
    }
}

async fn connect_with_url(url: &str) -> Result<Database, String> {
    Database::connect(url)
        .await
        .map_err(|error| crate::utils::masked_connection_error(error, url))
}

fn admin_connection_url(config: &TideConfig) -> Result<String, String> {
    match config.database.driver_name() {
        "postgres" => override_database_in_url(config, "postgres"),
        "mysql" => override_database_in_url(config, "mysql"),
        "sqlite" => Ok(config.database.connection_url()),
        driver => Err(format!("Unsupported database driver: {}", driver)),
    }
}

fn override_database_in_url(config: &TideConfig, database_name: &str) -> Result<String, String> {
    if config.database.url.is_some() {
        let url = config.database.connection_url();
        if let Some((scheme, remainder)) = url.split_once("://") {
            let authority_end = remainder.find(['/', '?', '#']).unwrap_or(remainder.len());
            let authority = &remainder[..authority_end];
            if !scheme.is_empty() && !authority.is_empty() {
                let suffix = &remainder[authority_end..];
                let query_or_fragment = if suffix.starts_with('/') {
                    suffix.find(['?', '#']).map_or("", |index| &suffix[index..])
                } else {
                    suffix
                };
                return Ok(format!(
                    "{}://{}/{}{}",
                    scheme, authority, database_name, query_or_fragment
                ));
            }
        }

        return Err(
            "Failed to derive admin database URL from configured connection URL".to_string(),
        );
    }

    config.database.server_url_for_database(database_name)
}

fn columns_sql(config: &TideConfig, table_name: &str) -> Result<String, String> {
    Ok(match config.database.driver_name() {
        "sqlite" => format!(
            "PRAGMA table_info({})",
            quoted_identifier(config, table_name)
        ),
        "postgres" => format!(
            "SELECT c.column_name, c.data_type, (c.is_nullable = 'YES') AS nullable, \
             CASE \
               WHEN EXISTS ( \
                 SELECT 1 FROM information_schema.table_constraints tc \
                 JOIN information_schema.key_column_usage kcu \
                   ON tc.constraint_name = kcu.constraint_name \
                  AND tc.table_schema = kcu.table_schema \
                 WHERE tc.constraint_type = 'PRIMARY KEY' \
                   AND tc.table_schema = c.table_schema \
                   AND tc.table_name = c.table_name \
                   AND kcu.column_name = c.column_name \
               ) THEN 'PRI' \
               WHEN EXISTS ( \
                 SELECT 1 FROM information_schema.table_constraints tc \
                 JOIN information_schema.key_column_usage kcu \
                   ON tc.constraint_name = kcu.constraint_name \
                  AND tc.table_schema = kcu.table_schema \
                 WHERE tc.constraint_type = 'UNIQUE' \
                   AND tc.table_schema = c.table_schema \
                   AND tc.table_name = c.table_name \
                   AND kcu.column_name = c.column_name \
               ) THEN 'UNI' \
               WHEN EXISTS ( \
                 SELECT 1 FROM information_schema.table_constraints tc \
                 JOIN information_schema.key_column_usage kcu \
                   ON tc.constraint_name = kcu.constraint_name \
                  AND tc.table_schema = kcu.table_schema \
                 WHERE tc.constraint_type = 'FOREIGN KEY' \
                   AND tc.table_schema = c.table_schema \
                   AND tc.table_name = c.table_name \
                   AND kcu.column_name = c.column_name \
               ) THEN 'FK' \
               ELSE NULL \
             END AS key_name, \
             c.column_default AS default_value, \
             NULL::TEXT AS extra \
             FROM information_schema.columns c \
             WHERE {} \
             ORDER BY c.ordinal_position",
            postgres_table_filter("c", table_name)
        ),
        "mysql" => format!(
            "SELECT COLUMN_NAME AS column_name, COLUMN_TYPE AS data_type, (IS_NULLABLE = 'YES') AS nullable, \
             NULLIF(COLUMN_KEY, '') AS key_name, COLUMN_DEFAULT AS default_value, NULLIF(EXTRA, '') AS extra \
             FROM information_schema.columns \
             WHERE table_schema = DATABASE() AND table_name = {} \
             ORDER BY ORDINAL_POSITION",
            sql_string(table_name)
        ),
        driver => return Err(format!("Unsupported database driver: {}", driver)),
    })
}

async fn sqlite_indexes(
    config: &TideConfig,
    table_name: &str,
) -> Result<Vec<IndexDetails>, String> {
    let rows = query_json(
        config,
        &format!(
            "PRAGMA index_list({})",
            quoted_identifier(config, table_name)
        ),
    )
    .await?;

    let mut indexes = Vec::new();
    for row in rows {
        let Some(index_name) = string_field(&row, &["name"]) else {
            continue;
        };
        let columns = query_json(
            config,
            &format!(
                "PRAGMA index_info({})",
                quoted_identifier(config, &index_name)
            ),
        )
        .await?
        .into_iter()
        .filter_map(|column| string_field(&column, &["name"]))
        .collect::<Vec<_>>();

        indexes.push(IndexDetails {
            name: index_name,
            columns,
            unique: bool_field(&row, &["unique"]),
        });
    }

    Ok(indexes)
}

async fn postgres_indexes(
    config: &TideConfig,
    table_name: &str,
) -> Result<Vec<IndexDetails>, String> {
    let (schema, table) = postgres_table_parts(table_name);
    let rows = query_json(
        config,
        &format!(
            "SELECT indexname, indexdef FROM pg_indexes WHERE schemaname = {} AND tablename = {} ORDER BY indexname",
            sql_string(schema),
            sql_string(table)
        ),
    )
    .await?;

    let capture =
        regex::Regex::new(r"\((?P<columns>[^\)]*)\)").map_err(|error| error.to_string())?;

    Ok(rows
        .into_iter()
        .filter_map(|row| {
            let index_name = string_field(&row, &["indexname"])?;
            let index_def = string_field(&row, &["indexdef"])?;
            let columns = capture
                .captures(&index_def)
                .and_then(|captures| captures.name("columns"))
                .map(|value| {
                    value
                        .as_str()
                        .split(',')
                        .map(|column| column.trim().trim_matches('"').to_string())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();

            Some(IndexDetails {
                name: index_name,
                columns,
                unique: index_def.contains("UNIQUE INDEX"),
            })
        })
        .collect())
}

async fn mysql_indexes(config: &TideConfig, table_name: &str) -> Result<Vec<IndexDetails>, String> {
    let rows = query_json(
        config,
        &format!("SHOW INDEX FROM {}", quoted_identifier(config, table_name)),
    )
    .await?;

    let mut grouped = BTreeMap::<String, IndexDetails>::new();
    for row in rows {
        let Some(index_name) = string_field(&row, &["Key_name"]) else {
            continue;
        };

        let column_name = string_field(&row, &["Column_name"]).unwrap_or_default();
        let unique = int_field(&row, &["Non_unique"]).unwrap_or(1) == 0;

        let entry = grouped
            .entry(index_name.clone())
            .or_insert_with(|| IndexDetails {
                name: index_name.clone(),
                columns: Vec::new(),
                unique,
            });
        if !column_name.is_empty() {
            entry.columns.push(column_name);
        }
    }

    Ok(grouped.into_values().collect())
}

fn column_from_row(config: &TideConfig, row: &Value) -> ColumnDetails {
    if config.database.driver_name() == "sqlite" {
        let is_primary = int_field(row, &["pk"]).unwrap_or(0) > 0;
        let nullable = int_field(row, &["notnull"]).unwrap_or(0) == 0;

        return ColumnDetails {
            name: string_field(row, &["name", "column_name"]).unwrap_or_default(),
            data_type: string_field(row, &["type", "data_type"]).unwrap_or_default(),
            nullable,
            key: if is_primary {
                Some("PRI".to_string())
            } else {
                None
            },
            default: string_field(row, &["dflt_value", "default_value"]),
            extra: None,
        };
    }

    ColumnDetails {
        name: string_field(row, &["column_name", "name"]).unwrap_or_default(),
        data_type: string_field(row, &["data_type", "type"]).unwrap_or_default(),
        nullable: bool_field(row, &["nullable"]),
        key: string_field(row, &["key_name", "column_key", "key"]),
        default: string_field(row, &["default_value", "column_default", "dflt_value"]),
        extra: string_field(row, &["extra"]),
    }
}

/// Validate the SQLite database file path a command is about to act on.
///
/// Relative paths stay relative to the current directory, which is how the
/// configured `sqlite_path` is interpreted everywhere else.
fn sqlite_target_path(database_name: &str) -> Result<&str, String> {
    let path = database_name.trim();
    if path.is_empty() {
        return Err("SQLite database path not specified".to_string());
    }

    Ok(path)
}

pub(crate) fn quoted_identifier(config: &TideConfig, identifier: &str) -> String {
    match config.database.driver_name() {
        "mysql" => format!("`{}`", identifier.replace('`', "``")),
        _ => format!("\"{}\"", identifier.replace('"', "\"\"")),
    }
}

fn quoted_relation_identifier(config: &TideConfig, identifier: &str) -> String {
    if config.database.driver_name() == "postgres" {
        let (schema, name) = postgres_table_parts(identifier);
        return format!(
            "{}.{}",
            quoted_identifier(config, schema),
            quoted_identifier(config, name)
        );
    }
    quoted_identifier(config, identifier)
}

/// The schema of a PostgreSQL table, `public` unless it names one, and its
/// bare name.
fn postgres_table_parts(table_name: &str) -> (&str, &str) {
    let (schema, table) = crate::utils::split_table(table_name);
    (schema.unwrap_or("public"), table)
}

fn postgres_table_filter(alias: &str, table_name: &str) -> String {
    let (schema, table) = postgres_table_parts(table_name);
    format!(
        "{alias}.table_schema = {} AND {alias}.table_name = {}",
        sql_string(schema),
        sql_string(table)
    )
}

fn sql_string(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn string_field(row: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| match row.get(*key) {
        Some(Value::String(value)) if !value.is_empty() => Some(value.clone()),
        Some(Value::Number(value)) => Some(value.to_string()),
        Some(Value::Bool(value)) => Some(value.to_string()),
        _ => None,
    })
}

fn bool_field(row: &Value, keys: &[&str]) -> bool {
    keys.iter().any(|key| match row.get(*key) {
        Some(Value::Bool(value)) => *value,
        Some(Value::Number(value)) => value.as_i64().unwrap_or_default() != 0,
        Some(Value::String(value)) => matches!(value.as_str(), "1" | "t" | "true" | "YES" | "yes"),
        _ => false,
    })
}

fn int_field(row: &Value, keys: &[&str]) -> Option<i64> {
    keys.iter().find_map(|key| match row.get(*key) {
        Some(Value::Number(value)) => value.as_i64(),
        Some(Value::String(value)) => value.parse::<i64>().ok(),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        Value, admin_connection_url, columns_sql, connect, create_database, drop_database,
        execute_on_db, list_names, override_database_in_url, ping, postgres_extension_member,
        postgres_table_filter, query_json, quoted_identifier, quoted_relation_identifier,
        table_columns, tables_sql, views_sql, wipe_tables,
    };
    use crate::config::{TideConfig, sqlite_connection_url};
    use tempfile::TempDir;

    fn config(driver: &str) -> TideConfig {
        let mut config = TideConfig::default();
        config.database.driver = driver.to_string();
        config
    }

    /// `information_schema.tables` lists views too, and `DROP TABLE` refuses one.
    #[test]
    fn mysql_lists_views_apart_from_base_tables() {
        assert!(
            tables_sql("mysql")
                .unwrap()
                .contains("table_type IN ('BASE TABLE', 'SYSTEM VERSIONED')")
        );
        assert!(
            views_sql("mysql")
                .unwrap()
                .contains("information_schema.views")
        );
        assert!(views_sql("sqlite").unwrap().contains("type = 'view'"));
        assert!(views_sql("postgres").unwrap().contains("relkind IN ('v')"));
    }

    #[test]
    fn postgres_relation_lookup_and_wipe_preserve_schema_names() {
        let config = config("postgres");
        let tables = tables_sql("postgres").unwrap();
        let columns = columns_sql(&config, "app.users").unwrap();

        assert!(tables.contains("n.nspname || '.' || c.relname"), "{tables}");
        assert!(tables.contains("left(n.nspname, 3) <> 'pg_'"), "{tables}");
        assert!(columns.contains("c.table_schema = 'app' AND c.table_name = 'users'"));
        assert!(columns.contains("tc.table_schema = c.table_schema"));
        assert_eq!(
            postgres_table_filter("c", "users"),
            "c.table_schema = 'public' AND c.table_name = 'users'"
        );
        assert_eq!(
            quoted_relation_identifier(&config, "app.users"),
            "\"app\".\"users\""
        );
        assert_eq!(
            quoted_relation_identifier(&config, "users"),
            "\"public\".\"users\""
        );
        assert_eq!(quoted_identifier(&config, "users"), "\"users\"");
    }

    /// A database of its own on the server the environment variable
    /// `variable` names, for one test: `None`, with a skip line, when it is
    /// not set. The caller drops it with [`drop_database`].
    async fn throwaway_database(variable: &str, driver: &str) -> Option<(TideConfig, String)> {
        let Ok(url) = std::env::var(variable) else {
            println!("skipping: set {variable} to a {driver} server URL to run this test");
            return None;
        };
        let mut server = config(driver);
        server.database.url = Some(url);
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let name = format!("tideorm_cli_{}_{stamp}", std::process::id());
        create_database(&server, &name)
            .await
            .expect("the throwaway database should be created");
        let mut config = server.clone();
        config.database.url = Some(override_database_in_url(&server, &name).unwrap());
        Some((config, name))
    }

    /// An extension's views and tables are its own, and PostgreSQL refuses to
    /// drop them alone: wipe leaves them, and drops the database's own views,
    /// tables, sequences and enum types.
    #[tokio::test]
    async fn postgres_wipe_leaves_what_an_extension_installed() {
        let Some((config, name)) = throwaway_database("TIDEORM_CLI_POSTGRES_URL", "postgres").await
        else {
            return;
        };
        let setup = async {
            let db = connect(&config).await?;
            for sql in [
                "CREATE EXTENSION pg_buffercache",
                "CREATE SCHEMA app",
                "CREATE TABLE users (id SERIAL PRIMARY KEY)",
                "CREATE VIEW user_ids AS SELECT id FROM users",
                "CREATE VIEW constant AS SELECT 1 AS one",
                "CREATE SEQUENCE invoice_numbers",
                "CREATE TYPE mood AS ENUM ('ok')",
                "CREATE TABLE app.posts (id SERIAL PRIMARY KEY)",
                "CREATE VIEW app.post_ids AS SELECT id FROM app.posts",
                "CREATE SEQUENCE app.invoice_numbers",
                "CREATE TYPE app.mood AS ENUM ('ok')",
            ] {
                execute_on_db(&db, sql).await?;
            }
            Ok::<_, String>(())
        };
        let wiped = match setup.await {
            Ok(()) => wipe_tables(&config, true).await,
            Err(error) => Err(error),
        };
        let own = query_json(
            &config,
            &format!(
                "SELECT c.relname AS name FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE n.nspname = 'public' AND NOT {} UNION ALL SELECT typname FROM pg_type WHERE typname = 'mood'",
                postgres_extension_member("pg_class", "c.oid")
            ),
        )
        .await;
        let installed = list_names(
            &config,
            "SELECT viewname AS table_name FROM pg_views WHERE viewname = 'pg_buffercache'",
        )
        .await;
        let custom = query_json(
            &config,
            "SELECT c.relname AS name FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE n.nspname = 'app' AND c.relkind IN ('r', 'p', 'v', 'S') UNION ALL SELECT t.typname AS name FROM pg_type t JOIN pg_namespace n ON n.oid = t.typnamespace WHERE n.nspname = 'app' AND t.typname = 'mood'",
        )
        .await;
        drop_database(&config, &name).await.unwrap();
        wiped.unwrap();
        assert_eq!(own.unwrap(), Vec::<Value>::new());
        assert_eq!(installed.unwrap(), ["pg_buffercache"]);
        assert_eq!(custom.unwrap(), Vec::<Value>::new());
    }

    /// MariaDB lists a system-versioned table and a sequence apart from its
    /// base tables; wipe drops them too, or `migrate fresh` fails creating them.
    #[tokio::test]
    async fn mariadb_wipe_drops_versioned_tables_and_sequences() {
        let Some((config, name)) = throwaway_database("TIDEORM_CLI_MARIADB_URL", "mysql").await
        else {
            return;
        };
        let setup = async {
            let db = connect(&config).await?;
            for sql in [
                "CREATE TABLE plain (id INT PRIMARY KEY)",
                "CREATE TABLE audited (id INT PRIMARY KEY) WITH SYSTEM VERSIONING",
                "CREATE SEQUENCE invoice_numbers",
            ] {
                execute_on_db(&db, sql).await?;
            }
            Ok::<_, String>(())
        };
        let wiped = match setup.await {
            Ok(()) => wipe_tables(&config, false).await,
            Err(error) => Err(error),
        };
        let left = list_names(
            &config,
            "SELECT table_name AS table_name FROM information_schema.tables WHERE table_schema = DATABASE()",
        )
        .await;
        drop_database(&config, &name).await.unwrap();
        wiped.unwrap();
        assert_eq!(left.unwrap(), Vec::<String>::new());
    }

    #[test]
    fn mariadb_is_quoted_as_mysql() {
        assert_eq!(quoted_identifier(&config("mariadb"), "order"), "`order`");
        assert_eq!(quoted_identifier(&config("MySQL"), "order"), "`order`");
    }

    #[test]
    fn admin_urls_encode_credentials_and_bracket_ipv6_hosts() {
        let mut config = config("postgres");
        config.database.host = "::1".to_string();
        config.database.username = Some("app".to_string());
        config.database.password = Some("p?ss@w/rd".to_string());

        assert_eq!(
            admin_connection_url(&config).unwrap(),
            "postgres://app:p%3Fss%40w%2Frd@[::1]:5432/postgres"
        );

        config.database.driver = "mariadb".to_string();
        config.database.port = Some(3307);
        assert_eq!(
            admin_connection_url(&config).unwrap(),
            "mysql://app:p%3Fss%40w%2Frd@[::1]:3307/mysql"
        );
    }

    #[test]
    fn admin_urls_add_a_database_path_when_the_configured_url_omits_one() {
        let mut config = config("postgres");
        config.database.url = Some("postgres://app:p%40ss@db.example?sslmode=require".to_string());

        assert_eq!(
            admin_connection_url(&config).unwrap(),
            "postgres://app:p%40ss@db.example/postgres?sslmode=require"
        );

        config.database.driver = "mysql".to_string();
        config.database.url = Some("mysql://app@db.example/".to_string());
        assert_eq!(
            admin_connection_url(&config).unwrap(),
            "mysql://app@db.example/mysql"
        );
    }

    #[tokio::test]
    async fn ping_does_not_create_a_missing_sqlite_database() {
        let dir = TempDir::new().expect("temp dir should be created");
        let path = dir.path().join("missing.sqlite3");
        let mut config = config("sqlite");
        config.database.sqlite_path = Some(path.to_string_lossy().into_owned());

        assert!(ping(&config).await.is_err());
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn readonly_sqlite_connection_does_not_create_a_missing_database() {
        let dir = TempDir::new().expect("temp dir should be created");
        let path = dir.path().join("missing.sqlite3");
        let url = format!("{}?mode=ro", sqlite_connection_url(&path.to_string_lossy()));
        let mut config = config("sqlite");
        config.database.url = Some(url);

        assert!(connect(&config).await.is_err());
        assert!(!path.exists());
    }

    /// sqlx opens a URL read-only when any `mode` is `ro`, whatever follows it,
    /// so the CLI creates no file for one either.
    #[tokio::test]
    async fn a_read_only_mode_anywhere_does_not_create_a_missing_database() {
        let dir = TempDir::new().expect("temp dir should be created");
        let path = dir.path().join("missing.sqlite3");
        let url = format!(
            "{}?mode=ro&mode=rwc",
            sqlite_connection_url(&path.to_string_lossy())
        );
        let mut config = config("sqlite");
        config.database.url = Some(url);

        let _ = connect(&config).await;
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn table_columns_rejects_a_missing_sqlite_table() {
        let dir = TempDir::new().expect("temp dir should be created");
        let path = dir.path().join("schema.sqlite3");
        let mut config = config("sqlite");
        config.database.sqlite_path = Some(path.to_string_lossy().into_owned());

        let error = table_columns(&config, "missing_table")
            .await
            .expect_err("a missing table should not look like an empty schema");
        assert!(error.contains("Table or view not found: missing_table"));
    }
}
