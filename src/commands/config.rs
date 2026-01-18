//! Config command for TideORM CLI

use crate::config::TideConfig;
use crate::utils::print_info;
use colored::Colorize;

/// Show TideORM configuration
pub async fn show(config_path: &str, verbose: bool) -> Result<(), String> {
    if verbose {
        print_info(&format!("Reading configuration from: {}", config_path));
    }

    let config = TideConfig::load(config_path)?;

    println!("\n{}", "TideORM Configuration:".cyan().bold());
    println!("{}", "═".repeat(60));

    // Project
    println!("\n{}", "[project]".yellow());
    println!("  name = \"{}\"", config.project.name);
    println!("  environment = \"{}\"", config.project.environment);

    // Database
    println!("\n{}", "[database]".yellow());
    println!("  driver = \"{}\"", config.database.driver);
    
    match config.database.driver.as_str() {
        "sqlite" => {
            println!(
                "  sqlite_path = \"{}\"",
                config.database.sqlite_path.as_deref().unwrap_or("database.db")
            );
        }
        _ => {
            println!("  host = \"{}\"", config.database.host);
            if let Some(port) = config.database.port {
                println!("  port = {}", port);
            }
            if let Some(db) = &config.database.database {
                println!("  database = \"{}\"", db);
            }
            if let Some(user) = &config.database.username {
                println!("  username = \"{}\"", user);
            }
            println!("  password = \"********\"");
        }
    }
    
    if let Some(url) = &config.database.url {
        println!("  url = \"{}\"", mask_password(url));
    }
    
    println!("  pool_size = {}", config.database.pool_size);
    println!("  timeout = {}", config.database.timeout);

    // Paths
    println!("\n{}", "[paths]".yellow());
    println!("  models = \"{}\"", config.paths.models);
    println!("  migrations = \"{}\"", config.paths.migrations);
    println!("  seeders = \"{}\"", config.paths.seeders);
    println!("  factories = \"{}\"", config.paths.factories);
    println!("  config_file = \"{}\"", config.paths.config_file);

    // Migration
    println!("\n{}", "[migration]".yellow());
    println!("  table = \"{}\"", config.migration.table);
    println!("  timestamps = {}", config.migration.timestamps);

    // Seeder
    println!("\n{}", "[seeder]".yellow());
    println!("  default_seeder = \"{}\"", config.seeder.default_seeder);

    // Model
    println!("\n{}", "[model]".yellow());
    println!("  timestamps = {}", config.model.timestamps);
    println!("  soft_deletes = {}", config.model.soft_deletes);
    println!("  tokenize = {}", config.model.tokenize);
    println!("  primary_key = \"{}\"", config.model.primary_key);
    println!("  primary_key_type = \"{}\"", config.model.primary_key_type);

    println!("\n{}", "═".repeat(60));

    // Show connection URL
    println!("\n{}", "Connection URL:".cyan());
    println!("  {}", mask_password(&config.database.connection_url()));

    Ok(())
}

/// Mask password in connection URL
fn mask_password(url: &str) -> String {
    // Match password in URL format: protocol://user:password@host
    let re = regex::Regex::new(r"://([^:]+):([^@]+)@").unwrap();
    re.replace(url, "://$1:********@").to_string()
}
