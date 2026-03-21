//! Init command for TideORM CLI

use crate::config::default_config_content;
use crate::utils::{ensure_directory, file_exists, print_info, print_success, print_warning};
use colored::Colorize;

/// Initialize a new TideORM project
pub async fn run(name: &str, database: &str, verbose: bool) -> Result<(), String> {
    let project_path = if name == "." {
        std::env::current_dir()
            .map_err(|e| format!("Failed to get current directory: {}", e))?
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

    // Create project directory if it doesn't exist
    if !project_path.exists() {
        std::fs::create_dir_all(&project_path)
            .map_err(|e| format!("Failed to create project directory: {}", e))?;
        print_success(&format!("Created directory: {}", project_path.display()));
    }

    // Change to project directory for relative paths
    std::env::set_current_dir(&project_path)
        .map_err(|e| format!("Failed to change directory: {}", e))?;

    // Create configuration file
    let config_path = "tideorm.toml";
    if file_exists(config_path) {
        print_warning("tideorm.toml already exists, skipping...");
    } else {
        let config_content = default_config_content(database);
        std::fs::write(config_path, config_content)
            .map_err(|e| format!("Failed to create config file: {}", e))?;
        print_success("Created tideorm.toml");
    }

    // Create directory structure
    let directories = [
        "src/models",
        "src/migrations",
        "src/seeders",
        "src/factories",
    ];

    for dir in directories {
        ensure_directory(dir)?;
        if verbose {
            print_success(&format!("Created directory: {}", dir));
        }
    }

    // Create mod.rs files
    let mod_files = [
        ("src/models/mod.rs", "//! Database models\n"),
        ("src/migrations/mod.rs", "//! Database migrations\n"),
        ("src/seeders/mod.rs", "//! Database seeders\n"),
        ("src/factories/mod.rs", "//! Model factories\n"),
    ];

    for (path, content) in mod_files {
        if !file_exists(path) {
            std::fs::write(path, content)
                .map_err(|e| format!("Failed to create {}: {}", path, e))?;
        }
    }

    print_success("Created directory structure");

    // Create Cargo.toml if it doesn't exist so `tideorm init` produces a runnable Rust project
    let cargo_toml_path = "Cargo.toml";
    if !file_exists(cargo_toml_path) {
        let package_name = infer_package_name(&project_path);
        let cargo_toml_content = generate_cargo_toml(&package_name, database);
        std::fs::write(cargo_toml_path, cargo_toml_content)
            .map_err(|e| format!("Failed to create Cargo.toml: {}", e))?;
        print_success("Created Cargo.toml");
    }

    // Create main.rs if it doesn't exist
    let main_rs_path = "src/main.rs";
    if !file_exists(main_rs_path) {
        let main_rs_content = generate_main_rs();
        std::fs::write(main_rs_path, main_rs_content)
            .map_err(|e| format!("Failed to create src/main.rs: {}", e))?;
        print_success("Created src/main.rs");
    }

    // Create config.rs if it doesn't exist
    let config_rs_path = "src/config.rs";
    if !file_exists(config_rs_path) {
        let config_rs_content = generate_config_rs(database);
        std::fs::write(config_rs_path, config_rs_content)
            .map_err(|e| format!("Failed to create config.rs: {}", e))?;
        print_success("Created src/config.rs");
    }

    // Create DatabaseSeeder
    let seeder_path = "src/seeders/database_seeder.rs";
    if !file_exists(seeder_path) {
        let seeder_content = generate_database_seeder();
        std::fs::write(seeder_path, seeder_content)
            .map_err(|e| format!("Failed to create DatabaseSeeder: {}", e))?;
        print_success("Created DatabaseSeeder");
        
        // Update mod.rs
        let mod_path = "src/seeders/mod.rs";
        let mod_content = std::fs::read_to_string(mod_path).unwrap_or_default();
        if !mod_content.contains("database_seeder") {
            std::fs::write(mod_path, format!("{}pub mod database_seeder;\n", mod_content))
                .map_err(|e| format!("Failed to update mod.rs: {}", e))?;
        }
    }

    println!("{}", "─".repeat(50));
    println!("\n{}", "✓ TideORM project initialized successfully!".green().bold());
    
    println!("\n{}", "Next steps:".cyan().bold());
    println!("  1. Update tideorm.toml with your database configuration");
    println!("  2. Create your first model:");
    println!("     {}", "tideorm make model User --fields=\"name:string,email:string:unique\" --migration".yellow());
    println!("  3. Run migrations:");
    println!("     {}", "tideorm migrate run".yellow());
    println!("  4. Seed the database:");
    println!("     {}", "tideorm db seed".yellow());

    Ok(())
}

/// Generate config.rs content
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
///
/// Call this function if you want to register more options before connecting.
///
/// # Example
///
/// ```rust
/// #[tokio::main]
/// async fn main() -> Result<(), Box<dyn std::error::Error>> {{
///     crate::config::connect_tideorm().await?;
///     
///     // Your application code here...
///     
///     Ok(())
/// }}
/// ```
pub fn tideorm_config() -> TideConfig {{{db_setup}
}}

/// Connect TideORM using the generated configuration.
pub async fn connect_tideorm() -> tideorm::Result<&'static Database> {{
    tideorm_config().connect().await
}}

/// Get the TideORM configuration for CLI usage.
///
/// This returns the builder before `connect()` so callers can add extra options.
pub fn get_cli_config() -> TideConfig {{
    tideorm_config()
}}

#[cfg(test)]
mod tests {{
    use super::*;

    #[test]
    fn test_config_creation() {{
        // This test just ensures the config can be created.
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

/// Generate DatabaseSeeder content
fn generate_database_seeder() -> String {
    r#"//! Database Seeder
//!
//! This is the main seeder that runs all other seeders.
//! Use `tideorm db seed` to run this seeder.

use tideorm::prelude::*;

/// Database seeder - runs all seeders
#[derive(Default)]
pub struct DatabaseSeeder;

#[async_trait]
impl Seed for DatabaseSeeder {
    fn name(&self) -> &str {
        "database_seeder"
    }

    async fn run(&self, _db: &Database) -> tideorm::Result<()> {
        println!("Running database seeders...");

        // Add your seeders here:
        // let result = Seeder::new()
        //     .add(UserSeeder::default())
        //     .add(PostSeeder::default())
        //     .run()
        //     .await?;
        // println!("{}", result);

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
    use super::{generate_cargo_toml, infer_package_name};

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
}
