//! Init command for TideORM CLI

use crate::config::TideConfig;
use crate::runtime_db;
use crate::utils::{confirm, ensure_directory, file_exists, print_info, print_success, print_warning};
use colored::Colorize;
use dialoguer::{Input, Password};
use std::collections::VecDeque;
use std::io::IsTerminal;
use std::sync::{LazyLock, Mutex};

static PROMPT_SCRIPT: LazyLock<Mutex<Option<VecDeque<String>>>> =
    LazyLock::new(|| Mutex::new(None));

/// Initialize a new TideORM project
pub async fn run(name: &str, database: &str, verbose: bool) -> Result<(), String> {
    let project_path = if name == "." {
        std::env::current_dir()
            .map_err(|error| format!("Failed to get current directory: {}", error))?
    } else {
        std::path::PathBuf::from(name)
    };

    if verbose {
        print_info(&format!(
            "Initializing TideORM project in: {}",
            project_path.display()
        ));
    }

    println!("\n{}", "Initializing TideORM project...".cyan().bold());
    println!("{}", "─".repeat(50));

    if !project_path.exists() {
        std::fs::create_dir_all(&project_path)
            .map_err(|error| format!("Failed to create project directory: {}", error))?;
        print_success(&format!("Created directory: {}", project_path.display()));
    }

    let _cwd_guard = WorkingDirectoryGuard::change_to(&project_path)?;

    let init_options = collect_init_options(database, verbose)?;
    let should_write_project_config = !file_exists("tideorm.toml") || init_options.overwrite_config;

    if should_write_project_config {
        write_env_file(&init_options)?;
        write_tideorm_config(&init_options)?;
    } else if verbose {
        print_warning("Keeping existing tideorm.toml and skipping env/database initialization");
    }

    create_project_structure(verbose)?;
    create_scaffold_files(&project_path, init_options.database.driver())?;

    if should_write_project_config && init_options.create_database_now {
        create_database_from_config("tideorm.toml", &init_options, verbose).await?;
    }

    if should_write_project_config && init_options.run_migrations_now {
        crate::commands::migrate::run("tideorm.toml", None, false, true, None).await?;
    }

    println!("{}", "─".repeat(50));
    println!("\n{}", "✓ TideORM project initialized successfully!".green().bold());

    print_next_steps(&init_options, should_write_project_config);

    Ok(())
}

struct WorkingDirectoryGuard {
    original_dir: std::path::PathBuf,
}

impl WorkingDirectoryGuard {
    fn change_to(path: &std::path::Path) -> Result<Self, String> {
        let original_dir = std::env::current_dir()
            .map_err(|error| format!("Failed to get current directory: {}", error))?;
        std::env::set_current_dir(path)
            .map_err(|error| format!("Failed to change directory: {}", error))?;
        Ok(Self { original_dir })
    }
}

impl Drop for WorkingDirectoryGuard {
    fn drop(&mut self) {
        let _ = std::env::set_current_dir(&self.original_dir);
    }
}

#[derive(Debug, Clone)]
struct InitOptions {
    env_file_name: String,
    overwrite_config: bool,
    database: DatabaseInit,
    create_database_now: bool,
    run_migrations_now: bool,
}

#[derive(Debug, Clone)]
enum DatabaseInit {
    Sqlite {
        path: String,
    },
    Postgres {
        host: String,
        port: u16,
        database: String,
        username: String,
        password: String,
    },
    MySql {
        host: String,
        port: u16,
        database: String,
        username: String,
        password: String,
    },
}

impl DatabaseInit {
    fn driver(&self) -> &'static str {
        match self {
            Self::Sqlite { .. } => "sqlite",
            Self::Postgres { .. } => "postgres",
            Self::MySql { .. } => "mysql",
        }
    }

    fn database_name(&self) -> Option<&str> {
        match self {
            Self::Sqlite { .. } => None,
            Self::Postgres { database, .. } | Self::MySql { database, .. } => Some(database),
        }
    }

    fn display_name(&self) -> &'static str {
        match self {
            Self::Sqlite { .. } => "SQLite",
            Self::Postgres { .. } => "Postgres",
            Self::MySql { .. } => "MySQL",
        }
    }

    fn connection_url(&self) -> String {
        match self {
            Self::Sqlite { path } => sqlite_url(path),
            Self::Postgres {
                host,
                port,
                database,
                username,
                password,
            } => build_url("postgres", host, *port, database, username, password),
            Self::MySql {
                host,
                port,
                database,
                username,
                password,
            } => build_url("mysql", host, *port, database, username, password),
        }
    }
}

fn collect_init_options(database: &str, verbose: bool) -> Result<InitOptions, String> {
    let running_under_rust_tests = cfg!(test) || std::env::var_os("RUST_TEST_THREADS").is_some();
    let forced_noninteractive = std::env::var_os("TIDEORM_NONINTERACTIVE").is_some()
        || std::env::var_os("CI").is_some();
    let interactive = has_prompt_script()
        || (!running_under_rust_tests
            && !forced_noninteractive
            && std::io::stdin().is_terminal()
            && std::io::stdout().is_terminal());

    if interactive {
        print_setup_intro(database);
    }

    let env_file_name = if interactive {
        prompt_text("Environment file", ".env")?
    } else {
        ".env".to_string()
    };

    let overwrite_config = if file_exists("tideorm.toml") {
        if interactive {
            prompt_confirm("Found tideorm.toml. Replace it with the new setup answers?")?
        } else {
            false
        }
    } else {
        true
    };

    let database = match normalize_database(database) {
        "sqlite" => collect_sqlite_options(interactive)?,
        "mysql" => collect_mysql_options(interactive)?,
        _ => collect_postgres_options(interactive)?,
    };

    let create_database_now = match &database {
        DatabaseInit::Sqlite { .. } => true,
        _ if interactive => {
            let use_existing_database = prompt_confirm(&format!(
                "Use an existing {} database?",
                database.display_name()
            ))?;
            if use_existing_database {
                false
            } else {
                prompt_confirm("Create the database during setup?")?
            }
        }
        _ => false,
    };

    let run_migrations_now = if interactive {
        prompt_confirm("Run pending migrations during setup?")?
    } else {
        false
    };

    if verbose {
        print_info(&format!(
            "Using environment file {} with database driver {}",
            env_file_name,
            database.driver()
        ));
    }

    Ok(InitOptions {
        env_file_name,
        overwrite_config,
        database,
        create_database_now,
        run_migrations_now,
    })
}

fn collect_sqlite_options(interactive: bool) -> Result<DatabaseInit, String> {
    let path = if interactive {
        prompt_text("SQLite file path", "database.db")?
    } else {
        "database.db".to_string()
    };

    Ok(DatabaseInit::Sqlite { path })
}

fn collect_postgres_options(interactive: bool) -> Result<DatabaseInit, String> {
    let host = if interactive {
        prompt_text("Postgres host", "localhost")?
    } else {
        "localhost".to_string()
    };
    let port = if interactive {
        prompt_u16("Postgres port", 5432)?
    } else {
        5432
    };
    let database = if interactive {
        prompt_text("Postgres database", "tideorm_db")?
    } else {
        "tideorm_db".to_string()
    };
    let username = if interactive {
        prompt_text("Postgres user", "postgres")?
    } else {
        "postgres".to_string()
    };
    let password = if interactive {
        prompt_password("Postgres password (leave blank to skip)")?
    } else {
        String::new()
    };

    Ok(DatabaseInit::Postgres {
        host,
        port,
        database,
        username,
        password,
    })
}

fn collect_mysql_options(interactive: bool) -> Result<DatabaseInit, String> {
    let host = if interactive {
        prompt_text("MySQL host", "localhost")?
    } else {
        "localhost".to_string()
    };
    let port = if interactive {
        prompt_u16("MySQL port", 3306)?
    } else {
        3306
    };
    let database = if interactive {
        prompt_text("MySQL database", "tideorm_db")?
    } else {
        "tideorm_db".to_string()
    };
    let username = if interactive {
        prompt_text("MySQL user", "root")?
    } else {
        "root".to_string()
    };
    let password = if interactive {
        prompt_password("MySQL password (leave blank to skip)")?
    } else {
        String::new()
    };

    Ok(DatabaseInit::MySql {
        host,
        port,
        database,
        username,
        password,
    })
}

fn prompt_text(prompt: &str, default_value: &str) -> Result<String, String> {
    if let Some(value) = next_prompt_script_value()? {
        return Ok(if value.is_empty() {
            default_value.to_string()
        } else {
            value
        });
    }

    Input::<String>::new()
        .with_prompt(prompt)
        .default(default_value.to_string())
        .interact_text()
        .map_err(|error| format!("Failed to read {}: {}", prompt, error))
}

fn prompt_u16(prompt: &str, default_value: u16) -> Result<u16, String> {
    if let Some(value) = next_prompt_script_value()? {
        if value.is_empty() {
            return Ok(default_value);
        }

        return value
            .parse::<u16>()
            .map_err(|error| format!("Failed to parse {}: {}", prompt, error));
    }

    Input::<u16>::new()
        .with_prompt(prompt)
        .default(default_value)
        .interact_text()
        .map_err(|error| format!("Failed to read {}: {}", prompt, error))
}

fn prompt_password(prompt: &str) -> Result<String, String> {
    if let Some(value) = next_prompt_script_value()? {
        return Ok(value);
    }

    Password::new()
        .with_prompt(prompt)
        .allow_empty_password(true)
        .interact()
        .map_err(|error| format!("Failed to read {}: {}", prompt, error))
}

fn prompt_confirm(prompt: &str) -> Result<bool, String> {
    if let Some(value) = next_prompt_script_value()? {
        return parse_prompt_confirm(prompt, &value);
    }

    Ok(confirm(prompt))
}

fn parse_prompt_confirm(prompt: &str, value: &str) -> Result<bool, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "" | "n" | "no" | "false" | "0" => Ok(false),
        "y" | "yes" | "true" | "1" => Ok(true),
        other => Err(format!("Failed to parse {} response: {}", prompt, other)),
    }
}

fn has_prompt_script() -> bool {
    std::env::var_os("TIDEORM_PROMPT_SCRIPT").is_some()
}

fn next_prompt_script_value() -> Result<Option<String>, String> {
    if !has_prompt_script() {
        return Ok(None);
    }

    let mut script = PROMPT_SCRIPT.lock().map_err(|_| "Prompt script lock poisoned".to_string())?;
    if script.is_none() {
        let raw = std::env::var("TIDEORM_PROMPT_SCRIPT").unwrap_or_default();
        let values = raw
            .split('\n')
            .map(|value| value.trim_end_matches('\r').to_string())
            .collect::<VecDeque<_>>();
        *script = Some(values);
    }

    match script.as_mut().and_then(|values| values.pop_front()) {
        Some(value) => Ok(Some(value)),
        None => Err("TIDEORM_PROMPT_SCRIPT ran out of answers".to_string()),
    }
}

fn print_setup_intro(database: &str) {
    println!("\n{}", "Setup".cyan().bold());
    println!(
        "{}",
        format!(
            "Answer a few questions to configure your {} project. Press Enter to accept the default value.",
            database_label(database)
        )
        .dimmed()
    );
}

fn print_next_steps(options: &InitOptions, wrote_project_config: bool) {
    println!("\n{}", "Next steps:".cyan().bold());

    if wrote_project_config {
        println!(
            "  1. Review {} and {}",
            options.env_file_name.yellow(),
            "tideorm.toml".yellow()
        );
    } else {
        println!("  1. Review your existing {}", "tideorm.toml".yellow());
    }

    println!("  2. Create your first model:");
    println!(
        "     {}",
        "tideorm make model User --fields=\"name:string,email:string:unique\" --migration"
            .yellow()
    );

    if options.run_migrations_now {
        println!("  3. Migrations were already run during setup.");
    } else {
        println!("  3. Run migrations:");
        println!("     {}", "tideorm migrate run".yellow());
    }

    println!("  4. Seed the database:");
    println!("     {}", "tideorm db seed".yellow());
}

fn database_label(database: &str) -> &'static str {
    match normalize_database(database) {
        "sqlite" => "SQLite",
        "mysql" => "MySQL",
        _ => "Postgres",
    }
}

fn normalize_database(database: &str) -> &str {
    match database.to_ascii_lowercase().as_str() {
        "sqlite" => "sqlite",
        "mysql" => "mysql",
        _ => "postgres",
    }
}

fn write_env_file(options: &InitOptions) -> Result<(), String> {
    let env_path = std::path::Path::new(&options.env_file_name);
    let existed = env_path.exists();
    upsert_env_value(env_path, "DATABASE_URL", &options.database.connection_url())?;

    if existed {
        print_success(&format!("Updated {}", options.env_file_name));
    } else {
        print_success(&format!("Created {}", options.env_file_name));
    }

    Ok(())
}

fn write_tideorm_config(options: &InitOptions) -> Result<(), String> {
    if file_exists("tideorm.toml") && !options.overwrite_config {
        print_warning("tideorm.toml already exists, keeping current file...");
        return Ok(());
    }

    std::fs::write("tideorm.toml", generate_tideorm_toml(options))
        .map_err(|error| format!("Failed to create config file: {}", error))?;
    print_success("Created tideorm.toml");
    Ok(())
}

fn create_project_structure(verbose: bool) -> Result<(), String> {
    let directories = ["src/models", "src/migrations", "src/seeders", "src/factories"];

    for dir in directories {
        ensure_directory(dir)?;
        if verbose {
            print_success(&format!("Created directory: {}", dir));
        }
    }

    let mod_files = [
        ("src/models/mod.rs", "//! Database models\n"),
        ("src/migrations/mod.rs", "//! Database migrations\n"),
        ("src/seeders/mod.rs", "//! Database seeders\n"),
        ("src/factories/mod.rs", "//! Model factories\n"),
    ];

    for (path, content) in mod_files {
        if !file_exists(path) {
            std::fs::write(path, content)
                .map_err(|error| format!("Failed to create {}: {}", path, error))?;
        }
    }

    print_success("Created directory structure");
    Ok(())
}

fn create_scaffold_files(project_path: &std::path::Path, database: &str) -> Result<(), String> {
    if !file_exists("Cargo.toml") {
        let package_name = infer_package_name(project_path);
        let cargo_toml_content = generate_cargo_toml(&package_name, database);
        std::fs::write("Cargo.toml", cargo_toml_content)
            .map_err(|error| format!("Failed to create Cargo.toml: {}", error))?;
        print_success("Created Cargo.toml");
    }

    if !file_exists("src/main.rs") {
        std::fs::write("src/main.rs", generate_main_rs())
            .map_err(|error| format!("Failed to create src/main.rs: {}", error))?;
        print_success("Created src/main.rs");
    }

    if !file_exists("src/config.rs") {
        std::fs::write("src/config.rs", generate_config_rs(database))
            .map_err(|error| format!("Failed to create config.rs: {}", error))?;
        print_success("Created src/config.rs");
    }

    if !file_exists("src/seeders/database_seeder.rs") {
        std::fs::write("src/seeders/database_seeder.rs", generate_database_seeder())
            .map_err(|error| format!("Failed to create DatabaseSeeder: {}", error))?;
        print_success("Created DatabaseSeeder");

        let mod_path = "src/seeders/mod.rs";
        let mod_content = std::fs::read_to_string(mod_path).unwrap_or_default();
        if !mod_content.contains("database_seeder") {
            std::fs::write(mod_path, format!("{}pub mod database_seeder;\n", mod_content))
                .map_err(|error| format!("Failed to update mod.rs: {}", error))?;
        }
    }

    Ok(())
}

async fn create_database_from_config(
    config_path: &str,
    options: &InitOptions,
    verbose: bool,
) -> Result<(), String> {
    let config = TideConfig::load(config_path)?;

    if let Some(database_name) = options.database.database_name() {
        if verbose {
            print_info(&format!("Creating database: {}", database_name));
        }
        runtime_db::create_database(&config, database_name).await?;
        print_success(&format!("Created database '{}'", database_name));
    } else if let DatabaseInit::Sqlite { path } = &options.database {
        if verbose {
            print_info(&format!("Creating SQLite database file: {}", path));
        }
        runtime_db::create_database(&config, path).await?;
        print_success(&format!("Created SQLite database '{}'", path));
    }

    Ok(())
}

fn generate_tideorm_toml(options: &InitOptions) -> String {
    match &options.database {
        DatabaseInit::Sqlite { path } => format!(
            r#"# TideORM Configuration File
# This file configures the TideORM CLI and runtime behavior.

[project]
name = "my-tideorm-project"
environment = "development"
env_file = "{env_file}"

[database]
driver = "sqlite"
sqlite_path = "{path}"
url = "${{DATABASE_URL}}"
pool_size = 5
timeout = 30

[paths]
models = "src/models"
migrations = "src/migrations"
seeders = "src/seeders"
factories = "src/factories"
config_file = "src/config.rs"

[migration]
table = "_migrations"
timestamps = true

[seeder]
default_seeder = "DatabaseSeeder"

[model]
timestamps = true
soft_deletes = false
tokenize = false
primary_key = "id"
primary_key_type = "i64"
"#,
            env_file = options.env_file_name,
            path = path
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
environment = "development"
env_file = "{env_file}"

[database]
driver = "postgres"
host = "{host}"
port = {port}
database = "{database}"
username = "{username}"
password = ""
url = "${{DATABASE_URL}}"
pool_size = 5
timeout = 30

[paths]
models = "src/models"
migrations = "src/migrations"
seeders = "src/seeders"
factories = "src/factories"
config_file = "src/config.rs"

[migration]
table = "_migrations"
timestamps = true

[seeder]
default_seeder = "DatabaseSeeder"

[model]
timestamps = true
soft_deletes = false
tokenize = false
primary_key = "id"
primary_key_type = "i64"
"#,
            env_file = options.env_file_name,
            host = host,
            port = port,
            database = database,
            username = username
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
environment = "development"
env_file = "{env_file}"

[database]
driver = "mysql"
host = "{host}"
port = {port}
database = "{database}"
username = "{username}"
password = ""
url = "${{DATABASE_URL}}"
pool_size = 5
timeout = 30

[paths]
models = "src/models"
migrations = "src/migrations"
seeders = "src/seeders"
factories = "src/factories"
config_file = "src/config.rs"

[migration]
table = "_migrations"
timestamps = true

[seeder]
default_seeder = "DatabaseSeeder"

[model]
timestamps = true
soft_deletes = false
tokenize = false
primary_key = "id"
primary_key_type = "i64"
"#,
            env_file = options.env_file_name,
            host = host,
            port = port,
            database = database,
            username = username
        ),
    }
}

fn upsert_env_value(path: &std::path::Path, key: &str, value: &str) -> Result<(), String> {
    let entry = format!("{}={}", key, value);
    let existing = if path.exists() {
        std::fs::read_to_string(path)
            .map_err(|error| format!("Failed to read {}: {}", path.display(), error))?
    } else {
        String::new()
    };

    let mut replaced = false;
    let mut lines = Vec::new();
    for line in existing.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with(&format!("{}=", key)) || trimmed.starts_with(&format!("export {}=", key)) {
            lines.push(entry.clone());
            replaced = true;
        } else {
            lines.push(line.to_string());
        }
    }

    if !replaced {
        lines.push(entry);
    }

    let mut content = lines.join("\n");
    if !content.ends_with('\n') {
        content.push('\n');
    }

    std::fs::write(path, content)
        .map_err(|error| format!("Failed to write {}: {}", path.display(), error))
}

fn build_url(
    scheme: &str,
    host: &str,
    port: u16,
    database: &str,
    username: &str,
    password: &str,
) -> String {
    if password.is_empty() {
        format!("{}://{}@{}:{}/{}", scheme, username, host, port, database)
    } else {
        format!(
            "{}://{}:{}@{}:{}/{}",
            scheme, username, password, host, port, database
        )
    }
}

fn sqlite_url(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    let bytes = normalized.as_bytes();
    let is_windows_absolute = bytes.len() >= 3 && bytes[1] == b':' && bytes[2] == b'/';
    if is_windows_absolute {
        format!("sqlite:///{}", normalized)
    } else {
        format!("sqlite://{}", normalized)
    }
}

fn generate_config_rs(database: &str) -> String {
    let db_setup = match database {
        "sqlite" => r#"
    // SQLite setup
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "sqlite://database.db".to_string());

    TideConfig::init()
        .database_type(DatabaseType::SQLite)
        .database(&database_url)
        .max_connections(5)
        .min_connections(1)
"#,
        "mysql" => r#"
    // MySQL setup
    let database_url = std::env::var("DATABASE_URL")
        .expect("DATABASE_URL must be set");

    TideConfig::init()
        .database_type(DatabaseType::MySQL)
        .database(&database_url)
        .max_connections(10)
        .min_connections(2)
"#,
        _ => r#"
    // PostgreSQL setup
    let database_url = std::env::var("DATABASE_URL")
        .expect("DATABASE_URL must be set");

    TideConfig::init()
        .database_type(DatabaseType::Postgres)
        .database(&database_url)
        .max_connections(10)
        .min_connections(2)
"#,
    };

    format!(
        r#"//! TideORM Configuration
//!
//! This file configures the TideORM database connection and runtime settings.
//! It is used by both the application and the TideORM CLI.

use tideorm::prelude::*;

/// Build the TideORM configuration
pub fn tideorm_config() -> TideConfig {{{db_setup}
}}

pub async fn connect_tideorm() -> tideorm::Result<&'static Database> {{
    tideorm_config().connect().await
}}

pub fn get_cli_config() -> TideConfig {{
    tideorm_config()
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
        db_setup = db_setup
    )
}

fn infer_package_name(project_path: &std::path::Path) -> String {
    project_path
        .file_name()
        .and_then(|name| name.to_str())
        .map(crate::utils::to_snake_case)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "tideorm_app".to_string())
}

fn generate_cargo_toml(package_name: &str, database: &str) -> String {
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

[dependencies]
tokio = {{ version = "1", features = ["full"] }}
serde = {{ version = "1", features = ["derive"] }}
chrono = "0.4"
tideorm = {{ version = "0.8.7", features = ["{database_feature}", "runtime-tokio"] }}
"#,
        package_name = package_name,
        database_feature = database_feature,
    )
}

fn generate_main_rs() -> String {
    r#"pub mod config;
pub mod factories;
pub mod migrations;
pub mod models;
pub mod seeders;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    config::connect_tideorm().await?;
    println!("TideORM project initialized.");
    Ok(())
}
"#
    .to_string()
}

fn generate_database_seeder() -> String {
    r#"//! Database Seeder
//!
//! This is the main seeder that runs all other seeders.
//! Use `tideorm db seed` to run this seeder.

use tideorm::prelude::*;

#[derive(Default)]
pub struct DatabaseSeeder;

#[async_trait]
impl Seed for DatabaseSeeder {
    fn name(&self) -> &str {
        "database_seeder"
    }

    async fn run(&self, _db: &Database) -> tideorm::Result<()> {
        println!("Running database seeders...");
        println!("Database seeding completed!");
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

#[cfg(test)]
mod tests {
    use super::{
        generate_cargo_toml, generate_tideorm_toml, infer_package_name, run, upsert_env_value,
        DatabaseInit, InitOptions,
    };
    use std::fs;
    use std::sync::{LazyLock, Mutex};
    use tempfile::TempDir;

    static INIT_TEST_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    #[test]
    fn generated_cargo_toml_uses_current_tideorm_version() {
        let cargo_toml = generate_cargo_toml("demo_app", "sqlite");

        assert!(cargo_toml.contains("tideorm = { version = \"0.8.7\""));
        assert!(cargo_toml.contains("serde = { version = \"1\", features = [\"derive\"] }"));
        assert!(cargo_toml.contains("chrono = \"0.4\""));
        assert!(cargo_toml.contains("features = [\"sqlite\", \"runtime-tokio\"]"));
    }

    #[test]
    fn infer_package_name_falls_back_for_empty_paths() {
        let package_name = infer_package_name(std::path::Path::new(""));
        assert_eq!(package_name, "tideorm_app");
    }

    #[test]
    fn generate_tideorm_toml_uses_custom_env_file() {
        let options = InitOptions {
            env_file_name: ".env.local".to_string(),
            overwrite_config: true,
            database: DatabaseInit::Postgres {
                host: "localhost".to_string(),
                port: 5432,
                database: "app_db".to_string(),
                username: "postgres".to_string(),
                password: "secret".to_string(),
            },
            create_database_now: false,
            run_migrations_now: false,
        };

        let toml = generate_tideorm_toml(&options);
        assert!(toml.contains("env_file = \".env.local\""));
        assert!(toml.contains("url = \"${DATABASE_URL}\""));
        assert!(toml.contains("database = \"app_db\""));
    }

    #[test]
    fn upsert_env_value_updates_existing_database_url() {
        let dir = TempDir::new().unwrap();
        let env_path = dir.path().join(".env");
        fs::write(&env_path, "APP_NAME=demo\nDATABASE_URL=old\n").unwrap();

        upsert_env_value(&env_path, "DATABASE_URL", "new").unwrap();

        let updated = fs::read_to_string(&env_path).unwrap();
        assert!(updated.contains("APP_NAME=demo"));
        assert!(updated.contains("DATABASE_URL=new"));
        assert!(!updated.contains("DATABASE_URL=old"));
    }

    #[tokio::test]
    async fn run_restores_working_directory() {
        let _guard = INIT_TEST_LOCK.lock().unwrap();
        let workspace = TempDir::new().unwrap();
        let original_dir = std::env::current_dir().unwrap();
        unsafe {
            std::env::set_var("TIDEORM_NONINTERACTIVE", "1");
        }
        std::env::set_current_dir(workspace.path()).unwrap();

        let result = run("generated", "sqlite", false).await;
        let restored_dir = std::env::current_dir().unwrap();

        std::env::set_current_dir(&original_dir).unwrap();
        unsafe {
            std::env::remove_var("TIDEORM_NONINTERACTIVE");
        }

        assert!(result.is_ok());
        assert_eq!(restored_dir, workspace.path());
    }

    #[tokio::test]
    async fn run_keeps_existing_config_without_mutating_env_file() {
        let _guard = INIT_TEST_LOCK.lock().unwrap();
        let workspace = TempDir::new().unwrap();
        let project_dir = workspace.path().join("existing_project");
        fs::create_dir_all(&project_dir).unwrap();
        fs::write(project_dir.join("tideorm.toml"), "[project]\nname = \"demo\"\n").unwrap();
        fs::write(project_dir.join(".env"), "DATABASE_URL=preserve-me\n").unwrap();

        let original_dir = std::env::current_dir().unwrap();
        unsafe {
            std::env::set_var("TIDEORM_NONINTERACTIVE", "1");
        }
        std::env::set_current_dir(workspace.path()).unwrap();

        let result = run(project_dir.to_str().unwrap(), "sqlite", false).await;
        let env_contents = fs::read_to_string(project_dir.join(".env")).unwrap();

        std::env::set_current_dir(&original_dir).unwrap();
        unsafe {
            std::env::remove_var("TIDEORM_NONINTERACTIVE");
        }

        assert!(result.is_ok());
        assert_eq!(env_contents, "DATABASE_URL=preserve-me\n");
    }
}
