use crate::config::TideConfig;
use std::fs;
use std::path::Path;
use tideorm::prelude::{Database, DatabaseType};
use tideorm::schema::{ColumnInfo, ForeignKeyInfo, IndexInfo};

/// The ledger TideORM's `Seeder` records runs in.
pub const DEFAULT_SEEDERS_TABLE: &str = "_seeds";

/// Connect to the configured database. A SQLite file must exist: only
/// `db create` and `init` create one, so a mistyped path is an error rather
/// than an empty database to inspect or wipe.
pub async fn connect(config: &TideConfig) -> Result<Database, String> {
    require_sqlite_file(config)?;
    connect_with_url(&config.database.connection_url()).await
}

/// Connect on a pinned session, so every statement shares one connection:
/// `SET FOREIGN_KEY_CHECKS` and `PRAGMA foreign_keys` hold only on the
/// connection they ran on, and a pool may run the next statement on another,
/// or reconnect.
pub(crate) async fn connect_single_session(config: &TideConfig) -> Result<Database, String> {
    let url = config.database.connection_url();
    connect(config)
        .await?
        .pin_session()
        .await
        .map_err(|error| crate::utils::masked_connection_error(error, &url))
}

/// Refuse a configured SQLite file that does not exist; an in-memory
/// database has none.
fn require_sqlite_file(config: &TideConfig) -> Result<(), String> {
    if config.database.driver_name() == "sqlite"
        && let Some(path) = config.database.sqlite_file()
        && !Path::new(&path).exists()
    {
        return Err(format!(
            "SQLite database file does not exist: {path} (create it with `tideorm db create`)"
        ));
    }
    Ok(())
}

pub async fn ping(config: &TideConfig) -> Result<(), String> {
    require_sqlite_file(config)?;

    let db = connect_with_url(&config.database.try_connection_url()?).await?;
    db.ping()
        .await
        .map(|_| ())
        .map_err(|error| error.to_string())
}

/// Run `sql` unprepared, so it may hold several `;`-separated statements.
pub async fn execute_on_db(db: &Database, sql: &str) -> Result<u64, String> {
    db.exec_raw(sql).await.map_err(|error| error.to_string())
}

/// The base tables of the current database: MariaDB's system-versioned
/// ones too, and on PostgreSQL those of every user schema, `schema.table`
/// outside the current one.
pub async fn list_tables(db: &Database) -> Result<Vec<String>, String> {
    db.catalog()
        .tables()
        .await
        .map_err(|error| error.to_string())
}

/// The columns of a table or view; an error when there is none.
pub async fn table_columns(db: &Database, table_name: &str) -> Result<Vec<ColumnInfo>, String> {
    let columns = db
        .catalog()
        .columns(table_name)
        .await
        .map_err(|error| error.to_string())?;
    if columns.is_empty() {
        return Err(format!("Table or view not found: {table_name}"));
    }
    Ok(columns)
}

pub async fn table_indexes(db: &Database, table_name: &str) -> Result<Vec<IndexInfo>, String> {
    db.catalog()
        .indexes(table_name)
        .await
        .map_err(|error| error.to_string())
}

pub async fn table_foreign_keys(
    db: &Database,
    table_name: &str,
) -> Result<Vec<ForeignKeyInfo>, String> {
    db.catalog()
        .foreign_keys(table_name)
        .await
        .map_err(|error| error.to_string())
}

/// The key a column's listing shows: `PRI`, `UNI` or `FK`, or none.
pub fn column_key(column: &ColumnInfo) -> &'static str {
    if column.primary_key {
        "PRI"
    } else if column.unique {
        "UNI"
    } else if column.foreign_key {
        "FK"
    } else {
        ""
    }
}

/// Drop every view, table and sequence, and with `drop_types` PostgreSQL's
/// enum types. Views go with `DROP VIEW`: MySQL refuses `DROP TABLE` on one,
/// and SQLite and MySQL keep a view whose tables are dropped. What a
/// PostgreSQL extension installed stays; PostgreSQL drops it only with the
/// extension.
pub async fn wipe_tables(config: &TideConfig, drop_types: bool) -> Result<(), String> {
    let db = connect_single_session(config).await?;
    let catalog = db.catalog();
    let listed = async {
        Ok::<_, tideorm::Error>((
            catalog.views().await?,
            catalog.tables().await?,
            catalog.sequences().await?,
        ))
    };
    let (views, tables, sequences) = listed.await.map_err(|error| error.to_string())?;
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
                let types = catalog
                    .enum_types()
                    .await
                    .map_err(|error| error.to_string())?;
                for type_name in types {
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

/// Forget every seed the ledger records, so the next `db seed` runs them all:
/// no migration owns the ledger, so it outlives a reset of the tables it
/// describes. A database no seeder ran on has none.
pub async fn clear_seed_ledger(config: &TideConfig) -> Result<(), String> {
    let db = connect(config).await?;
    let tables = db
        .catalog()
        .tables()
        .await
        .map_err(|error| error.to_string())?;
    if !tables.iter().any(|table| table == DEFAULT_SEEDERS_TABLE) {
        return Ok(());
    }
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
                DatabaseType::Postgres
                    .dialect()
                    .string_literal(database_name)
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
        let mut url = url::Url::parse(&config.database.connection_url())
            .ok()
            .filter(url::Url::has_host)
            .ok_or("Failed to derive admin database URL from configured connection URL")?;
        url.set_path(&format!("/{database_name}"));
        return Ok(url.into());
    }

    config.database.server_url_for_database(database_name)
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
    let mut quoted = String::new();
    crate::utils::database_type(&config.database.driver)
        .dialect()
        .push_identifier(&mut quoted, identifier);
    quoted
}

/// A relation the catalog listed: on PostgreSQL `schema.name` outside the
/// current schema, the bare name in it.
fn quoted_relation_identifier(config: &TideConfig, identifier: &str) -> String {
    if config.database.driver_name() == "postgres"
        && let (Some(schema), name) = crate::utils::split_table(identifier)
    {
        return format!(
            "{}.{}",
            quoted_identifier(config, schema),
            quoted_identifier(config, name)
        );
    }
    quoted_identifier(config, identifier)
}

#[cfg(test)]
#[path = "../tests/unit/runtime_db_tests.rs"]
mod tests;
