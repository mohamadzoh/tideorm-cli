use crate::config::TideConfig;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use tideorm::internal::{ConnectionTrait, Statement};
use tideorm::prelude::Database;

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
    Database::connect(&config.database.connection_url())
        .await
        .map_err(|error| error.to_string())
}

pub async fn ping(config: &TideConfig) -> Result<(), String> {
    let db = connect(config).await?;
    db.ping().await.map(|_| ()).map_err(|error| error.to_string())
}

pub async fn execute(config: &TideConfig, sql: &str) -> Result<u64, String> {
    let db = connect(config).await?;
    execute_on_db(&db, sql).await
}

pub async fn query_json(config: &TideConfig, sql: &str) -> Result<Vec<Value>, String> {
    let db = connect(config).await?;
    query_json_on_db(&db, sql).await
}

pub async fn execute_on_db(db: &Database, sql: &str) -> Result<u64, String> {
    db.__internal_connection()
        .execute_unprepared(sql)
        .await
        .map(|result| result.rows_affected())
        .map_err(|error| error.to_string())
}

pub async fn query_json_on_db(db: &Database, sql: &str) -> Result<Vec<Value>, String> {
    let backend = db.__internal_connection().get_database_backend();
    let statement = Statement::from_string(backend, sql.to_string());
    let rows = db
        .__internal_connection()
        .query_all_raw(statement)
        .await
        .map_err(|error| error.to_string())?;

    let mut values = Vec::new();
    for row in rows {
        let mut object = serde_json::Map::new();

        for column_name in row.column_names() {
            let json_value = if let Ok(value) = row.try_get::<Option<bool>>("", &column_name) {
                value.map(Value::from).unwrap_or(Value::Null)
            } else if let Ok(value) = row.try_get::<Option<i64>>("", &column_name) {
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
    let sql = match normalized_driver(config) {
        "sqlite" => {
            "SELECT name AS table_name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name"
                .to_string()
        }
        "postgres" => {
            "SELECT tablename AS table_name FROM pg_tables WHERE schemaname = 'public' ORDER BY tablename"
                .to_string()
        }
        "mysql" => {
            "SELECT table_name FROM information_schema.tables WHERE table_schema = DATABASE() ORDER BY table_name"
                .to_string()
        }
        driver => return Err(format!("Unsupported database driver: {}", driver)),
    };

    let rows = query_json(config, &sql).await?;
    Ok(rows
        .into_iter()
        .filter_map(|row| string_field(&row, &["table_name", "Tables_in_database", "name"]))
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

    if normalized_driver(config) == "sqlite" {
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
    match normalized_driver(config) {
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
    let sql = match normalized_driver(config) {
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
               AND tc.table_schema = 'public' \
               AND tc.table_name = {} \
             ORDER BY kcu.ordinal_position",
            sql_string(table_name)
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
                column: string_field(&row, &["column_name", "from"] )?,
                references_table: string_field(&row, &["references_table", "table"] )?,
                references_column: string_field(&row, &["references_column", "to"] )?,
            })
        })
        .collect())
}

pub async fn wipe_tables(config: &TideConfig, drop_types: bool) -> Result<(), String> {
    let db = connect(config).await?;
    let tables = list_tables(config).await?;

    match normalized_driver(config) {
        "sqlite" => {
            execute_on_db(&db, "PRAGMA foreign_keys = OFF;").await?;
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
            if !tables.is_empty() {
                let joined = tables
                    .iter()
                    .map(|table| quoted_identifier(config, table))
                    .collect::<Vec<_>>()
                    .join(", ");
                execute_on_db(&db, &format!("DROP TABLE IF EXISTS {} CASCADE", joined)).await?;
            }

            if drop_types {
                let types = query_json(
                    config,
                    "SELECT t.typname AS type_name FROM pg_type t JOIN pg_namespace n ON n.oid = t.typnamespace WHERE n.nspname = 'public' AND t.typtype = 'e' ORDER BY t.typname",
                )
                .await?;

                for row in types {
                    if let Some(type_name) = string_field(&row, &["type_name"]) {
                        execute_on_db(
                            &db,
                            &format!("DROP TYPE IF EXISTS {} CASCADE", quoted_identifier(config, &type_name)),
                        )
                        .await?;
                    }
                }
            }
        }
        "mysql" => {
            execute_on_db(&db, "SET FOREIGN_KEY_CHECKS = 0").await?;
            if !tables.is_empty() {
                let joined = tables
                    .iter()
                    .map(|table| quoted_identifier(config, table))
                    .collect::<Vec<_>>()
                    .join(", ");
                execute_on_db(&db, &format!("DROP TABLE IF EXISTS {}", joined)).await?;
            }
            execute_on_db(&db, "SET FOREIGN_KEY_CHECKS = 1").await?;
        }
        driver => return Err(format!("Unsupported database driver: {}", driver)),
    }

    Ok(())
}

pub async fn create_database(config: &TideConfig, database_name: &str) -> Result<(), String> {
    match normalized_driver(config) {
        "sqlite" => {
            let path = config
                .database
                .sqlite_path
                .as_deref()
                .unwrap_or(database_name);
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
                &format!("CREATE DATABASE {}", quoted_identifier(config, database_name)),
            )
            .await
            .map(|_| ())
        }
        driver => Err(format!("Unsupported database driver: {}", driver)),
    }
}

pub async fn drop_database(config: &TideConfig, database_name: &str) -> Result<(), String> {
    match normalized_driver(config) {
        "sqlite" => {
            let path = config
                .database
                .sqlite_path
                .as_deref()
                .unwrap_or(database_name);
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
                &format!("DROP DATABASE IF EXISTS {}", quoted_identifier(config, database_name)),
            )
            .await
            .map(|_| ())
        }
        "mysql" => {
            let admin_db = connect_with_url(&admin_connection_url(config)?).await?;
            execute_on_db(
                &admin_db,
                &format!("DROP DATABASE IF EXISTS {}", quoted_identifier(config, database_name)),
            )
            .await
            .map(|_| ())
        }
        driver => Err(format!("Unsupported database driver: {}", driver)),
    }
}

pub fn formats_result_set(sql: &str) -> bool {
    let normalized = sql.trim_start().to_ascii_uppercase();

    normalized.starts_with("SELECT")
        || normalized.starts_with("WITH")
        || normalized.starts_with("SHOW")
        || normalized.starts_with("DESCRIBE")
        || normalized.starts_with("DESC")
        || normalized.starts_with("EXPLAIN")
        || normalized.starts_with("PRAGMA")
        || normalized.contains(" RETURNING ")
}

async fn connect_with_url(url: &str) -> Result<Database, String> {
    Database::connect(url)
        .await
        .map_err(|error| error.to_string())
}

fn admin_connection_url(config: &TideConfig) -> Result<String, String> {
    match normalized_driver(config) {
        "postgres" => override_database_in_url(config, "postgres"),
        "mysql" => override_database_in_url(config, "mysql"),
        "sqlite" => Ok(config.database.connection_url()),
        driver => Err(format!("Unsupported database driver: {}", driver)),
    }
}

fn override_database_in_url(config: &TideConfig, database_name: &str) -> Result<String, String> {
    if config.database.url.is_some() {
        let url = config.database.connection_url();
        let pattern = regex::Regex::new(r"^(?P<prefix>[a-zA-Z0-9+]+://[^/]+/)(?P<db>[^?]+)(?P<suffix>\?.*)?$")
            .map_err(|error| error.to_string())?;

        if let Some(captures) = pattern.captures(&url) {
            let prefix = captures.name("prefix").map(|value| value.as_str()).unwrap_or("");
            let suffix = captures.name("suffix").map(|value| value.as_str()).unwrap_or("");
            return Ok(format!("{}{}{}", prefix, database_name, suffix));
        }

        return Err("Failed to derive admin database URL from configured connection URL".to_string());
    }

    match normalized_driver(config) {
        "postgres" => {
            let user = config.database.username.as_deref().unwrap_or("postgres");
            let password = config.database.password.as_deref().unwrap_or("");
            let host = &config.database.host;
            let port = config.database.port.unwrap_or(5432);

            if password.is_empty() {
                Ok(format!("postgres://{}@{}:{}/{}", user, host, port, database_name))
            } else {
                Ok(format!("postgres://{}:{}@{}:{}/{}", user, password, host, port, database_name))
            }
        }
        "mysql" => {
            let user = config.database.username.as_deref().unwrap_or("root");
            let password = config.database.password.as_deref().unwrap_or("");
            let host = &config.database.host;
            let port = config.database.port.unwrap_or(3306);

            if password.is_empty() {
                Ok(format!("mysql://{}@{}:{}/{}", user, host, port, database_name))
            } else {
                Ok(format!("mysql://{}:{}@{}:{}/{}", user, password, host, port, database_name))
            }
        }
        driver => Err(format!("Unsupported database driver: {}", driver)),
    }
}

fn columns_sql(config: &TideConfig, table_name: &str) -> Result<String, String> {
    Ok(match normalized_driver(config) {
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
                   AND tc.table_schema = 'public' \
                   AND tc.table_name = c.table_name \
                   AND kcu.column_name = c.column_name \
               ) THEN 'PRI' \
               WHEN EXISTS ( \
                 SELECT 1 FROM information_schema.table_constraints tc \
                 JOIN information_schema.key_column_usage kcu \
                   ON tc.constraint_name = kcu.constraint_name \
                  AND tc.table_schema = kcu.table_schema \
                 WHERE tc.constraint_type = 'UNIQUE' \
                   AND tc.table_schema = 'public' \
                   AND tc.table_name = c.table_name \
                   AND kcu.column_name = c.column_name \
               ) THEN 'UNI' \
               WHEN EXISTS ( \
                 SELECT 1 FROM information_schema.table_constraints tc \
                 JOIN information_schema.key_column_usage kcu \
                   ON tc.constraint_name = kcu.constraint_name \
                  AND tc.table_schema = kcu.table_schema \
                 WHERE tc.constraint_type = 'FOREIGN KEY' \
                   AND tc.table_schema = 'public' \
                   AND tc.table_name = c.table_name \
                   AND kcu.column_name = c.column_name \
               ) THEN 'FK' \
               ELSE NULL \
             END AS key_name, \
             c.column_default AS default_value, \
             NULL::TEXT AS extra \
             FROM information_schema.columns c \
             WHERE c.table_schema = 'public' AND c.table_name = {} \
             ORDER BY c.ordinal_position",
            sql_string(table_name)
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

async fn sqlite_indexes(config: &TideConfig, table_name: &str) -> Result<Vec<IndexDetails>, String> {
    let rows = query_json(
        config,
        &format!("PRAGMA index_list({})", quoted_identifier(config, table_name)),
    )
    .await?;

    let mut indexes = Vec::new();
    for row in rows {
        let Some(index_name) = string_field(&row, &["name"]) else {
            continue;
        };
        let columns = query_json(
            config,
            &format!("PRAGMA index_info({})", quoted_identifier(config, &index_name)),
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

async fn postgres_indexes(config: &TideConfig, table_name: &str) -> Result<Vec<IndexDetails>, String> {
    let rows = query_json(
        config,
        &format!(
            "SELECT indexname, indexdef FROM pg_indexes WHERE schemaname = 'public' AND tablename = {} ORDER BY indexname",
            sql_string(table_name)
        ),
    )
    .await?;

    let capture = regex::Regex::new(r"\((?P<columns>[^\)]*)\)").map_err(|error| error.to_string())?;

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

        let entry = grouped.entry(index_name.clone()).or_insert_with(|| IndexDetails {
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
    if normalized_driver(config) == "sqlite" {
        let is_primary = int_field(row, &["pk"]).unwrap_or(0) > 0;
        let nullable = int_field(row, &["notnull"]).unwrap_or(0) == 0;

        return ColumnDetails {
            name: string_field(row, &["name", "column_name"]).unwrap_or_default(),
            data_type: string_field(row, &["type", "data_type"]).unwrap_or_default(),
            nullable,
            key: if is_primary { Some("PRI".to_string()) } else { None },
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

fn normalized_driver(config: &TideConfig) -> &str {
    match config.database.driver.as_str() {
        "postgresql" => "postgres",
        driver => driver,
    }
}

fn quoted_identifier(config: &TideConfig, identifier: &str) -> String {
    match normalized_driver(config) {
        "mysql" => format!("`{}`", identifier.replace('`', "``")),
        _ => format!("\"{}\"", identifier.replace('"', "\"\"")),
    }
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