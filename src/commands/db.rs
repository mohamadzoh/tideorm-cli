//! Database commands for TideORM CLI

use crate::DbCommands;
use crate::commands::migrate::MigrateRunner;
use crate::commands::runner::{Role, Runner};
use crate::config::TideConfig;
use crate::runtime_db;
use crate::utils::{confirm_destructive, print_info, print_success, print_warning, to_snake_case};
use colored::Colorize;
use std::path::Path;

impl Runner {
    /// The registered seed `name` refers to: `name` snake-cased, or with
    /// `_seeder` added, which is how `make seeder` names what it generates
    /// (`User` finds `user_seeder`). The runner lists its seeds without
    /// connecting, so a name that matches none is refused before any table is
    /// touched.
    pub(crate) fn resolve_seed(&self, name: &str) -> Result<String, String> {
        let output = self
            .command()
            .arg("--list")
            .output()
            .map_err(|error| format!("Failed to run cargo: {}", error))?;
        if !output.status.success() {
            return Err(format!(
                "The seeder runner could not list its seeders ({}): {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        let listed = String::from_utf8_lossy(&output.stdout);
        let seeds: Vec<&str> = listed
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect();
        pick_seed(&seeds, name)
    }
}

/// The seed of `seeds` that `name` refers to; see [`Runner::resolve_seed`].
fn pick_seed(seeds: &[&str], name: &str) -> Result<String, String> {
    let snake = to_snake_case(name);
    let suffixed = format!("{snake}_seeder");
    [snake, suffixed]
        .into_iter()
        .find(|candidate| seeds.contains(&candidate.as_str()))
        .ok_or_else(|| {
            if seeds.is_empty() {
                format!("No seeder named `{name}`: seeders::all() registers none")
            } else {
                format!(
                    "No seeder named `{name}`; the project has: {}",
                    seeds.join(", ")
                )
            }
        })
}

/// Handle database subcommands
pub async fn handle(config_path: &str, cmd: DbCommands, verbose: bool) -> Result<(), String> {
    match cmd {
        DbCommands::Seed { seeder, force } => seed(config_path, seeder, force, verbose).await,
        DbCommands::Fresh { force } => fresh(config_path, force, verbose).await,
        DbCommands::Status => status(config_path, verbose).await,
        DbCommands::Check => check(config_path, verbose).await,
        DbCommands::Create { name } => create_database(config_path, name, verbose).await,
        DbCommands::Drop { name, force } => drop_database(config_path, name, force, verbose).await,
        DbCommands::Wipe { drop_types, force } => {
            wipe(config_path, drop_types, force, verbose).await
        }
        DbCommands::Table { name } => show_table(config_path, &name, verbose).await,
        DbCommands::Tables => list_tables(config_path, verbose).await,
    }
}

/// Run database seeders
///
/// Without `seeder`, every seeder that has not run yet runs; with it, that one
/// seeder runs again. The project's runner binary does the work, against the
/// database this CLI resolved.
pub async fn seed(
    config_path: &str,
    seeder: Option<String>,
    force: bool,
    verbose: bool,
) -> Result<(), String> {
    let config = TideConfig::load(config_path)?;

    config.guard_production(force, "run seeders")?;

    let runner = Runner::build(config_path, Role::Seed)?;
    let seed = seeder
        .as_deref()
        .map(|name| runner.resolve_seed(name))
        .transpose()?;

    let mut command = runner.command();
    command.env("DATABASE_URL", config.database.try_connection_url()?);
    if let Some(seed) = &seed {
        command.arg(seed);
    }

    if verbose {
        print_info(&format!(
            "Running the `{}` binary of {}",
            runner.bin,
            runner.manifest.display()
        ));
    }

    println!("\n{}", "Running seeders:".cyan().bold());
    println!("{}", "─".repeat(50));
    let status = command
        .status()
        .map_err(|error| format!("Failed to run cargo: {}", error))?;
    println!("{}", "─".repeat(50));

    if !status.success() {
        return Err(format!("The seeder runner failed ({})", status));
    }

    print_success("Seeding complete");
    Ok(())
}

/// Drop all tables and re-seed
async fn fresh(config_path: &str, force: bool, verbose: bool) -> Result<(), String> {
    let config = TideConfig::load(config_path)?;

    config.guard_production(force, "run db:fresh")?;

    // db:fresh always re-seeds, so a project that cannot seed is refused here
    // instead of dropping every table and only then aborting on the seeding
    // step, which would leave it with an empty, unseeded database.
    Runner::build(config_path, Role::Seed).map_err(|reason| {
        format!(
            "{} Run `tideorm migrate fresh --force` if you only need to drop tables and re-run migrations.",
            reason
        )
    })?;

    if verbose {
        print_warning("This will drop all tables and re-run migrations and seeders!");
    }

    // Use migrate:fresh with --seed. `force` is forwarded as given: hard coding
    // `true` here would skip migrate:fresh's own confirmation for a `db fresh`
    // that was never forced.
    crate::commands::migrate::handle_subcommand(
        config_path,
        crate::MigrateCommands::Fresh {
            seed: true,
            seeder: None,
            force,
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
        "sqlite" => match config.database.sqlite_file() {
            Some(path) => {
                println!("  Path:       {}", path);
                let exists = Path::new(&path).exists();
                println!(
                    "  Status:     {}",
                    if exists {
                        "EXISTS".green()
                    } else {
                        "NOT FOUND".yellow()
                    }
                );
            }
            None => println!("  Path:       {}", "in memory".green()),
        },
        _ => {
            println!("  Host:       {}", config.database.host);
            println!(
                "  Port:       {}",
                config
                    .database
                    .port
                    .map_or("default".to_string(), |p| p.to_string())
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
    let connection_result = test_connection(&config).await;
    match &connection_result {
        Ok(()) => println!("{}", "OK".green()),
        Err(error) => println!("{} ({})", "FAILED".red(), error),
    }

    println!("{}", "─".repeat(50));

    connection_result
}

/// Create the migrations ledger through the project's migration runner,
/// whose `Migrator` creates it when it reads the migrations' status. The
/// seed ledger is created when seeders first run.
async fn check(config_path: &str, verbose: bool) -> Result<(), String> {
    let config = TideConfig::load(config_path)?;

    if verbose {
        print_info("Creating the migrations ledger...");
    }

    let registered = MigrateRunner::build(config_path, &config)?.status()?.len();

    println!("\n{}", "Database Metadata:".cyan().bold());
    println!("{}", "─".repeat(50));
    println!("  Migration table: {}", config.migration.table.green());
    println!("  Migrations:      {} registered", registered);
    println!("{}", "─".repeat(50));

    print_success("The migrations ledger is ready");
    Ok(())
}

/// Create a database
async fn create_database(
    config_path: &str,
    name: Option<String>,
    verbose: bool,
) -> Result<(), String> {
    let config = TideConfig::load(config_path)?;

    let db_name = resolve_target_database(&config, name.as_deref())?;

    if verbose {
        print_info(&format!("Creating database: {}", db_name));
    }

    // Create the database
    create_db(&config, &db_name).await?;

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

    config.guard_production(force, "drop database")?;

    let db_name = resolve_target_database(&config, name.as_deref())?;

    if !force
        && !confirm_destructive(&format!(
            "Are you sure you want to drop database '{}' ?",
            db_name
        ))?
    {
        print_info("Operation cancelled");
        return Ok(());
    }

    if verbose {
        print_info(&format!("Dropping database: {}", db_name));
    }

    // Drop the database
    drop_db(&config, &db_name).await?;

    print_success(&format!("Database '{}' dropped successfully", db_name));

    Ok(())
}

/// Wipe all tables by dropping them
///
/// This drops every table and view, schema included - it is not a `TRUNCATE`. The drop
/// semantics are load bearing: `migrate fresh` goes through the same
/// `runtime_db::wipe_tables` and relies on them to rebuild from the migrations.
async fn wipe(
    config_path: &str,
    drop_types: bool,
    force: bool,
    verbose: bool,
) -> Result<(), String> {
    let config = TideConfig::load(config_path)?;

    config.guard_production(force, "wipe database")?;

    if drop_types && !drop_types_supported(&config) {
        print_warning(&format!(
            "--drop-types only applies to PostgreSQL; ignoring it for driver '{}'",
            config.database.driver
        ));
    }

    if !force
        && !confirm_destructive("Are you sure you want to drop all tables (schema included)?")?
    {
        print_info("Operation cancelled");
        return Ok(());
    }

    if verbose {
        print_info("Dropping all tables...");
    }

    wipe_tables(&config, drop_types).await?;

    print_success("All tables dropped successfully");

    Ok(())
}

/// Show table information
async fn show_table(config_path: &str, table_name: &str, verbose: bool) -> Result<(), String> {
    let config = TideConfig::load(config_path)?;

    if verbose {
        print_info(&format!("Getting info for table: {}", table_name));
    }

    let db = runtime_db::connect(&config).await?;
    let columns = runtime_db::table_columns(&db, table_name).await?;

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
            runtime_db::column_key(col),
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

    let db = runtime_db::connect(&config).await?;
    let tables = runtime_db::list_tables(&db).await?;

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

/// Resolve the database a `db create` / `db drop` invocation should act on.
///
/// For SQLite the database *is* a file, so an explicitly supplied name is the
/// target file path (relative names resolve against the current directory) and
/// the file the connection opens is only the fallback for when no name was
/// given: the connection URL's file when one is set, `sqlite_path` otherwise.
/// Other drivers fall back to the configured database name.
fn resolve_target_database(config: &TideConfig, name: Option<&str>) -> Result<String, String> {
    let explicit = match name {
        Some(value) if value.trim().is_empty() => {
            return Err("Explicit database name or SQLite path must not be empty".to_string());
        }
        Some(value) => Some(value.trim()),
        None => None,
    };

    if config.database.driver_name() == "sqlite" {
        return match explicit {
            Some(path) => Ok(path.to_string()),
            None => config.database.sqlite_file().ok_or_else(|| {
                "The configured SQLite database is in memory, so it has no file".to_string()
            }),
        };
    }

    explicit
        .or(config.database.database.as_deref())
        .map(str::to_string)
        .ok_or_else(|| "Database name not specified".to_string())
}

/// Whether `--drop-types` does anything for the configured driver.
///
/// Only PostgreSQL has user-defined types for `db wipe` to drop; on the other
/// backends the flag is inert, so it is reported rather than silently ignored.
fn drop_types_supported(config: &TideConfig) -> bool {
    config.database.driver_name() == "postgres"
}

/// Test database connection
async fn test_connection(config: &TideConfig) -> Result<(), String> {
    runtime_db::ping(config).await
}

/// Create a database
async fn create_db(config: &TideConfig, name: &str) -> Result<(), String> {
    runtime_db::create_database(config, name).await
}

/// Drop a database
async fn drop_db(config: &TideConfig, name: &str) -> Result<(), String> {
    runtime_db::drop_database(config, name).await
}

/// Drop every table and view in the database (and, on PostgreSQL, the enum types)
async fn wipe_tables(config: &TideConfig, drop_types: bool) -> Result<(), String> {
    runtime_db::wipe_tables(config, drop_types).await
}

#[cfg(test)]
#[path = "../../tests/unit/commands_db_tests.rs"]
mod tests;
