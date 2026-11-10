use super::{drop_types_supported, pick_seed, resolve_target_database};
use crate::config::TideConfig;
use crate::runtime_db;
use std::fs;
use tempfile::TempDir;

#[test]
fn a_seeder_is_found_by_its_own_name_or_its_model_name() {
    let seeds = ["database_seeder", "user_seeder", "demo"];
    assert_eq!(pick_seed(&seeds, "UserSeeder").unwrap(), "user_seeder");
    assert_eq!(pick_seed(&seeds, "user_seeder").unwrap(), "user_seeder");
    assert_eq!(pick_seed(&seeds, "User").unwrap(), "user_seeder");
    // A seed named exactly so wins over one with `_seeder` added.
    assert_eq!(pick_seed(&seeds, "Demo").unwrap(), "demo");

    let unknown = pick_seed(&seeds, "Nope").unwrap_err();
    assert!(unknown.contains("No seeder named `Nope`"), "{unknown}");
    assert!(
        unknown.contains("database_seeder, user_seeder, demo"),
        "{unknown}"
    );
    assert!(
        pick_seed(&[], "User")
            .unwrap_err()
            .contains("registers none")
    );
}

#[test]
fn an_explicit_empty_database_target_does_not_fall_back_to_config() {
    let mut config = TideConfig::default();
    config.database.database = Some("production".to_string());

    let error = resolve_target_database(&config, Some("  "))
        .expect_err("an explicitly empty target must not select the configured database");
    assert!(error.contains("must not be empty"));
}

#[test]
fn drop_types_is_reported_as_postgres_only() {
    let fixture = TempDbProject::new();
    let mut config = TideConfig::load(fixture.config_path()).expect("config should load");

    config.database.driver = "sqlite".to_string();
    assert!(!drop_types_supported(&config));

    config.database.driver = "mysql".to_string();
    assert!(!drop_types_supported(&config));

    config.database.driver = "postgres".to_string();
    assert!(drop_types_supported(&config));

    config.database.driver = "postgresql".to_string();
    assert!(drop_types_supported(&config));
}

#[tokio::test]
async fn status_returns_failure_when_sqlite_database_is_missing() {
    let dir = TempDir::new().expect("temp dir should be created");
    let database_path = dir.path().join("missing.sqlite3");
    let config_path = dir.path().join("tideorm.toml");
    let database_url = crate::config::sqlite_connection_url(
        database_path
            .to_str()
            .expect("database path should be UTF-8"),
    );
    fs::write(
        &config_path,
        format!(
            "[project]\nenvironment = \"development\"\n\n[database]\ndriver = \"sqlite\"\nurl = {:?}\n",
            database_url
        ),
    )
    .expect("config should be written");

    assert!(
        super::status(&slash_path(&config_path), false)
            .await
            .is_err()
    );
    assert!(!database_path.exists());
}

/// `a_parents` is dropped first, while `b_children` still points at it, so
/// the drops need the foreign-key switch of their own session.
#[tokio::test]
async fn wipe_drops_a_table_other_rows_still_reference() {
    let fixture = TempDbProject::new();
    let config = TideConfig::load(fixture.config_path()).expect("config should load");
    let db = fixture.connect(&config).await;
    for sql in [
        "CREATE TABLE a_parents (id INTEGER PRIMARY KEY)",
        "CREATE TABLE b_children (id INTEGER PRIMARY KEY, parent_id INTEGER NOT NULL REFERENCES a_parents(id))",
        "INSERT INTO a_parents (id) VALUES (1)",
        "INSERT INTO b_children (id, parent_id) VALUES (1, 1)",
    ] {
        runtime_db::execute_on_db(&db, sql).await.expect(sql);
    }

    runtime_db::wipe_tables(&config, false)
        .await
        .expect("wipe should drop every table");

    let tables = runtime_db::list_tables(&db)
        .await
        .expect("tables should be listed");
    assert!(tables.is_empty(), "{tables:?}");
}

/// A view left behind made `migrate fresh` fail on its `CREATE VIEW` after
/// every table was already gone.
#[tokio::test]
async fn wipe_drops_views_too() {
    let fixture = TempDbProject::new();
    let config = TideConfig::load(fixture.config_path()).expect("config should load");
    let db = fixture.connect(&config).await;
    for sql in [
        "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)",
        "CREATE VIEW user_names AS SELECT name FROM users",
    ] {
        runtime_db::execute_on_db(&db, sql).await.expect(sql);
    }

    runtime_db::wipe_tables(&config, false)
        .await
        .expect("wipe should drop every table and view");

    let left = db
        .query_raw_json("SELECT name FROM sqlite_master")
        .await
        .expect("schema should be readable");
    assert!(left.is_empty(), "{left:?}");
}

/// `db drop` deletes the file the connection URL opens, not `sqlite_path`.
#[tokio::test]
async fn drop_deletes_the_file_the_connection_url_opens() {
    let dir = TempDir::new().expect("temp dir should be created");
    let root = dir.path();
    let real = root.join("real.sqlite3");
    let decoy = root.join("decoy.sqlite3");
    fs::write(&real, b"").expect("real database should be created");
    fs::write(&decoy, b"").expect("decoy should be created");

    let config_path = root.join("tideorm.toml");
    fs::write(
        &config_path,
        format!(
            "[project]\nenvironment = \"development\"\n\n[database]\ndriver = \"sqlite\"\nsqlite_path = \"{}\"\nurl = \"${{TIDEORM_TEST_DROP_URL}}\"\n",
            slash_path(&decoy)
        ),
    )
    .expect("config should be written");
    fs::write(
        root.join(".env"),
        format!(
            "TIDEORM_TEST_DROP_URL=sqlite:{}?mode=rwc\n",
            slash_path(&real)
        ),
    )
    .expect("env file should be written");

    super::drop_database(&slash_path(&config_path), None, true, false)
        .await
        .expect("drop should succeed");

    assert!(!real.exists(), "the database the URL names must be dropped");
    assert!(decoy.exists(), "sqlite_path is not the database in use");
}

struct TempDbProject {
    _dir: TempDir,
    config_path: String,
}

impl TempDbProject {
    fn new() -> Self {
        let dir = TempDir::new().expect("temp dir should be created");
        let root = dir.path();
        let db_path = slash_path(root.join("check.sqlite3"));
        let config_path = root.join("tideorm.toml");

        let config_contents = format!(
            "[project]\nname = \"test-project\"\nenvironment = \"development\"\n\n[database]\ndriver = \"sqlite\"\nsqlite_path = \"{}\"\n\n[paths]\nmigrations = \"src/migrations\"\nmodels = \"src/models\"\nseeders = \"src/seeders\"\nfactories = \"src/factories\"\n\n[migration]\ntable = \"_migrations\"\ntimestamps = true\n\n[model]\ntimestamps = true\nsoft_deletes = false\ntokenize = false\nprimary_key = \"id\"\nprimary_key_type = \"i64\"\n",
            db_path,
        );

        fs::write(&config_path, config_contents).expect("config should be written");

        Self {
            _dir: dir,
            config_path: slash_path(config_path),
        }
    }

    fn config_path(&self) -> &str {
        &self.config_path
    }

    /// Create the project's database file and connect to it.
    async fn connect(&self, config: &TideConfig) -> tideorm::prelude::Database {
        let path = config.database.sqlite_file().expect("a file database");
        runtime_db::create_database(config, &path)
            .await
            .expect("create");
        runtime_db::connect(config).await.expect("connect")
    }
}

fn slash_path(path: impl AsRef<std::path::Path>) -> String {
    path.as_ref().to_string_lossy().replace('\\', "/")
}
