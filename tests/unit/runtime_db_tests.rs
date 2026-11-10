use super::{
    admin_connection_url, connect, create_database, drop_database, execute_on_db,
    override_database_in_url, ping, quoted_identifier, quoted_relation_identifier, table_columns,
    wipe_tables,
};
use crate::config::{TideConfig, sqlite_connection_url};
use serde_json::Value;
use tempfile::TempDir;

/// Each row of `sql` as a JSON object, its values read by the column's
/// declared type.
async fn query_json(config: &TideConfig, sql: &str) -> Result<Vec<Value>, String> {
    connect(config)
        .await?
        .query_raw_json(sql)
        .await
        .map_err(|error| error.to_string())
}

fn config(driver: &str) -> TideConfig {
    let mut config = TideConfig::default();
    config.database.driver = driver.to_string();
    config
}

/// The catalog names a relation outside the current schema `schema.name`.
#[test]
fn postgres_wipe_quotes_the_schema_of_a_qualified_name() {
    let config = config("postgres");
    assert_eq!(
        quoted_relation_identifier(&config, "app.users"),
        "\"app\".\"users\""
    );
    assert_eq!(quoted_relation_identifier(&config, "users"), "\"users\"");
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
        "SELECT c.relname AS name FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE n.nspname = 'public' AND NOT EXISTS (SELECT 1 FROM pg_depend d WHERE d.classid = 'pg_class'::regclass AND d.objid = c.oid AND d.deptype = 'e') UNION ALL SELECT typname FROM pg_type WHERE typname = 'mood'",
    )
    .await;
    let installed = query_json(
        &config,
        "SELECT viewname::text AS name FROM pg_views WHERE viewname = 'pg_buffercache'",
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
    assert_eq!(
        installed.unwrap(),
        [serde_json::json!({ "name": "pg_buffercache" })]
    );
    assert_eq!(custom.unwrap(), Vec::<Value>::new());
}

/// MariaDB lists a system-versioned table and a sequence apart from its
/// base tables; wipe drops them too, or `migrate fresh` fails creating them.
#[tokio::test]
async fn mariadb_wipe_drops_versioned_tables_and_sequences() {
    let Some((config, name)) = throwaway_database("TIDEORM_CLI_MARIADB_URL", "mysql").await else {
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
    let left = query_json(
        &config,
        "SELECT table_name AS name FROM information_schema.tables WHERE table_schema = DATABASE()",
    )
    .await;
    drop_database(&config, &name).await.unwrap();
    wiped.unwrap();
    assert_eq!(left.unwrap(), Vec::<Value>::new());
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

#[tokio::test]
async fn table_columns_rejects_a_missing_sqlite_table() {
    let dir = TempDir::new().expect("temp dir should be created");
    let path = dir.path().join("schema.sqlite3");
    let mut config = config("sqlite");
    config.database.sqlite_path = Some(path.to_string_lossy().into_owned());
    create_database(&config, &path.to_string_lossy())
        .await
        .unwrap();

    let db = connect(&config).await.unwrap();
    let error = table_columns(&db, "missing_table")
        .await
        .expect_err("a missing table should not look like an empty schema");
    assert!(error.contains("Table or view not found: missing_table"));
}

/// Inspecting or wiping a mistyped path is an error, and leaves no file
/// or directory behind.
#[tokio::test]
async fn inspection_and_wipe_refuse_a_missing_sqlite_file() {
    let dir = TempDir::new().expect("temp dir should be created");
    let path = dir.path().join("typo").join("app.sqlite3");
    let mut config = config("sqlite");
    config.database.sqlite_path = Some(path.to_string_lossy().into_owned());

    assert!(connect(&config).await.is_err());
    assert!(wipe_tables(&config, false).await.is_err());
    assert!(!dir.path().join("typo").exists());
}
