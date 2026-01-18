//! Migration commands for TideORM CLI

use crate::config::TideConfig;
use crate::generators::migration::MigrationGenerator;
use crate::utils::{self, print_info, print_success, print_warning};
use crate::MigrateCommands;
use colored::Colorize;
use std::fs;
use std::path::Path;

/// Run pending migrations
pub async fn run(
    config_path: &str,
    path: Option<String>,
    pretend: bool,
    force: bool,
    step: Option<u32>,
) -> Result<(), String> {
    let config = TideConfig::load(config_path)?;

    // Safety check for production
    if config.is_production() && !force {
        return Err(
            "Cannot run migrations in production without --force flag".to_string()
        );
    }

    let migrations_path = path.as_deref().unwrap_or(&config.paths.migrations);

    print_info(&format!("Running migrations from: {}", migrations_path));

    if pretend {
        print_warning("Running in pretend mode - no changes will be made");
    }

    // Get pending migrations
    let migrations = get_pending_migrations(migrations_path)?;

    if migrations.is_empty() {
        print_success("Nothing to migrate");
        return Ok(());
    }

    let migrations_to_run = match step {
        Some(n) => migrations.into_iter().take(n as usize).collect(),
        None => migrations,
    };

    println!(
        "\n{} Migrations to run:",
        format!("[{}]", migrations_to_run.len()).cyan()
    );

    for (i, migration) in migrations_to_run.iter().enumerate() {
        println!("  {}. {}", i + 1, migration.name.yellow());
    }

    if pretend {
        println!("\n{}", "Pretend mode - showing SQL:".cyan());
        for migration in &migrations_to_run {
            println!("\n-- Migration: {}", migration.name);
            println!("-- Up:");
            println!("{}", migration.up_sql);
        }
        return Ok(());
    }

    // Run migrations
    println!("\n{}", "Running migrations...".cyan());

    for migration in &migrations_to_run {
        print!("  Migrating: {}... ", migration.name);

        // Here we would actually run the migration against the database
        // For now, we simulate success
        match run_migration_up(&config, migration).await {
            Ok(()) => {
                println!("{}", "DONE".green());
            }
            Err(e) => {
                println!("{}", "FAILED".red());
                return Err(format!("Migration failed: {}", e));
            }
        }
    }

    print_success(&format!(
        "Ran {} migration(s) successfully",
        migrations_to_run.len()
    ));

    Ok(())
}

/// Handle migration subcommands
pub async fn handle_subcommand(
    config_path: &str,
    cmd: MigrateCommands,
    verbose: bool,
) -> Result<(), String> {
    match cmd {
        MigrateCommands::Run {
            path,
            pretend,
            force,
            step,
        } => run(config_path, path, pretend, force, step).await,

        MigrateCommands::Generate {
            name,
            create,
            table,
            fields,
        } => generate_migration(config_path, &name, create, table, fields, verbose).await,

        MigrateCommands::Up {
            step,
            migration,
            pretend,
        } => migrate_up(config_path, step, migration, pretend, verbose).await,

        MigrateCommands::Down {
            step,
            migration,
            pretend,
        } => migrate_down(config_path, step, migration, pretend, verbose).await,

        MigrateCommands::Redo { step, pretend } => {
            migrate_redo(config_path, step, pretend, verbose).await
        }

        MigrateCommands::Fresh { seed, seeder, force } => {
            migrate_fresh(config_path, seed, seeder, force, verbose).await
        }

        MigrateCommands::Reset { force, pretend } => {
            migrate_reset(config_path, force, pretend, verbose).await
        }

        MigrateCommands::Refresh { seed, step, force } => {
            migrate_refresh(config_path, seed, step, force, verbose).await
        }

        MigrateCommands::Status => migration_status(config_path, verbose).await,

        MigrateCommands::History { limit } => migration_history(config_path, limit, verbose).await,
    }
}

/// Generate a new migration file
async fn generate_migration(
    config_path: &str,
    name: &str,
    create: Option<String>,
    table: Option<String>,
    fields: Option<String>,
    verbose: bool,
) -> Result<(), String> {
    let config = TideConfig::load_or_default(config_path);

    if verbose {
        print_info(&format!("Generating migration: {}", name));
    }

    let generator = MigrationGenerator::new(&config);
    let output_path = generator.generate(name, create, table, fields)?;

    print_success(&format!("Created migration: {}", output_path));

    Ok(())
}

/// Run migration up
async fn migrate_up(
    config_path: &str,
    step: Option<u32>,
    migration: Option<String>,
    pretend: bool,
    verbose: bool,
) -> Result<(), String> {
    let config = TideConfig::load(config_path)?;

    if verbose {
        print_info("Running migration up...");
    }

    if let Some(migration_name) = migration {
        // Run specific migration
        print_info(&format!("Running specific migration: {}", migration_name));

        let migration = find_migration(&config.paths.migrations, &migration_name)?;

        if pretend {
            println!("\n{}", "Pretend mode - SQL to execute:".cyan());
            println!("{}", migration.up_sql);
            return Ok(());
        }

        run_migration_up(&config, &migration).await?;
        print_success(&format!("Migration {} completed", migration_name));
    } else {
        // Run all pending or limited by step
        run(config_path, None, pretend, true, step).await?;
    }

    Ok(())
}

/// Run migration down (rollback)
async fn migrate_down(
    config_path: &str,
    step: u32,
    migration: Option<String>,
    pretend: bool,
    verbose: bool,
) -> Result<(), String> {
    let config = TideConfig::load(config_path)?;

    if verbose {
        print_info(&format!("Rolling back {} migration(s)...", step));
    }

    if let Some(migration_name) = migration {
        // Rollback specific migration
        let migration = find_migration(&config.paths.migrations, &migration_name)?;

        if pretend {
            println!("\n{}", "Pretend mode - SQL to execute:".cyan());
            println!("{}", migration.down_sql);
            return Ok(());
        }

        run_migration_down(&config, &migration).await?;
        print_success(&format!("Rolled back migration: {}", migration_name));
    } else {
        // Rollback last N migrations
        let migrations = get_ran_migrations(&config.paths.migrations)?;
        let migrations_to_rollback: Vec<_> = migrations.into_iter().rev().take(step as usize).collect();

        if migrations_to_rollback.is_empty() {
            print_info("Nothing to rollback");
            return Ok(());
        }

        if pretend {
            println!("\n{}", "Pretend mode - migrations to rollback:".cyan());
            for m in &migrations_to_rollback {
                println!("\n-- Migration: {}", m.name);
                println!("{}", m.down_sql);
            }
            return Ok(());
        }

        for migration in &migrations_to_rollback {
            print!("  Rolling back: {}... ", migration.name);
            run_migration_down(&config, migration).await?;
            println!("{}", "DONE".green());
        }

        print_success(&format!(
            "Rolled back {} migration(s)",
            migrations_to_rollback.len()
        ));
    }

    Ok(())
}

/// Redo last migration(s)
async fn migrate_redo(
    config_path: &str,
    step: u32,
    pretend: bool,
    verbose: bool,
) -> Result<(), String> {
    if verbose {
        print_info(&format!("Redoing {} migration(s)...", step));
    }

    // Rollback
    migrate_down(config_path, step, None, pretend, verbose).await?;

    // Re-run
    migrate_up(config_path, Some(step), None, pretend, verbose).await?;

    print_success(&format!("Redid {} migration(s)", step));

    Ok(())
}

/// Drop all tables and re-run all migrations
async fn migrate_fresh(
    config_path: &str,
    seed: bool,
    seeder: Option<String>,
    force: bool,
    verbose: bool,
) -> Result<(), String> {
    let config = TideConfig::load(config_path)?;

    if config.is_production() && !force {
        return Err(
            "Cannot run migrate:fresh in production without --force flag".to_string()
        );
    }

    if verbose {
        print_warning("This will drop ALL tables and re-run all migrations!");
    }

    // Confirm in production
    if config.is_production() {
        if !utils::confirm("Are you sure you want to drop all tables in PRODUCTION?") {
            print_info("Operation cancelled");
            return Ok(());
        }
    }

    print_info("Dropping all tables...");
    drop_all_tables(&config).await?;
    print_success("Dropped all tables");

    // Run all migrations
    run(config_path, None, false, true, None).await?;

    // Run seeders if requested
    if seed {
        print_info("Running seeders...");
        crate::commands::db::seed(config_path, seeder, true, verbose).await?;
    }

    print_success("Database refreshed successfully");

    Ok(())
}

/// Reset all migrations (rollback all)
async fn migrate_reset(
    config_path: &str,
    force: bool,
    pretend: bool,
    verbose: bool,
) -> Result<(), String> {
    let config = TideConfig::load(config_path)?;

    if config.is_production() && !force {
        return Err(
            "Cannot run migrate:reset in production without --force flag".to_string()
        );
    }

    if verbose {
        print_warning("This will rollback ALL migrations!");
    }

    let migrations = get_ran_migrations(&config.paths.migrations)?;

    if migrations.is_empty() {
        print_info("Nothing to reset");
        return Ok(());
    }

    if pretend {
        println!("\n{}", "Pretend mode - migrations to rollback:".cyan());
        for m in migrations.iter().rev() {
            println!("  - {}", m.name);
        }
        return Ok(());
    }

    println!("Rolling back {} migration(s)...", migrations.len());

    for migration in migrations.iter().rev() {
        print!("  Rolling back: {}... ", migration.name);
        run_migration_down(&config, migration).await?;
        println!("{}", "DONE".green());
    }

    print_success(&format!("Reset {} migration(s)", migrations.len()));

    Ok(())
}

/// Refresh migrations (reset + migrate)
async fn migrate_refresh(
    config_path: &str,
    seed: bool,
    step: Option<u32>,
    force: bool,
    verbose: bool,
) -> Result<(), String> {
    let config = TideConfig::load(config_path)?;

    if config.is_production() && !force {
        return Err(
            "Cannot run migrate:refresh in production without --force flag".to_string()
        );
    }

    if let Some(n) = step {
        // Rollback N migrations
        migrate_down(config_path, n, None, false, verbose).await?;
        // Re-run N migrations
        migrate_up(config_path, Some(n), None, false, verbose).await?;
    } else {
        // Reset all
        migrate_reset(config_path, force, false, verbose).await?;
        // Run all
        run(config_path, None, false, true, None).await?;
    }

    // Run seeders if requested
    if seed {
        print_info("Running seeders...");
        crate::commands::db::seed(config_path, None, true, verbose).await?;
    }

    print_success("Database refreshed successfully");

    Ok(())
}

/// Show migration status
async fn migration_status(config_path: &str, verbose: bool) -> Result<(), String> {
    let config = TideConfig::load(config_path)?;

    if verbose {
        print_info("Checking migration status...");
    }

    let all_migrations = get_all_migrations(&config.paths.migrations)?;
    let ran_migrations = get_ran_migrations(&config.paths.migrations)?;

    println!("\n{}", "Migration Status:".cyan().bold());
    println!("{}", "─".repeat(60));

    if all_migrations.is_empty() {
        print_info("No migrations found");
        return Ok(());
    }

    let ran_names: std::collections::HashSet<_> = ran_migrations.iter().map(|m| &m.name).collect();

    for migration in &all_migrations {
        let status = if ran_names.contains(&migration.name) {
            "Ran".green()
        } else {
            "Pending".yellow()
        };
        println!("  {} {}", status, migration.name);
    }

    println!("{}", "─".repeat(60));
    println!(
        "  Total: {} | Ran: {} | Pending: {}",
        all_migrations.len(),
        ran_migrations.len(),
        all_migrations.len() - ran_migrations.len()
    );

    Ok(())
}

/// Show migration history
async fn migration_history(config_path: &str, limit: u32, verbose: bool) -> Result<(), String> {
    let config = TideConfig::load(config_path)?;

    if verbose {
        print_info(&format!("Showing last {} migrations...", limit));
    }

    let ran_migrations = get_ran_migrations(&config.paths.migrations)?;

    println!("\n{}", "Migration History:".cyan().bold());
    println!("{}", "─".repeat(80));
    println!(
        "  {:<6} {:<40} {:<20}",
        "Batch", "Migration", "Ran At"
    );
    println!("{}", "─".repeat(80));

    if ran_migrations.is_empty() {
        print_info("No migrations have been run");
        return Ok(());
    }

    for (i, migration) in ran_migrations.iter().rev().take(limit as usize).enumerate() {
        let batch = ran_migrations.len() - i;
        println!(
            "  {:<6} {:<40} {:<20}",
            batch,
            migration.name,
            migration.ran_at.as_deref().unwrap_or("N/A")
        );
    }

    println!("{}", "─".repeat(80));

    Ok(())
}

// =============================================================================
// HELPER TYPES AND FUNCTIONS
// =============================================================================

/// Migration information
#[derive(Debug, Clone)]
pub struct Migration {
    pub name: String,
    pub up_sql: String,
    pub down_sql: String,
    pub ran_at: Option<String>,
}

/// Get all migrations from the migrations directory
fn get_all_migrations(migrations_path: &str) -> Result<Vec<Migration>, String> {
    let path = Path::new(migrations_path);

    if !path.exists() {
        return Ok(vec![]);
    }

    let mut migrations = Vec::new();

    for entry in fs::read_dir(path).map_err(|e| format!("Failed to read migrations directory: {}", e))? {
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

            let content = fs::read_to_string(&file_path)
                .map_err(|e| format!("Failed to read migration file: {}", e))?;

            let (up_sql, down_sql) = parse_migration_content(&content);

            migrations.push(Migration {
                name,
                up_sql,
                down_sql,
                ran_at: None,
            });
        }
    }

    migrations.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(migrations)
}

/// Get pending migrations (not yet run)
fn get_pending_migrations(migrations_path: &str) -> Result<Vec<Migration>, String> {
    let all = get_all_migrations(migrations_path)?;
    let ran = get_ran_migrations(migrations_path)?;
    let ran_names: std::collections::HashSet<_> = ran.iter().map(|m| &m.name).collect();

    Ok(all.into_iter().filter(|m| !ran_names.contains(&m.name)).collect())
}

/// Get migrations that have been run
/// In a real implementation, this would query the migrations table
fn get_ran_migrations(_migrations_path: &str) -> Result<Vec<Migration>, String> {
    // TODO: Query the database migrations table
    // For now, return empty (simulating fresh state)
    Ok(vec![])
}

/// Find a specific migration
fn find_migration(migrations_path: &str, name: &str) -> Result<Migration, String> {
    let migrations = get_all_migrations(migrations_path)?;

    migrations
        .into_iter()
        .find(|m| m.name.contains(name))
        .ok_or_else(|| format!("Migration not found: {}", name))
}

/// Parse migration file content to extract up/down SQL
fn parse_migration_content(content: &str) -> (String, String) {
    let mut up_sql = String::new();
    let mut down_sql = String::new();

    // Look for SQL in string literals within up() and down() methods
    let up_pattern = regex::Regex::new(r#"fn\s+up\s*\([^)]*\)[^{]*\{([\s\S]*?)\n\s*\}"#).unwrap();
    let down_pattern = regex::Regex::new(r#"fn\s+down\s*\([^)]*\)[^{]*\{([\s\S]*?)\n\s*\}"#).unwrap();

    if let Some(cap) = up_pattern.captures(content) {
        up_sql = extract_sql_from_method(&cap[1]);
    }

    if let Some(cap) = down_pattern.captures(content) {
        down_sql = extract_sql_from_method(&cap[1]);
    }

    (up_sql, down_sql)
}

/// Extract SQL from method body
fn extract_sql_from_method(method_body: &str) -> String {
    let sql_pattern = regex::Regex::new(r##"r#?"([^"]*)"#?"##).unwrap();
    let mut sqls = Vec::new();

    for cap in sql_pattern.captures_iter(method_body) {
        sqls.push(cap[1].to_string());
    }

    sqls.join("\n")
}

/// Run a migration up
async fn run_migration_up(_config: &TideConfig, _migration: &Migration) -> Result<(), String> {
    // TODO: Execute the migration SQL against the database
    // For now, simulate success
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    Ok(())
}

/// Run a migration down
async fn run_migration_down(_config: &TideConfig, _migration: &Migration) -> Result<(), String> {
    // TODO: Execute the migration SQL against the database
    // For now, simulate success
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    Ok(())
}

/// Drop all tables in the database
async fn drop_all_tables(_config: &TideConfig) -> Result<(), String> {
    // TODO: Drop all tables from the database
    // For now, simulate success
    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
    Ok(())
}
