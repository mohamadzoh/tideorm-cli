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
    
    TideConfig::new()
        .database_url(&database_url)
        .pool_size(5)
"#,
        "mysql" => r#"
    // MySQL setup
    let database_url = std::env::var("DATABASE_URL")
        .expect("DATABASE_URL must be set");
    
    TideConfig::new()
        .database_url(&database_url)
        .pool_size(10)
"#,
        _ => r#"
    // PostgreSQL setup
    let database_url = std::env::var("DATABASE_URL")
        .expect("DATABASE_URL must be set");
    
    TideConfig::new()
        .database_url(&database_url)
        .pool_size(10)
"#,
    };

    format!(
        r#"//! TideORM Configuration
//!
//! This file configures the TideORM database connection and runtime settings.
//! It is used by both the application and the TideORM CLI.

use tideorm::prelude::*;

/// Initialize TideORM configuration
///
/// Call this function at application startup to configure the database connection.
///
/// # Example
///
/// ```rust
/// #[tokio::main]
/// async fn main() -> Result<(), Box<dyn std::error::Error>> {{
///     // Load environment variables
///     dotenv::dotenv().ok();
///     
///     // Initialize TideORM
///     let config = crate::config::init_tideorm();
///     tideorm::init(config).await?;
///     
///     // Your application code here...
///     
///     Ok(())
/// }}
/// ```
pub fn init_tideorm() -> TideConfig {{{db_setup}
        .build()
}}

/// Get the TideORM configuration for CLI usage
///
/// This function is called by the TideORM CLI to get the database configuration.
/// It reads from environment variables to avoid hardcoding credentials.
pub fn get_cli_config() -> TideConfig {{
    dotenv::dotenv().ok();
    init_tideorm()
}}

#[cfg(test)]
mod tests {{
    use super::*;

    #[test]
    fn test_config_creation() {{
        // This test just ensures the config can be created
        // In a real scenario, you'd want to mock the environment
        std::env::set_var("DATABASE_URL", "sqlite://test.db");
        let _config = init_tideorm();
    }}
}}
"#,
        db_setup = db_setup
    )
}

/// Generate DatabaseSeeder content
fn generate_database_seeder() -> String {
    r#"//! Database Seeder
//!
//! This is the main seeder that runs all other seeders.
//! Use `tideorm db seed` to run this seeder.

use tideorm::prelude::*;

/// Database seeder - runs all seeders
pub struct DatabaseSeeder;

impl DatabaseSeeder {
    /// Run all seeders
    pub async fn run() -> tideorm::Result<()> {
        println!("Running database seeders...");

        // Add your seeders here:
        // UserSeeder::run().await?;
        // PostSeeder::run().await?;

        println!("Database seeding completed!");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_seeder_runs() {
        // This test just ensures the seeder can be called
        // In a real scenario, you'd want to use a test database
    }
}
"#
    .to_string()
}
