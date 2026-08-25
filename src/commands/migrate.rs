//! Migration commands for TideORM CLI

use crate::MigrateCommands;
use crate::config::TideConfig;
use crate::generators::migration::MigrationGenerator;
use crate::runtime_db;
use crate::utils::{self, print_info, print_success, print_warning};
use colored::Colorize;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use tideorm::internal::{ConnectionTrait, OrmStatement as Statement};

/// Run pending migrations
pub async fn run(
    config_path: &str,
    path: Option<String>,
    pretend: bool,
    force: bool,
    step: Option<u32>,
) -> Result<(), String> {
    let config = TideConfig::load(config_path)?;

    if config.is_production() && !force {
        return Err("Cannot run migrations in production without --force flag".to_string());
    }

    let migrations_path = path.as_deref().unwrap_or(&config.paths.migrations);

    print_info(&format!("Running migrations from: {}", migrations_path));

    // Pretend mode must stay side effect free: it never opens a connection and
    // never creates the migrations table, which also means already applied
    // migrations cannot be filtered out here.
    let migrations = if pretend {
        print_warning("Running in pretend mode - no changes will be made");
        print_warning(
            "Pretend mode does not contact the database, so migrations that already ran are still listed",
        );
        get_all_migrations(migrations_path)?
    } else {
        get_pending_migrations(&config, migrations_path).await?
    };

    if migrations.is_empty() {
        print_success("Nothing to migrate");
        return Ok(());
    }

    let migrations_to_run: Vec<_> = match step {
        Some(n) => migrations.into_iter().take(n as usize).collect(),
        None => migrations,
    };

    println!(
        "\n{} Migrations to run:",
        format!("[{}]", migrations_to_run.len()).cyan()
    );

    for (index, migration) in migrations_to_run.iter().enumerate() {
        println!("  {}. {}", index + 1, migration.file_name.yellow());
    }

    if pretend {
        println!("\n{}", "Pretend mode - showing SQL:".cyan());
        for migration in &migrations_to_run {
            println!("\n-- Migration: {}", migration.file_name);
            println!("-- Up:");
            println!("{}", migration.up_sql());
        }
        return Ok(());
    }

    println!("\n{}", "Running migrations...".cyan());

    for migration in &migrations_to_run {
        print!("  Migrating: {}... ", migration.file_name);

        match run_migration_up(&config, migration).await {
            Ok(()) => println!("{}", "DONE".green()),
            Err(error) => {
                println!("{}", "FAILED".red());
                return Err(format!("Migration failed: {}", error));
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
            force,
        } => migrate_up(config_path, step, migration, pretend, force, verbose).await,
        MigrateCommands::Down {
            step,
            migration,
            pretend,
            force,
        } => migrate_down(config_path, step, migration, pretend, force, verbose).await,
        MigrateCommands::Redo {
            step,
            pretend,
            force,
        } => migrate_redo(config_path, step, pretend, force, verbose).await,
        MigrateCommands::Fresh {
            seed,
            seeder,
            force,
        } => migrate_fresh(config_path, seed, seeder, force, verbose).await,
        MigrateCommands::Reset { force, pretend } => {
            migrate_reset(config_path, force, pretend, verbose).await
        }
        MigrateCommands::Refresh { seed, step, force } => {
            migrate_refresh(config_path, seed, step, force, verbose).await
        }
        MigrateCommands::Mark {
            migration,
            unmark,
            force,
        } => migrate_mark(config_path, &migration, unmark, force, verbose).await,
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
    // A broken `tideorm.toml` must not be swallowed: the generated DDL is driver
    // specific, so falling back to the built-in Postgres defaults would write a
    // migration for the wrong backend into the wrong directory. A file that is
    // simply absent still defaults, since generating needs no project.
    let config = TideConfig::load_or_default(config_path)?;

    if verbose {
        print_info(&format!("Generating migration: {}", name));
    }

    let generator = MigrationGenerator::new(&config);
    let output_path = generator.generate(name, create, table, fields, false, false)?;

    print_success(&format!("Created migration: {}", output_path));

    Ok(())
}

/// Run migration up
async fn migrate_up(
    config_path: &str,
    step: Option<u32>,
    migration: Option<String>,
    pretend: bool,
    force: bool,
    verbose: bool,
) -> Result<(), String> {
    let config = TideConfig::load(config_path)?;

    if config.is_production() && !force {
        return Err("Cannot run migrate:up in production without --force flag".to_string());
    }

    if verbose {
        print_info("Running migration up...");
    }

    if let Some(migration_name) = migration {
        print_info(&format!("Running specific migration: {}", migration_name));

        let migration = find_migration(&config.paths.migrations, &migration_name)?;

        if pretend {
            println!("\n{}", "Pretend mode - SQL to execute:".cyan());
            println!("{}", migration.up_sql());
            return Ok(());
        }

        let ran_migrations = get_ran_migrations(&config, &config.paths.migrations).await?;
        if ran_migrations
            .iter()
            .any(|ran| ran.version == migration.version)
        {
            return Err(format!("Migration already ran: {}", migration.file_name));
        }

        run_migration_up(&config, &migration).await?;
        print_success(&format!("Migration {} completed", migration_name));
    } else {
        run(config_path, None, pretend, force, step).await?;
    }

    Ok(())
}

/// Run migration down (rollback)
async fn migrate_down(
    config_path: &str,
    step: u32,
    migration: Option<String>,
    pretend: bool,
    force: bool,
    verbose: bool,
) -> Result<(), String> {
    let config = TideConfig::load(config_path)?;

    if config.is_production() && !force {
        return Err("Cannot run migrate:down in production without --force flag".to_string());
    }

    if verbose {
        print_info(&format!("Rolling back {} migration(s)...", step));
    }

    if let Some(migration_name) = migration {
        let migration = find_migration(&config.paths.migrations, &migration_name)?;

        if pretend {
            println!("\n{}", "Pretend mode - SQL to execute:".cyan());
            println!("{}", migration.down_sql());
            return Ok(());
        }

        let ran_migrations = get_ran_migrations(&config, &config.paths.migrations).await?;
        if !ran_migrations
            .iter()
            .any(|ran| ran.version == migration.version)
        {
            return Err(format!(
                "Migration has not been run: {}",
                migration.file_name
            ));
        }

        run_migration_down(&config, &migration).await?;
        print_success(&format!("Rolled back migration: {}", migration_name));
    } else {
        // `get_ran_migrations` is ordered by application order, so reversing it
        // yields the most recently applied migrations first.
        let migrations = get_ran_migrations(&config, &config.paths.migrations).await?;
        let migrations_to_rollback: Vec<_> =
            migrations.into_iter().rev().take(step as usize).collect();

        if migrations_to_rollback.is_empty() {
            print_info("Nothing to rollback");
            return Ok(());
        }

        if pretend {
            println!("\n{}", "Pretend mode - migrations to rollback:".cyan());
            for migration in &migrations_to_rollback {
                println!("\n-- Migration: {}", migration.file_name);
                println!("{}", migration.down_sql());
            }
            return Ok(());
        }

        for migration in &migrations_to_rollback {
            print!("  Rolling back: {}... ", migration.file_name);
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
    force: bool,
    verbose: bool,
) -> Result<(), String> {
    let config = TideConfig::load(config_path)?;

    if config.is_production() && !force {
        return Err("Cannot run migrate:redo in production without --force flag".to_string());
    }

    if verbose {
        print_info(&format!("Redoing {} migration(s)...", step));
    }

    migrate_down(config_path, step, None, pretend, force, verbose).await?;
    migrate_up(config_path, Some(step), None, pretend, force, verbose).await?;

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
        return Err("Cannot run migrate:fresh in production without --force flag".to_string());
    }

    if verbose {
        print_warning("This will drop ALL tables and re-run all migrations!");
    }

    // Seeding cannot actually run yet, so refuse before dropping anything rather
    // than wiping the database and only then failing on the seed step.
    if seed {
        crate::commands::db::ensure_seeding_supported().map_err(|reason| {
            format!(
                "{reason}\nRun `tideorm migrate fresh` without --seed if you only need to drop tables and re-run migrations."
            )
        })?;
    }

    // Validate the migration set BEFORE dropping anything: a wrong working
    // directory or a stale paths.migrations must not wipe the database and then
    // report "Nothing to migrate".
    let migrations = get_all_migrations(&config.paths.migrations)?;

    if migrations.is_empty() {
        return Err(format!(
            "No migrations found in '{}' - refusing to drop all tables. Check that you are in the project root and that paths.migrations is correct.",
            config.paths.migrations
        ));
    }

    if let Some(invalid) = migrations
        .iter()
        .find(|migration| executable_statements(&migration.up_statements).is_empty())
    {
        return Err(format!(
            "Migration {} does not contain executable SQL in up() - refusing to drop all tables",
            invalid.file_name
        ));
    }

    // `--force` is the non-interactive escape hatch, so it must SKIP this prompt
    // rather than be a precondition for reaching it. Without `--force` the drop
    // is confirmed interactively, and a run that cannot prompt fails instead of
    // reporting a successful no-op.
    if !force && !utils::confirm_destructive("Are you sure you want to drop all tables?")? {
        print_info("Operation cancelled");
        return Ok(());
    }

    print_info("Dropping all tables...");
    drop_all_tables(&config).await?;
    print_success("Dropped all tables");

    run(config_path, None, false, true, None).await?;

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
        return Err("Cannot run migrate:reset in production without --force flag".to_string());
    }

    if verbose {
        print_warning("This will rollback ALL migrations!");
    }

    let migrations = get_ran_migrations(&config, &config.paths.migrations).await?;

    if migrations.is_empty() {
        print_info("Nothing to reset");
        return Ok(());
    }

    if pretend {
        println!("\n{}", "Pretend mode - migrations to rollback:".cyan());
        for migration in migrations.iter().rev() {
            println!("  - {}", migration.file_name);
        }
        return Ok(());
    }

    println!("Rolling back {} migration(s)...", migrations.len());

    for migration in migrations.iter().rev() {
        print!("  Rolling back: {}... ", migration.file_name);
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
        return Err("Cannot run migrate:refresh in production without --force flag".to_string());
    }

    if let Some(count) = step {
        migrate_down(config_path, count, None, false, force, verbose).await?;
        migrate_up(config_path, Some(count), None, false, force, verbose).await?;
    } else {
        migrate_reset(config_path, force, false, verbose).await?;
        run(config_path, None, false, true, None).await?;
    }

    if seed {
        print_info("Running seeders...");
        crate::commands::db::seed(config_path, None, true, verbose).await?;
    }

    print_success("Database refreshed successfully");

    Ok(())
}

/// Record a migration as applied (or not applied) without running its SQL
///
/// MySQL and MariaDB commit DDL implicitly, so an apply whose ledger write fails
/// leaves the schema changed but unrecorded: every later run then hard-fails on
/// "table already exists" with nothing to reconcile it. Marking the migration as
/// applied is that escape hatch, and `--unmark` covers the mirror case where a
/// rollback dropped the schema but could not delete the ledger row.
async fn migrate_mark(
    config_path: &str,
    migration_name: &str,
    unmark: bool,
    force: bool,
    verbose: bool,
) -> Result<(), String> {
    let config = TideConfig::load(config_path)?;

    if config.is_production() && !force {
        return Err("Cannot run migrate:mark in production without --force flag".to_string());
    }

    let migration = find_migration(&config.paths.migrations, migration_name)?;

    if verbose {
        print_info(&format!(
            "Updating the migration ledger for: {}",
            migration.file_name
        ));
    }

    let already_applied = get_ran_migrations(&config, &config.paths.migrations)
        .await?
        .iter()
        .any(|ran| ran.version == migration.version);

    let (sql, message) = if unmark {
        if !already_applied {
            return Err(format!(
                "Migration is not recorded as applied: {}",
                migration.file_name
            ));
        }

        (
            delete_record_sql(&config, &migration),
            format!("Marked {} as not applied", migration.file_name),
        )
    } else {
        if already_applied {
            return Err(format!(
                "Migration is already recorded as applied: {}",
                migration.file_name
            ));
        }

        (
            insert_record_sql(&config, &migration),
            format!("Marked {} as applied", migration.file_name),
        )
    };

    let db = runtime_db::connect(&config).await?;
    runtime_db::ensure_migration_table_on_db(&db, &config, &config.migration.table).await?;
    runtime_db::execute_on_db(&db, &sql).await?;

    print_warning("No migration SQL was executed - only the migration ledger was changed");
    print_success(&message);

    Ok(())
}

/// Show migration status
async fn migration_status(config_path: &str, verbose: bool) -> Result<(), String> {
    let config = TideConfig::load(config_path)?;

    if verbose {
        print_info("Checking migration status...");
    }

    let all_migrations = get_all_migrations(&config.paths.migrations)?;
    let ran_migrations = get_ran_migrations(&config, &config.paths.migrations).await?;

    println!("\n{}", "Migration Status:".cyan().bold());
    println!("{}", "─".repeat(60));

    if all_migrations.is_empty() {
        print_info("No migrations found");
        return Ok(());
    }

    let ran_names: HashSet<_> = ran_migrations
        .iter()
        .map(|migration| migration.version.as_str())
        .collect();

    for migration in &all_migrations {
        let status = if ran_names.contains(migration.version.as_str()) {
            "Ran".green()
        } else {
            "Pending".yellow()
        };
        println!("  {} {}", status, migration.file_name);
    }

    println!("{}", "─".repeat(60));
    println!(
        "  Total: {} | Ran: {} | Pending: {}",
        all_migrations.len(),
        ran_migrations.len(),
        all_migrations.len().saturating_sub(ran_migrations.len())
    );

    Ok(())
}

/// Show migration history
async fn migration_history(config_path: &str, limit: u32, verbose: bool) -> Result<(), String> {
    let config = TideConfig::load(config_path)?;

    if verbose {
        print_info(&format!("Showing last {} migrations...", limit));
    }

    let ran_migrations = get_ran_migrations(&config, &config.paths.migrations).await?;

    println!("\n{}", "Migration History:".cyan().bold());
    println!("{}", "─".repeat(80));
    println!(
        "  {:<16} {:<40} {:<20}",
        "Version", "Migration", "Applied At"
    );
    println!("{}", "─".repeat(80));

    if ran_migrations.is_empty() {
        print_info("No migrations have been run");
        return Ok(());
    }

    for migration in ran_migrations.iter().rev().take(limit as usize) {
        println!(
            "  {:<16} {:<40} {:<20}",
            migration.version,
            migration.file_name,
            migration.applied_at.as_deref().unwrap_or("N/A")
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
    pub file_name: String,
    pub version: String,
    pub name: String,
    /// One entry per SQL statement found in `up()`, in source order.
    pub up_statements: Vec<String>,
    /// One entry per SQL statement found in `down()`, in source order.
    pub down_statements: Vec<String>,
    pub applied_at: Option<String>,
}

impl Migration {
    /// Rendering of the `up()` statements for display purposes only.
    pub fn up_sql(&self) -> String {
        render_statements(&self.up_statements)
    }

    /// Rendering of the `down()` statements for display purposes only.
    pub fn down_sql(&self) -> String {
        render_statements(&self.down_statements)
    }
}

/// Statements that are actually worth sending to the server, trimmed and
/// with empty fragments removed.
fn executable_statements(statements: &[String]) -> Vec<String> {
    statements
        .iter()
        .map(|statement| statement.trim().to_string())
        .filter(|statement| !statement.is_empty())
        .collect()
}

/// Join statements for display, terminating each one so the output is valid SQL.
fn render_statements(statements: &[String]) -> String {
    executable_statements(statements)
        .into_iter()
        .map(|statement| {
            if statement.ends_with(';') {
                statement
            } else {
                format!("{};", statement)
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Get all migrations from the migrations directory
fn get_all_migrations(migrations_path: &str) -> Result<Vec<Migration>, String> {
    let path = Path::new(migrations_path);

    if !path.exists() {
        return Ok(vec![]);
    }

    let mut migrations = Vec::new();

    for entry in fs::read_dir(path)
        .map_err(|error| format!("Failed to read migrations directory: {}", error))?
    {
        let entry = entry.map_err(|error| format!("Failed to read entry: {}", error))?;
        let file_path = entry.path();

        if file_path.extension().is_some_and(|ext| ext == "rs") {
            let name = file_path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("")
                .to_string();

            if name == "mod" {
                continue;
            }

            let content = fs::read_to_string(&file_path)
                .map_err(|error| format!("Failed to read migration file: {}", error))?;

            let (version, logical_name) = parse_migration_metadata(&name, &content);
            let (up_statements, down_statements) = parse_migration_content(&content);

            migrations.push(Migration {
                file_name: name,
                version,
                name: logical_name,
                up_statements,
                down_statements,
                applied_at: None,
            });
        }
    }

    migrations.sort_by(|left, right| left.version.cmp(&right.version));

    Ok(migrations)
}

/// Get pending migrations (not yet run)
async fn get_pending_migrations(
    config: &TideConfig,
    migrations_path: &str,
) -> Result<Vec<Migration>, String> {
    let all = get_all_migrations(migrations_path)?;
    let ran = get_ran_migrations(config, migrations_path).await?;
    let ran_versions: HashSet<_> = ran
        .iter()
        .map(|migration| migration.version.as_str())
        .collect();

    Ok(all
        .into_iter()
        .filter(|migration| !ran_versions.contains(migration.version.as_str()))
        .collect())
}

/// Get migrations that have been run, oldest applied first
async fn get_ran_migrations(
    config: &TideConfig,
    migrations_path: &str,
) -> Result<Vec<Migration>, String> {
    runtime_db::ensure_migration_table(config, &config.migration.table).await?;
    let db = runtime_db::connect(config).await?;

    let all_migrations = get_all_migrations(migrations_path)?;
    let all_by_name: HashMap<_, _> = all_migrations
        .into_iter()
        .map(|migration| (migration.version.clone(), migration))
        .collect();

    let connection = db
        .__internal_connection()
        .map_err(|error| error.to_string())?;
    let backend = connection.get_database_backend();
    let statement = Statement::from_string(
        backend,
        migration_records_query(config, &config.migration.table),
    );
    let rows = connection
        .query_all_raw(statement)
        .await
        .map_err(|error| error.to_string())?;

    let mut migrations = Vec::with_capacity(rows.len());

    for row in rows {
        let version = match row.try_get::<String>("", "version") {
            Ok(version) if !version.is_empty() => version,
            _ => continue,
        };

        let name = row
            .try_get::<String>("", "name")
            .ok()
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| version.clone());
        let applied_at = row
            .try_get::<String>("", "applied_at")
            .ok()
            .filter(|value| !value.is_empty());

        let mut migration = all_by_name.get(&version).cloned().unwrap_or(Migration {
            file_name: version.clone(),
            version: version.clone(),
            name,
            up_statements: Vec::new(),
            down_statements: Vec::new(),
            applied_at: None,
        });

        if migration.name.is_empty() {
            migration.name = version.clone();
        }

        migration.applied_at = applied_at;
        migrations.push(migration);
    }

    Ok(migrations)
}

/// Query the migration ledger in the order the migrations were applied.
///
/// Ordering is by the auto-increment `id`, not by `version`: after a long lived
/// branch merges, a migration with an older version string can be applied last,
/// and rolling back by version would then revert the wrong migration. Callers
/// that want newest-first (rollback, history) reverse this list.
fn migration_records_query(config: &TideConfig, table_name: &str) -> String {
    let table = quoted_identifier(config, table_name);
    let id = quoted_identifier(config, "id");
    let version = quoted_identifier(config, "version");
    let name = quoted_identifier(config, "name");
    let applied_at = quoted_identifier(config, "applied_at");
    let applied_at_expr = match config.database.driver.as_str() {
        "mysql" => format!("CAST({} AS CHAR) AS {}", applied_at, applied_at),
        _ => format!("CAST({} AS TEXT) AS {}", applied_at, applied_at),
    };

    format!(
        "SELECT {}, {}, {} FROM {} ORDER BY {} ASC",
        version, name, applied_at_expr, table, id
    )
}

/// Find a specific migration
///
/// An exact file name, version or logical name always wins. Otherwise a
/// substring match is accepted only when it identifies exactly one migration -
/// an ambiguous name is rejected instead of silently picking the first match.
fn find_migration(migrations_path: &str, name: &str) -> Result<Migration, String> {
    let migrations = get_all_migrations(migrations_path)?;

    let mut matches: Vec<Migration> = migrations
        .iter()
        .filter(|migration| {
            migration.file_name == name || migration.version == name || migration.name == name
        })
        .cloned()
        .collect();

    if matches.is_empty() {
        matches = migrations
            .into_iter()
            .filter(|migration| {
                migration.file_name.contains(name)
                    || migration.version.contains(name)
                    || migration.name.contains(name)
            })
            .collect();
    }

    match matches.len() {
        0 => Err(format!("Migration not found: {}", name)),
        1 => Ok(matches.remove(0)),
        _ => {
            let candidates = matches
                .iter()
                .map(|migration| format!("  - {}", migration.file_name))
                .collect::<Vec<_>>()
                .join("\n");

            Err(format!(
                "Migration name is ambiguous: '{}' matches {} migrations:\n{}\nUse the full migration name or its version instead.",
                name,
                matches.len(),
                candidates
            ))
        }
    }
}

fn parse_migration_metadata(file_name: &str, content: &str) -> (String, String) {
    let version_pattern =
        regex::Regex::new(r#"fn\s+version\s*\([^)]*\)\s*->\s*&str\s*\{\s*\"([^\"]+)\""#).unwrap();
    let name_pattern =
        regex::Regex::new(r#"fn\s+name\s*\([^)]*\)\s*->\s*&str\s*\{\s*\"([^\"]+)\""#).unwrap();

    let version = version_pattern
        .captures(content)
        .and_then(|captures| captures.get(1))
        .map(|value| value.as_str().to_string())
        .or_else(|| split_file_name(file_name).map(|(version, _)| version.to_string()))
        .unwrap_or_else(|| file_name.to_string());

    let logical_name = name_pattern
        .captures(content)
        .and_then(|captures| captures.get(1))
        .map(|value| value.as_str().to_string())
        .or_else(|| split_file_name(file_name).map(|(_, name)| name.to_string()))
        .unwrap_or_else(|| file_name.to_string());

    (version, logical_name)
}

fn split_file_name(file_name: &str) -> Option<(&str, &str)> {
    let (version, name) = file_name.split_once('_')?;
    if version.chars().all(|character| character.is_ascii_digit()) {
        Some((version, name))
    } else {
        None
    }
}

/// Parse migration file content to extract the up/down SQL statements
fn parse_migration_content(content: &str) -> (Vec<String>, Vec<String>) {
    let mut up_statements = Vec::new();
    let mut down_statements = Vec::new();

    let up_pattern = regex::Regex::new(r#"fn\s+up\s*\([^)]*\)[^{]*\{([\s\S]*?)\n\s*\}"#).unwrap();
    let down_pattern =
        regex::Regex::new(r#"fn\s+down\s*\([^)]*\)[^{]*\{([\s\S]*?)\n\s*\}"#).unwrap();

    if let Some(captures) = up_pattern.captures(content) {
        up_statements = extract_sql_from_method(&captures[1]);
    }

    if let Some(captures) = down_pattern.captures(content) {
        down_statements = extract_sql_from_method(&captures[1]);
    }

    (up_statements, down_statements)
}

/// Extract the SQL statements from a migration method body
///
/// Statements are read from Rust raw string literals (`r"..."`, `r#"..."#`, and
/// any other hash count), which is what `schema.raw(..)` calls contain. The
/// literals are scanned explicitly rather than matched with a regex so that SQL
/// containing double quoted identifiers is not truncated at the first quote.
fn extract_sql_from_method(method_body: &str) -> Vec<String> {
    let bytes = method_body.as_bytes();
    let mut statements = Vec::new();
    let mut index = 0;

    while index < bytes.len() {
        // Skip line comments so commented out example SQL is never executed.
        if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'/') {
            index = match method_body[index..].find('\n') {
                Some(offset) => index + offset + 1,
                None => break,
            };
            continue;
        }

        if bytes[index] != b'r' || !starts_literal(bytes, index) {
            index += 1;
            continue;
        }

        let mut cursor = index + 1;
        while cursor < bytes.len() && bytes[cursor] == b'#' {
            cursor += 1;
        }

        let hashes = cursor - index - 1;

        if bytes.get(cursor) != Some(&b'"') {
            index += 1;
            continue;
        }

        let content_start = cursor + 1;

        match find_raw_string_end(bytes, content_start, hashes) {
            Some(content_end) => {
                statements.push(method_body[content_start..content_end].to_string());
                index = content_end + 1 + hashes;
            }
            // Unterminated literal: nothing sensible left to scan.
            None => break,
        }
    }

    statements
}

/// Whether the byte at `index` can start a literal, i.e. is not part of an identifier
fn starts_literal(bytes: &[u8], index: usize) -> bool {
    match index.checked_sub(1) {
        Some(previous) => !bytes[previous].is_ascii_alphanumeric() && bytes[previous] != b'_',
        None => true,
    }
}

/// Position of the closing quote of a raw string literal opened with `hashes` hashes
fn find_raw_string_end(bytes: &[u8], start: usize, hashes: usize) -> Option<usize> {
    let mut index = start;

    while index < bytes.len() {
        if bytes[index] == b'"' {
            let tail = bytes.get(index + 1..index + 1 + hashes);
            if tail.is_some_and(|tail| tail.iter().all(|byte| *byte == b'#')) {
                return Some(index);
            }
        }

        index += 1;
    }

    None
}

/// Run a migration up
async fn run_migration_up(config: &TideConfig, migration: &Migration) -> Result<(), String> {
    let up_statements = executable_statements(&migration.up_statements);
    if up_statements.is_empty() {
        return Err(format!(
            "Migration {} does not contain executable SQL in up()",
            migration.file_name
        ));
    }

    let db = runtime_db::connect(config).await?;
    runtime_db::ensure_migration_table_on_db(&db, config, &config.migration.table).await?;
    let insert_sql = insert_record_sql(config, migration);

    db.transaction(|tx| {
        Box::pin(async move {
            // Each statement is executed on its own: concatenating them would
            // produce a single malformed statement.
            for statement in &up_statements {
                execute_on_transaction(tx.connection(), statement).await?;
            }
            execute_on_transaction(tx.connection(), &insert_sql).await?;
            Ok(())
        })
    })
    .await
    .map_err(|error| ledger_failure_message(config, migration, error.to_string()))
}

/// Annotate a failed apply/rollback on backends that cannot roll DDL back.
///
/// MySQL and MariaDB implicitly commit DDL, so the surrounding transaction
/// cannot undo a `CREATE TABLE` whose ledger write failed afterwards. The
/// schema change is then permanent and unrecorded, which every later run trips
/// over, so point at the escape hatch that repairs the ledger.
fn ledger_failure_message(config: &TideConfig, migration: &Migration, error: String) -> String {
    match config.database.driver.as_str() {
        "mysql" | "mariadb" => format!(
            "{}\nMySQL/MariaDB commit DDL implicitly, so the schema change may have been applied without the migration ledger being updated. If so, reconcile it with `tideorm migrate mark --migration {}` (add --unmark for a rollback).",
            error, migration.file_name
        ),
        _ => error,
    }
}

/// SQL that records `migration` in the migration ledger
fn insert_record_sql(config: &TideConfig, migration: &Migration) -> String {
    format!(
        "INSERT INTO {} ({}, {}) VALUES ({}, {})",
        quoted_identifier(config, &config.migration.table),
        quoted_identifier(config, "version"),
        quoted_identifier(config, "name"),
        sql_string(&migration.version),
        sql_string(&migration.name)
    )
}

/// SQL that removes `migration` from the migration ledger
fn delete_record_sql(config: &TideConfig, migration: &Migration) -> String {
    format!(
        "DELETE FROM {} WHERE {} = {}",
        quoted_identifier(config, &config.migration.table),
        quoted_identifier(config, "version"),
        sql_string(&migration.version)
    )
}

/// Run a migration down
async fn run_migration_down(config: &TideConfig, migration: &Migration) -> Result<(), String> {
    let down_statements = executable_statements(&migration.down_statements);
    if down_statements.is_empty() {
        return Err(format!(
            "Migration {} does not contain executable SQL in down()",
            migration.file_name
        ));
    }

    let db = runtime_db::connect(config).await?;
    runtime_db::ensure_migration_table_on_db(&db, config, &config.migration.table).await?;
    let delete_sql = delete_record_sql(config, migration);

    db.transaction(|tx| {
        Box::pin(async move {
            for statement in &down_statements {
                execute_on_transaction(tx.connection(), statement).await?;
            }
            execute_on_transaction(tx.connection(), &delete_sql).await?;
            Ok(())
        })
    })
    .await
    .map_err(|error| ledger_failure_message(config, migration, error.to_string()))
}

/// Drop all tables in the database
async fn drop_all_tables(config: &TideConfig) -> Result<(), String> {
    runtime_db::wipe_tables(config, true).await
}

async fn execute_on_transaction<C>(connection: &C, sql: &str) -> tideorm::Result<()>
where
    C: ConnectionTrait,
{
    connection
        .execute_unprepared(sql)
        .await
        .map(|_| ())
        .map_err(|error| tideorm::Error::query(error.to_string()))
}

fn quoted_identifier(config: &TideConfig, identifier: &str) -> String {
    match config.database.driver.as_str() {
        "mysql" => format!("`{}`", identifier.replace('`', "``")),
        _ => format!("\"{}\"", identifier.replace('"', "\"\"")),
    }
}

fn sql_string(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

#[cfg(test)]
mod tests {
    use super::{extract_sql_from_method, generate_migration, migrate_mark, render_statements};
    use super::{get_pending_migrations, get_ran_migrations, run, run_migration_down};
    use crate::config::TideConfig;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn extracts_every_statement_and_keeps_quoted_identifiers_intact() {
        let body = r####"
        schema.raw(r#"ALTER TABLE "users" ADD COLUMN "a" TEXT"#).await?;
        schema.raw(r#"ALTER TABLE "users" ADD COLUMN "b" TEXT"#).await?;
        // schema.raw(r#"only an example"#).await?;
        Ok(())
    "####;

        assert_eq!(
            extract_sql_from_method(body),
            vec![
                "ALTER TABLE \"users\" ADD COLUMN \"a\" TEXT".to_string(),
                "ALTER TABLE \"users\" ADD COLUMN \"b\" TEXT".to_string(),
            ]
        );
    }

    #[test]
    fn rendered_statements_are_terminated() {
        let statements = vec![
            "  ALTER TABLE users ADD COLUMN a TEXT  ".to_string(),
            "   ".to_string(),
            "ALTER TABLE users ADD COLUMN b TEXT;".to_string(),
        ];

        assert_eq!(
            render_statements(&statements),
            "ALTER TABLE users ADD COLUMN a TEXT;\nALTER TABLE users ADD COLUMN b TEXT;"
        );
    }

    #[tokio::test]
    async fn run_tracks_applied_migrations_and_skips_them_later() {
        let fixture = TestProject::new();

        run(fixture.config_path(), None, false, true, None)
            .await
            .expect("first migration run should succeed");

        let config = TideConfig::load(fixture.config_path()).expect("config should load");
        let ran = get_ran_migrations(&config, fixture.migrations_path())
            .await
            .expect("ran migrations should load");
        let pending = get_pending_migrations(&config, fixture.migrations_path())
            .await
            .expect("pending migrations should load");

        assert_eq!(ran.len(), 1);
        assert_eq!(ran[0].version, "20260321171859");
        assert_eq!(ran[0].file_name, "20260321171859_create_users_table");
        assert!(pending.is_empty());

        run(fixture.config_path(), None, false, true, None)
            .await
            .expect("second migration run should succeed");

        let pending_after_second_run = get_pending_migrations(&config, fixture.migrations_path())
            .await
            .expect("pending migrations should still be empty");
        assert!(pending_after_second_run.is_empty());
    }

    #[tokio::test]
    async fn rollback_removes_migration_record() {
        let fixture = TestProject::new();

        run(fixture.config_path(), None, false, true, None)
            .await
            .expect("migration run should succeed");

        let config = TideConfig::load(fixture.config_path()).expect("config should load");
        let ran = get_ran_migrations(&config, fixture.migrations_path())
            .await
            .expect("ran migrations should load");

        run_migration_down(&config, &ran[0])
            .await
            .expect("rollback should succeed");

        let ran_after_rollback = get_ran_migrations(&config, fixture.migrations_path())
            .await
            .expect("ran migrations should load after rollback");
        let pending_after_rollback = get_pending_migrations(&config, fixture.migrations_path())
            .await
            .expect("pending migrations should load after rollback");

        assert!(ran_after_rollback.is_empty());
        assert_eq!(pending_after_rollback.len(), 1);
    }

    #[tokio::test]
    async fn get_ran_migrations_reads_metadata_rows_like_library_migrator() {
        let fixture = TestProject::new();
        let config = TideConfig::load(fixture.config_path()).expect("config should load");

        crate::runtime_db::ensure_migration_table(&config, &config.migration.table)
            .await
            .expect("migration table should be created");
        let db = crate::runtime_db::connect(&config)
            .await
            .expect("database should connect");
        crate::runtime_db::execute_on_db(
            &db,
            "INSERT INTO \"_migrations\" (\"version\", \"name\") VALUES ('20260321171859', 'create_users_table')",
        )
        .await
        .expect("migration row should be inserted");

        let ran = get_ran_migrations(&config, fixture.migrations_path())
            .await
            .expect("ran migrations should load");

        assert_eq!(ran.len(), 1);
        assert_eq!(ran[0].version, "20260321171859");
        assert_eq!(ran[0].name, "create_users_table");
        assert!(ran[0].applied_at.is_some());
    }

    #[tokio::test]
    async fn ran_migrations_are_ordered_by_application_not_by_version() {
        let fixture = TestProject::new();
        let config = TideConfig::load(fixture.config_path()).expect("config should load");

        crate::runtime_db::ensure_migration_table(&config, &config.migration.table)
            .await
            .expect("migration table should be created");
        let db = crate::runtime_db::connect(&config)
            .await
            .expect("database should connect");

        // A long lived branch merges, so the newer version string is applied
        // first and the older one last.
        for (version, name) in [
            ("20260401000000", "add_orders_table"),
            ("20260321171859", "create_users_table"),
        ] {
            crate::runtime_db::execute_on_db(
                &db,
                &format!(
                    "INSERT INTO \"_migrations\" (\"version\", \"name\") VALUES ('{}', '{}')",
                    version, name
                ),
            )
            .await
            .expect("migration row should be inserted");
        }

        let ran = get_ran_migrations(&config, fixture.migrations_path())
            .await
            .expect("ran migrations should load");
        let versions: Vec<_> = ran
            .iter()
            .map(|migration| migration.version.as_str())
            .collect();

        assert_eq!(versions, vec!["20260401000000", "20260321171859"]);
        // Rollback reverses this list, so it must revert the last row applied.
        assert_eq!(
            ran.last().map(|migration| migration.version.as_str()),
            Some("20260321171859")
        );
    }

    #[tokio::test]
    async fn mark_records_a_migration_without_executing_it() {
        let fixture = TestProject::new();

        migrate_mark(
            fixture.config_path(),
            "create_users_table",
            false,
            false,
            false,
        )
        .await
        .expect("mark should record the migration");

        let config = TideConfig::load(fixture.config_path()).expect("config should load");
        let ran = get_ran_migrations(&config, fixture.migrations_path())
            .await
            .expect("ran migrations should load");

        assert_eq!(ran.len(), 1);
        assert_eq!(ran[0].version, "20260321171859");

        let tables = crate::runtime_db::list_tables(&config)
            .await
            .expect("tables should be listed");
        assert!(
            !tables.iter().any(|table| table == "users"),
            "mark must not execute the migration SQL"
        );

        // Marking an already recorded migration is an error, not a silent no-op.
        assert!(
            migrate_mark(
                fixture.config_path(),
                "create_users_table",
                false,
                false,
                false
            )
            .await
            .is_err()
        );

        migrate_mark(
            fixture.config_path(),
            "create_users_table",
            true,
            false,
            false,
        )
        .await
        .expect("unmark should remove the ledger row");

        let pending = get_pending_migrations(&config, fixture.migrations_path())
            .await
            .expect("pending migrations should load");
        assert_eq!(pending.len(), 1);
    }

    #[tokio::test]
    async fn generate_reports_a_broken_config_instead_of_defaulting_to_postgres() {
        let dir = TempDir::new().expect("temp dir should be created");
        let config_path = dir.path().join("tideorm.toml");
        fs::write(&config_path, "[database]\ndriver = \n").expect("config should be written");

        let error = generate_migration(
            config_path.to_str().expect("config path should be utf-8"),
            "create_users_table",
            None,
            None,
            None,
            false,
        )
        .await
        .expect_err("a malformed tideorm.toml must not fall back to the Postgres defaults");

        assert!(error.contains("Failed to parse config file"), "{}", error);
    }

    struct TestProject {
        _dir: TempDir,
        config_path: String,
        migrations_path: String,
    }

    impl TestProject {
        fn new() -> Self {
            let dir = TempDir::new().expect("temp dir should be created");
            let root = dir.path();
            let migrations_dir = root.join("src").join("migrations");
            fs::create_dir_all(&migrations_dir).expect("migrations directory should be created");

            let database_path = slash_path(root.join("test.sqlite3"));
            let config_path = root.join("tideorm.toml");
            let migrations_path = slash_path(&migrations_dir);
            let models_path = slash_path(root.join("src").join("models"));
            let seeders_path = slash_path(root.join("src").join("seeders"));
            let factories_path = slash_path(root.join("src").join("factories"));
            let config_file_path = slash_path(root.join("src").join("config.rs"));

            let config_contents = format!(
                "[project]\nname = \"test-project\"\nenvironment = \"development\"\n\n[database]\ndriver = \"sqlite\"\nsqlite_path = \"{}\"\n\n[paths]\nmigrations = \"{}\"\nmodels = \"{}\"\nseeders = \"{}\"\nfactories = \"{}\"\nconfig_file = \"{}\"\n\n[migration]\ntable = \"_migrations\"\ntimestamps = true\n\n[seeder]\ndefault_seeder = \"DatabaseSeeder\"\n\n[model]\ntimestamps = true\nsoft_deletes = false\ntokenize = false\nprimary_key = \"id\"\nprimary_key_type = \"i64\"\n",
                database_path,
                migrations_path,
                models_path,
                seeders_path,
                factories_path,
                config_file_path
            );
            fs::write(&database_path, b"").expect("database file should be created");
            fs::write(&config_path, config_contents).expect("config should be written");

            fs::write(migrations_dir.join("mod.rs"), "//! Database migrations\n")
                .expect("mod.rs should be written");
            fs::write(
                migrations_dir.join("20260321171859_create_users_table.rs"),
                TEST_MIGRATION,
            )
            .expect("migration should be written");

            Self {
                _dir: dir,
                config_path: slash_path(config_path),
                migrations_path,
            }
        }

        fn config_path(&self) -> &str {
            &self.config_path
        }

        fn migrations_path(&self) -> &str {
            &self.migrations_path
        }
    }

    fn slash_path(path: impl AsRef<std::path::Path>) -> String {
        path.as_ref().to_string_lossy().replace('\\', "/")
    }

    const TEST_MIGRATION: &str = r##"//! Migration: create_users_table

use tideorm::prelude::*;

pub struct CreateUsersTable;

#[async_trait]
impl Migration for CreateUsersTable {
    fn version(&self) -> &str {
        "20260321171859"
    }

    fn name(&self) -> &str {
        "create_users_table"
    }

    async fn up(&self, schema: &mut Schema) -> tideorm::Result<()> {
        schema.raw(r#"
        CREATE TABLE IF NOT EXISTS users (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL
        )
        "#).await?;
        Ok(())
    }

    async fn down(&self, schema: &mut Schema) -> tideorm::Result<()> {
        schema.raw(r#"DROP TABLE IF EXISTS users"#).await?;
        Ok(())
    }
}
"##;
}
