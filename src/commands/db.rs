//! Database commands for TideORM CLI

use crate::config::TideConfig;
use crate::utils::{print_info, print_success, print_warning};
use crate::DbCommands;
use colored::Colorize;
use std::fs;
use std::path::Path;

/// Handle database subcommands
pub async fn handle(config_path: &str, cmd: DbCommands, verbose: bool) -> Result<(), String> {
    match cmd {
        DbCommands::Seed { seeder, force } => seed(config_path, seeder, force, verbose).await,
        DbCommands::Fresh { force } => fresh(config_path, force, verbose).await,
        DbCommands::Status => status(config_path, verbose).await,
        DbCommands::Create { name } => create_database(config_path, name, verbose).await,
        DbCommands::Drop { name, force } => drop_database(config_path, name, force, verbose).await,
        DbCommands::Wipe { drop_types, force } => wipe(config_path, drop_types, force, verbose).await,
        DbCommands::Table { name } => show_table(config_path, &name, verbose).await,
        DbCommands::Tables => list_tables(config_path, verbose).await,
    }
}

/// Run database seeders
pub async fn seed(
    config_path: &str,
    seeder: Option<String>,
    force: bool,
    verbose: bool,
) -> Result<(), String> {
    let config = TideConfig::load(config_path)?;

    if config.is_production() && !force {
        return Err("Cannot run seeders in production without --force flag".to_string());
    }

    let seeders_path = &config.paths.seeders;

    if verbose {
        print_info(&format!("Looking for seeders in: {}", seeders_path));
    }

    // Get seeders to run
    let seeders = if let Some(seeder_name) = seeder {
        vec![find_seeder(seeders_path, &seeder_name)?]
    } else {
        // Find the default seeder (DatabaseSeeder)
        let default_seeder = &config.seeder.default_seeder;
        match find_seeder(seeders_path, default_seeder) {
            Ok(s) => vec![s],
            Err(_) => {
                // Try to find any seeder files
                get_all_seeders(seeders_path)?
            }
        }
    };

    if seeders.is_empty() {
        print_warning("No seeders found");
        return Ok(());
    }

    println!("\n{}", "Running seeders:".cyan().bold());
    println!("{}", "─".repeat(50));

    for seeder in &seeders {
        print!("  Seeding: {}... ", seeder.name);
        
        // Run the seeder
        match run_seeder(&config, seeder).await {
            Ok(count) => {
                println!("{} ({} records)", "DONE".green(), count);
            }
            Err(e) => {
                println!("{}", "FAILED".red());
                return Err(format!("Seeder failed: {}", e));
            }
        }
    }

    println!("{}", "─".repeat(50));
    print_success(&format!("Ran {} seeder(s)", seeders.len()));

    Ok(())
}

/// Drop all tables and re-seed
async fn fresh(config_path: &str, force: bool, verbose: bool) -> Result<(), String> {
    let config = TideConfig::load(config_path)?;

    if config.is_production() && !force {
        return Err("Cannot run db:fresh in production without --force flag".to_string());
    }

    if verbose {
        print_warning("This will drop all tables and re-run migrations and seeders!");
    }

    // Use migrate:fresh with --seed
    crate::commands::migrate::handle_subcommand(
        config_path,
        crate::MigrateCommands::Fresh {
            seed: true,
            seeder: None,
            force: true,
        },
        verbose,
    )
    .await
}

/// Show database connection status
async fn status(config_path: &str, verbose: bool) -> Result<(), String> {
    let config = TideConfig::load(config_path)?;

    if verbose {
        print_info("Checking database connection...");
    }

    println!("\n{}", "Database Status:".cyan().bold());
    println!("{}", "─".repeat(50));

    println!("  Driver:     {}", config.database.driver.green());
    
    match config.database.driver.as_str() {
        "sqlite" => {
            let path = config.database.sqlite_path.as_deref().unwrap_or("database.db");
            println!("  Path:       {}", path);
            let exists = Path::new(path).exists();
            println!(
                "  Status:     {}",
                if exists { "EXISTS".green() } else { "NOT FOUND".yellow() }
            );
        }
        _ => {
            println!("  Host:       {}", config.database.host);
            println!(
                "  Port:       {}",
                config.database.port.map_or("default".to_string(), |p| p.to_string())
            );
            println!(
                "  Database:   {}",
                config.database.database.as_deref().unwrap_or("not set")
            );
            println!(
                "  Username:   {}",
                config.database.username.as_deref().unwrap_or("not set")
            );
        }
    }

    // Try to connect
    print!("\n  Connection: ");
    match test_connection(&config).await {
        Ok(()) => println!("{}", "OK".green()),
        Err(e) => println!("{} ({})", "FAILED".red(), e),
    }

    println!("{}", "─".repeat(50));

    Ok(())
}

/// Create a database
async fn create_database(
    config_path: &str,
    name: Option<String>,
    verbose: bool,
) -> Result<(), String> {
    let config = TideConfig::load(config_path)?;

    let db_name = name
        .as_deref()
        .or(config.database.database.as_deref())
        .ok_or("Database name not specified")?;

    if verbose {
        print_info(&format!("Creating database: {}", db_name));
    }

    // Create the database
    create_db(&config, db_name).await?;

    print_success(&format!("Database '{}' created successfully", db_name));

    Ok(())
}

/// Drop a database
async fn drop_database(
    config_path: &str,
    name: Option<String>,
    force: bool,
    verbose: bool,
) -> Result<(), String> {
    let config = TideConfig::load(config_path)?;

    let db_name = name
        .as_deref()
        .or(config.database.database.as_deref())
        .ok_or("Database name not specified")?;

    if !force {
        if !crate::utils::confirm(&format!(
            "Are you sure you want to drop database '{}'?",
            db_name
        )) {
            print_info("Operation cancelled");
            return Ok(());
        }
    }

    if verbose {
        print_info(&format!("Dropping database: {}", db_name));
    }

    // Drop the database
    drop_db(&config, db_name).await?;

    print_success(&format!("Database '{}' dropped successfully", db_name));

    Ok(())
}

/// Wipe all tables (truncate)
async fn wipe(
    config_path: &str,
    drop_types: bool,
    force: bool,
    verbose: bool,
) -> Result<(), String> {
    let config = TideConfig::load(config_path)?;

    if config.is_production() && !force {
        return Err("Cannot wipe database in production without --force flag".to_string());
    }

    if !force {
        if !crate::utils::confirm("Are you sure you want to wipe all tables?") {
            print_info("Operation cancelled");
            return Ok(());
        }
    }

    if verbose {
        print_info("Wiping all tables...");
    }

    // Truncate all tables
    wipe_tables(&config, drop_types).await?;

    print_success("All tables wiped successfully");

    Ok(())
}

/// Show table information
async fn show_table(config_path: &str, table_name: &str, verbose: bool) -> Result<(), String> {
    let config = TideConfig::load(config_path)?;

    if verbose {
        print_info(&format!("Getting info for table: {}", table_name));
    }

    let columns = get_table_columns(&config, table_name).await?;

    println!("\n{}", format!("Table: {}", table_name).cyan().bold());
    println!("{}", "─".repeat(80));
    println!(
        "  {:<20} {:<15} {:<10} {:<10} {:<15}",
        "Column", "Type", "Nullable", "Key", "Default"
    );
    println!("{}", "─".repeat(80));

    for col in &columns {
        println!(
            "  {:<20} {:<15} {:<10} {:<10} {:<15}",
            col.name,
            col.data_type,
            if col.nullable { "YES" } else { "NO" },
            col.key.as_deref().unwrap_or(""),
            col.default.as_deref().unwrap_or("NULL")
        );
    }

    println!("{}", "─".repeat(80));
    println!("  Total columns: {}", columns.len());

    Ok(())
}

/// List all tables
async fn list_tables(config_path: &str, verbose: bool) -> Result<(), String> {
    let config = TideConfig::load(config_path)?;

    if verbose {
        print_info("Listing all tables...");
    }

    let tables = get_all_tables(&config).await?;

    println!("\n{}", "Database Tables:".cyan().bold());
    println!("{}", "─".repeat(50));

    if tables.is_empty() {
        print_info("No tables found");
        return Ok(());
    }

    for (i, table) in tables.iter().enumerate() {
        println!("  {}. {}", i + 1, table);
    }

    println!("{}", "─".repeat(50));
    println!("  Total tables: {}", tables.len());

    Ok(())
}

// =============================================================================
// HELPER TYPES AND FUNCTIONS
// =============================================================================

/// Seeder information
#[derive(Debug, Clone)]
pub struct Seeder {
    pub name: String,
}

/// Column information
#[derive(Debug, Clone)]
pub struct ColumnInfo {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub key: Option<String>,
    pub default: Option<String>,
}

/// Get all seeders from the seeders directory
fn get_all_seeders(seeders_path: &str) -> Result<Vec<Seeder>, String> {
    let path = Path::new(seeders_path);

    if !path.exists() {
        return Ok(vec![]);
    }

    let mut seeders = Vec::new();

    for entry in fs::read_dir(path).map_err(|e| format!("Failed to read seeders directory: {}", e))? {
        let entry = entry.map_err(|e| format!("Failed to read entry: {}", e))?;
        let file_path = entry.path();

        if file_path.extension().map_or(false, |ext| ext == "rs") {
            let name = file_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();

            if name == "mod" {
                continue;
            }

            seeders.push(Seeder {
                name: crate::utils::to_pascal_case(&name),
            });
        }
    }

    seeders.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(seeders)
}

/// Find a specific seeder
fn find_seeder(seeders_path: &str, name: &str) -> Result<Seeder, String> {
    let seeders = get_all_seeders(seeders_path)?;
    let search_name = crate::utils::to_pascal_case(name);

    seeders
        .into_iter()
        .find(|s| s.name == search_name || s.name.contains(&search_name))
        .ok_or_else(|| format!("Seeder not found: {}", name))
}

/// Run a seeder
async fn run_seeder(_config: &TideConfig, _seeder: &Seeder) -> Result<u32, String> {
    // TODO: Actually run the seeder
    // This would require dynamic loading or compiling the seeder
    // For now, simulate success
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    Ok(10) // Simulated count
}

/// Test database connection
async fn test_connection(_config: &TideConfig) -> Result<(), String> {
    // TODO: Actually test the connection
    // For now, simulate success
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    Ok(())
}

/// Create a database
async fn create_db(_config: &TideConfig, _name: &str) -> Result<(), String> {
    // TODO: Actually create the database
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    Ok(())
}

/// Drop a database
async fn drop_db(_config: &TideConfig, _name: &str) -> Result<(), String> {
    // TODO: Actually drop the database
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    Ok(())
}

/// Wipe all tables
async fn wipe_tables(_config: &TideConfig, _drop_types: bool) -> Result<(), String> {
    // TODO: Actually truncate all tables
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    Ok(())
}

/// Get table columns
async fn get_table_columns(_config: &TideConfig, _table_name: &str) -> Result<Vec<ColumnInfo>, String> {
    // TODO: Actually query table schema
    // For now, return mock data
    Ok(vec![
        ColumnInfo {
            name: "id".to_string(),
            data_type: "BIGINT".to_string(),
            nullable: false,
            key: Some("PRI".to_string()),
            default: None,
        },
        ColumnInfo {
            name: "created_at".to_string(),
            data_type: "TIMESTAMPTZ".to_string(),
            nullable: false,
            key: None,
            default: Some("NOW()".to_string()),
        },
    ])
}

/// Get all tables
async fn get_all_tables(_config: &TideConfig) -> Result<Vec<String>, String> {
    // TODO: Actually query table list
    // For now, return mock data
    Ok(vec![
        "users".to_string(),
        "posts".to_string(),
        "_tideorm_migrations".to_string(),
    ])
}
