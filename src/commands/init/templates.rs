//! The files `tideorm init` scaffolds.

use super::SCAFFOLD_TIDEORM_VERSION;
use super::options::{DatabaseInit, InitOptions};

/// `value` as a TOML string, quoted and escaped: a Windows path's backslashes, or a quote
/// in an answer, would otherwise leave `tideorm.toml` unparsable.
fn toml_string(value: &str) -> String {
    toml::Value::String(value.to_string()).to_string()
}

pub(super) fn generate_tideorm_toml(options: &InitOptions) -> String {
    let env_file = toml_string(&options.env_file_name);
    match &options.database {
        DatabaseInit::Sqlite { path } => format!(
            r#"# TideORM Configuration File
# This file configures the TideORM CLI and runtime behavior.

[project]
name = "my-tideorm-project"
environment = "${{TIDEORM_ENV}}"
env_file = {env_file}

[database]
driver = "sqlite"
sqlite_path = {path}
url = "${{DATABASE_URL}}"

[paths]
models = "src/models"
migrations = "src/migrations"
seeders = "src/seeders"
factories = "src/factories"

[migration]
table = "_migrations"
timestamps = true

[model]
timestamps = true
soft_deletes = false
tokenize = false
primary_key = "id"
primary_key_type = "i64"
"#,
            env_file = env_file,
            path = toml_string(path)
        ),
        DatabaseInit::Postgres {
            host,
            port,
            database,
            username,
            ..
        } => format!(
            r#"# TideORM Configuration File
# This file configures the TideORM CLI and runtime behavior.

[project]
name = "my-tideorm-project"
environment = "${{TIDEORM_ENV}}"
env_file = {env_file}

[database]
driver = "postgres"
host = {host}
port = {port}
database = {database}
username = {username}
password = ""
url = "${{DATABASE_URL}}"

[paths]
models = "src/models"
migrations = "src/migrations"
seeders = "src/seeders"
factories = "src/factories"

[migration]
table = "_migrations"
timestamps = true

[model]
timestamps = true
soft_deletes = false
tokenize = false
primary_key = "id"
primary_key_type = "i64"
"#,
            env_file = env_file,
            host = toml_string(host),
            port = port,
            database = toml_string(database),
            username = toml_string(username)
        ),
        DatabaseInit::MySql {
            host,
            port,
            database,
            username,
            ..
        } => format!(
            r#"# TideORM Configuration File
# This file configures the TideORM CLI and runtime behavior.

[project]
name = "my-tideorm-project"
environment = "${{TIDEORM_ENV}}"
env_file = {env_file}

[database]
driver = "mysql"
host = {host}
port = {port}
database = {database}
username = {username}
password = ""
url = "${{DATABASE_URL}}"

[paths]
models = "src/models"
migrations = "src/migrations"
seeders = "src/seeders"
factories = "src/factories"

[migration]
table = "_migrations"
timestamps = true

[model]
timestamps = true
soft_deletes = false
tokenize = false
primary_key = "id"
primary_key_type = "i64"
"#,
            env_file = env_file,
            host = toml_string(host),
            port = port,
            database = toml_string(database),
            username = toml_string(username)
        ),
    }
}

/// Contents of the scaffolded `.gitignore`.
///
/// `tideorm init` writes the database password into the env file in plaintext, so that
/// file - along with the build directory and any local SQLite database - must never be
/// committed by accident.
pub(super) fn generate_gitignore(env_file_name: &str) -> String {
    let mut content = r#"# Build output
/target

# Environment files - these hold database credentials in plaintext
.env
.env.*
!.env.example

# Local databases
*.db
*.db-shm
*.db-wal
*.sqlite
*.sqlite3
"#
    .to_string();

    // `tideorm init` lets the env file be renamed; anything not starting with `.env` is
    // not covered by the patterns above.
    if !env_file_name.starts_with(".env") {
        let extra = format!("\n# Configured environment file\n{}\n", env_file_name);
        content.push_str(&extra);
    }

    content
}

/// The scaffold's `src/config.rs`, which loads `env_file` - the file `tideorm init` writes
/// `DATABASE_URL` into - so the app reaches the database the CLI does. The runners connect
/// with its `tideorm_config()` through `connect_only()`, so the app's startup work, a
/// `force_sync` above all, never runs before a migration or seed.
pub(super) fn generate_config_rs(database: &str, env_file: &str) -> String {
    let db_setup = match database {
        "sqlite" => {
            r#"
    // SQLite setup
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "sqlite://database.db".to_string());

    TideConfig::init()
        .database_type(DatabaseType::SQLite)
        .database(&database_url)
        .max_connections(5)
        .min_connections(1)
"#
        }
        "mysql" => {
            r#"
    // MySQL setup
    let database_url = std::env::var("DATABASE_URL")
        .expect("DATABASE_URL must be set");

    TideConfig::init()
        .database_type(DatabaseType::MySQL)
        .database(&database_url)
        .max_connections(10)
        .min_connections(2)
"#
        }
        _ => {
            r#"
    // PostgreSQL setup
    let database_url = std::env::var("DATABASE_URL")
        .expect("DATABASE_URL must be set");

    TideConfig::init()
        .database_type(DatabaseType::Postgres)
        .database(&database_url)
        .max_connections(10)
        .min_connections(2)
"#
        }
    };

    format!(
        r#"//! TideORM Configuration
//!
//! This file configures the TideORM database connection and runtime settings.
//! The application connects with `connect_tideorm()`, which also runs the
//! startup work the configuration enables (schema sync, migrations, seeds, a
//! schema file). The runners `tideorm migrate` and `tideorm db seed` build in
//! `src/bin` connect with `tideorm_config().connect_only()` instead, which
//! skips that work.

use tideorm::prelude::*;

/// Build the TideORM configuration
pub fn tideorm_config() -> TideConfig {{
    // Variables already set, such as the `DATABASE_URL` that `tideorm db seed`
    // and `tideorm migrate` pass, win over the file.
    let _ = dotenvy::from_filename({env_file:?});
{db_setup}
}}

pub async fn connect_tideorm() -> tideorm::Result<&'static Database> {{
    tideorm_config().connect().await
}}

#[cfg(test)]
mod tests {{
    use super::*;

    #[test]
    fn test_config_creation() {{
        unsafe {{
            std::env::set_var("DATABASE_URL", "sqlite://test.db");
        }}
        let _config = tideorm_config();
    }}
}}
"#,
        db_setup = db_setup,
        env_file = env_file,
    )
}

pub(super) fn generate_cargo_toml(package_name: &str, database: &str) -> String {
    let database_feature = match database {
        "sqlite" => "sqlite",
        "mysql" => "mysql",
        _ => "postgres",
    };

    format!(
        r#"[package]
name = "{package_name}"
version = "0.1.0"
edition = "2024"
default-run = "{package_name}"

[dependencies]
tokio = {{ version = "1", features = ["full"] }}
serde = {{ version = "1", features = ["derive"] }}
serde_json = "1"
chrono = "0.4"
dotenvy = "0.15"
tideorm = {{ version = "{tideorm_version}", default-features = false, features = ["{database_feature}", "runtime-tokio"] }}

[features]
entity-manager = ["tideorm/entity-manager"]
"#,
        package_name = package_name,
        tideorm_version = SCAFFOLD_TIDEORM_VERSION,
        database_feature = database_feature,
    )
}

pub(super) const LIB_RS: &str = "pub mod config;
pub mod factories;
pub mod migrations;
pub mod models;
pub mod seeders;
";

pub(super) fn generate_main_rs(library: &str) -> String {
    format!(
        r#"#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {{
    {library}::config::connect_tideorm().await?;
    println!("TideORM project initialized.");
    Ok(())
}}
"#
    )
}

pub(super) fn generate_database_seeder() -> String {
    r#"//! Database Seeder
//!
//! The first seeder `tideorm db seed` runs. Add seed data here, or generate
//! more seeders with `tideorm make seeder`; each is registered in
//! `seeders::all()`.

use tideorm::prelude::*;

#[derive(Default)]
pub struct DatabaseSeeder;

#[async_trait]
impl Seed for DatabaseSeeder {
    fn name(&self) -> &str {
        "database_seeder"
    }

    async fn run(&self, _db: &Database) -> tideorm::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_seeder_runs() {
        let seeder = DatabaseSeeder::default();
        assert_eq!(seeder.name(), "database_seeder");
    }
}
"#
    .to_string()
}
