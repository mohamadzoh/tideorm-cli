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
use syn::spanned::Spanned;
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
        } => {
            migrate_down(
                config_path,
                step.unwrap_or(1),
                migration,
                pretend,
                force,
                verbose,
            )
            .await
        }
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
        let migrations_to_rollback = last_applied(&config, step).await?;

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

        ensure_all_roll_back(&migrations_to_rollback, "roll them back")?;
        roll_back(&config, &migrations_to_rollback).await?;

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

    let redone = redo_last(&config, step, pretend).await?;
    if !pretend && redone > 0 {
        print_success(&format!("Redid {} migration(s)", redone));
    }

    Ok(())
}

/// Roll back the last `step` applied migrations, then apply exactly those again,
/// oldest first; returns how many that was.
///
/// `migrate up --step` would apply the first *pending* migrations instead, which are
/// other ones whenever an older migration is still pending.
async fn redo_last(config: &TideConfig, step: u32, pretend: bool) -> Result<usize, String> {
    let migrations = last_applied(config, step).await?;

    if migrations.is_empty() {
        print_info("Nothing to redo");
        return Ok(0);
    }

    // Checked before anything is rolled back: a migration that cannot be rolled back,
    // or applied again, would otherwise be left half done.
    ensure_all_roll_back(&migrations, "redo them")?;
    ensure_all_run_up(&migrations, "redo them")?;

    if pretend {
        println!("\n{}", "Pretend mode - migrations to redo:".cyan());
        for migration in &migrations {
            println!("\n-- Migration: {}\n-- Down:", migration.file_name);
            println!("{}", migration.down_sql());
        }
        for migration in migrations.iter().rev() {
            println!("\n-- Migration: {}\n-- Up:", migration.file_name);
            println!("{}", migration.up_sql());
        }
        return Ok(migrations.len());
    }

    roll_back(config, &migrations).await?;
    for migration in migrations.iter().rev() {
        print!("  Migrating: {}... ", migration.file_name);
        run_migration_up(config, migration).await?;
        println!("{}", "DONE".green());
    }

    Ok(migrations.len())
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

    // A project that cannot seed is refused before anything is dropped, rather
    // than wiping the database and only then failing on the seed step.
    if seed {
        let runner =
            crate::commands::db::ensure_seeding_supported(config_path).map_err(|reason| {
                format!(
                    "{reason}\nRun `tideorm migrate fresh` without --seed if you only need to drop tables and re-run migrations."
                )
            })?;
        if let Some(name) = &seeder {
            runner.resolve_seed(name)?;
        }
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

    ensure_all_run_up(&migrations, "drop all tables")?;

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

    let migrations: Vec<_> = migrations.into_iter().rev().collect();
    ensure_all_roll_back(&migrations, "reset")?;
    println!("Rolling back {} migration(s)...", migrations.len());
    roll_back(&config, &migrations).await?;

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

    if seed {
        crate::commands::db::ensure_seeding_supported(config_path).map_err(|reason| {
            format!(
                "{reason}\nRun `tideorm migrate refresh` without --seed if you only need to re-run migrations."
            )
        })?;
    }

    if let Some(count) = step {
        redo_last(&config, count, false).await?;
    } else {
        // Checked before the reset: a migration that cannot run would leave the
        // database without the ones after it.
        ensure_all_run_up(&get_all_migrations(&config.paths.migrations)?, "refresh")?;
        migrate_reset(config_path, force, false, verbose).await?;
        run(config_path, None, false, true, None).await?;
    }

    if seed {
        runtime_db::clear_seed_ledger(&config).await?;
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

    let message = if unmark {
        if !already_applied {
            return Err(format!(
                "Migration is not recorded as applied: {}",
                migration.file_name
            ));
        }
        format!("Marked {} as not applied", migration.file_name)
    } else {
        if already_applied {
            return Err(format!(
                "Migration is already recorded as applied: {}",
                migration.file_name
            ));
        }
        format!("Marked {} as applied", migration.file_name)
    };

    let db = runtime_db::connect(&config).await?;
    tideorm::migration::__mark_migration(
        &db,
        &config.migration.table,
        &migration.version,
        &migration.name,
        !unmark,
    )
    .await
    .map_err(|error| error.to_string())?;

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
    /// One entry per SQL statement found in `up()`, in source order, or why
    /// the CLI cannot run it.
    pub up_statements: MethodStatements,
    /// One entry per SQL statement found in `down()`, in source order, or
    /// why the CLI cannot run it.
    pub down_statements: MethodStatements,
    /// What `foreign_keys_off()` returns, or why the CLI cannot tell.
    pub foreign_keys_off: Result<bool, String>,
    pub applied_at: Option<String>,
}

impl Migration {
    /// Rendering of the `up()` statements for display purposes only.
    pub fn up_sql(&self) -> String {
        render_method(&self.up_statements)
    }

    /// Rendering of the `down()` statements for display purposes only.
    pub fn down_sql(&self) -> String {
        render_method(&self.down_statements)
    }

    /// The statements `up()` runs, or why the CLI cannot run it.
    fn runnable_up(&self) -> Result<Vec<String>, String> {
        self.runnable(&self.up_statements, "up")
    }

    /// The statements `down()` runs, or why the CLI cannot run it.
    fn runnable_down(&self) -> Result<Vec<String>, String> {
        self.runnable(&self.down_statements, "down")
    }

    fn runnable(&self, statements: &MethodStatements, method: &str) -> Result<Vec<String>, String> {
        let statements = statements
            .as_ref()
            .map_err(|reason| format!("Migration {}: {reason}", self.file_name))?;
        let statements = executable_statements(statements);
        if statements.is_empty() {
            return Err(format!(
                "Migration {} does not contain executable SQL in {method}()",
                self.file_name
            ));
        }
        Ok(statements)
    }
}

/// A method's statements for display, or why the CLI cannot run it.
fn render_method(statements: &MethodStatements) -> String {
    match statements {
        Ok(statements) => render_statements(statements),
        Err(reason) => format!("-- not runnable by the CLI: {reason}"),
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
    let mut migrations = Vec::new();
    let mut migration_files_by_version = HashMap::new();

    let entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(error) if utils::is_missing_path(path, &error) => return Ok(vec![]),
        Err(error) => {
            return Err(format!("Failed to read migrations directory: {}", error));
        }
    };

    for entry in entries {
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

            let source = Source::new(&content);
            let (version, logical_name) = parse_migration_metadata(&name, &source);
            if let Some(existing_file) =
                migration_files_by_version.insert(version.clone(), name.clone())
            {
                return Err(format!(
                    "Duplicate migration version '{}': '{}.rs' and '{}.rs' cannot both be applied",
                    version, existing_file, name
                ));
            }
            migrations.push(Migration {
                file_name: name,
                version,
                name: logical_name,
                up_statements: method_statements(&source, "up"),
                down_statements: method_statements(&source, "down"),
                foreign_keys_off: foreign_keys_off(&source),
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

        let missing = || Err(format!("its file is missing from {migrations_path}"));
        let mut migration = all_by_name.get(&version).cloned().unwrap_or(Migration {
            file_name: version.clone(),
            version: version.clone(),
            name,
            up_statements: missing(),
            down_statements: missing(),
            foreign_keys_off: Ok(false),
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
    let table = runtime_db::quoted_identifier(config, table_name);
    let id = runtime_db::quoted_identifier(config, "id");
    let version = runtime_db::quoted_identifier(config, "version");
    let name = runtime_db::quoted_identifier(config, "name");
    let applied_at = runtime_db::quoted_identifier(config, "applied_at");
    let applied_at_expr = match config.database.driver_name() {
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
    let name = name.trim();
    if name.is_empty() {
        return Err("Migration name must not be empty".to_string());
    }

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

/// A migration file and what it parses to as Rust.
struct Source<'a> {
    content: &'a str,
    file: Result<syn::File, String>,
}

impl<'a> Source<'a> {
    fn new(content: &'a str) -> Self {
        Self {
            content,
            file: syn::parse_file(content)
                .map_err(|error| format!("the CLI cannot read the file as Rust: {error}")),
        }
    }

    /// The body of the first function named `name`, a method of an `impl`
    /// block or a free function, in source order; `None` without one.
    fn function(&self, name: &str) -> Result<Option<&syn::Block>, String> {
        fn find<'f>(items: &'f [syn::Item], name: &str) -> Option<&'f syn::Block> {
            items.iter().find_map(|item| match item {
                syn::Item::Fn(function) if function.sig.ident == name => Some(&*function.block),
                syn::Item::Impl(block) => block.items.iter().find_map(|item| match item {
                    syn::ImplItem::Fn(method) if method.sig.ident == name => Some(&method.block),
                    _ => None,
                }),
                syn::Item::Mod(module) => module
                    .content
                    .as_ref()
                    .and_then(|(_, items)| find(items, name)),
                _ => None,
            })
        }
        let file = self.file.as_ref().map_err(Clone::clone)?;
        Ok(find(&file.items, name))
    }

    /// The first line of the code `spanned` starts, shortened, for an error.
    fn snippet(&self, spanned: &impl Spanned) -> String {
        let start = spanned.span().start();
        let line = self.content.lines().nth(start.line.saturating_sub(1));
        let code: String = line.unwrap_or("").chars().skip(start.column).collect();
        let code = code.trim();
        match code.char_indices().nth(60) {
            Some((cut, _)) => format!("{}..", &code[..cut]),
            None => code.to_string(),
        }
    }
}

fn parse_migration_metadata(file_name: &str, source: &Source) -> (String, String) {
    let version = method_string_value(source, "version")
        .or_else(|| split_file_name(file_name).map(|(version, _)| version.to_string()))
        .unwrap_or_else(|| file_name.to_string());

    let logical_name = method_string_value(source, "name")
        .or_else(|| split_file_name(file_name).map(|(_, name)| name.to_string()))
        .unwrap_or_else(|| file_name.to_string());

    (version, logical_name)
}

/// The string a method whose body is a string literal returns.
fn method_string_value(source: &Source, method: &str) -> Option<String> {
    match source.function(method).ok()??.stmts.as_slice() {
        [syn::Stmt::Expr(syn::Expr::Lit(literal), None)] => match &literal.lit {
            syn::Lit::Str(value) => Some(value.value()),
            _ => None,
        },
        _ => None,
    }
}

fn split_file_name(file_name: &str) -> Option<(&str, &str)> {
    let (version, name) = file_name.split_once('_')?;
    if version.chars().all(|character| character.is_ascii_digit()) {
        Some((version, name))
    } else {
        None
    }
}

/// What a migration's `foreign_keys_off()` returns: `false` without the method,
/// or why the CLI, which reads the source rather than running it, cannot tell.
fn foreign_keys_off(source: &Source) -> Result<bool, String> {
    let Some(body) = source.function("foreign_keys_off")? else {
        return Ok(false);
    };
    if let [syn::Stmt::Expr(syn::Expr::Lit(literal), None)] = body.stmts.as_slice()
        && let syn::Lit::Bool(value) = &literal.lit
    {
        return Ok(value.value);
    }
    Err(format!(
        "foreign_keys_off() returns something other than `true` or `false`, which the CLI \
         cannot evaluate: `{}`. Run such a migration with TideORM's `Migrator` from the \
         project instead.",
        body.stmts
            .first()
            .map_or_else(String::new, |statement| source.snippet(statement))
    ))
}

/// The SQL of each `schema.raw(..)` statement in the body of `method`, in
/// source order; none when the file has no such method.
///
/// The CLI runs a migration by reading its source, not by compiling it, so it
/// runs these statements and nothing else. A body holding any other code (a
/// builder call, a loop, a binding) is refused rather than run in part.
fn method_statements(source: &Source, method: &str) -> MethodStatements {
    let Some(body) = source.function(method)? else {
        return Ok(Vec::new());
    };
    let mut statements = Vec::new();
    for (index, statement) in body.stmts.iter().enumerate() {
        match statement {
            syn::Stmt::Expr(expression, semicolon) => {
                if let Some(sql) = raw_statement(source, expression, semicolon.is_some())? {
                    statements.push(sql);
                    continue;
                }
                if semicolon.is_none() && index + 1 == body.stmts.len() && is_ok_unit(expression) {
                    continue;
                }
            }
            // What a new migration's stub holds until it is written.
            syn::Stmt::Local(local) if is_unused_schema(local) => continue,
            _ => {}
        }
        return Err(format!(
            "{method}() holds code other than `schema.raw(..)` statements, which the CLI \
             does not run: `{}`. Run such a migration with TideORM's `Migrator` from the \
             project instead.",
            source.snippet(statement)
        ));
    }
    Ok(statements)
}

/// What a migration method runs: its SQL statements, or why the CLI cannot
/// run it.
type MethodStatements = Result<Vec<String>, String>;

/// The SQL of a `schema.raw(<string literal>).await?;` statement; `None`
/// when `expression` is no `schema.raw(..)` call.
fn raw_statement(
    source: &Source,
    expression: &syn::Expr,
    terminated: bool,
) -> Result<Option<String>, String> {
    let mut core = expression;
    while let syn::Expr::Try(syn::ExprTry { expr, .. })
    | syn::Expr::Await(syn::ExprAwait { base: expr, .. }) = core
    {
        core = expr;
    }
    let syn::Expr::MethodCall(call) = core else {
        return Ok(None);
    };
    if call.method != "raw" || !is_path(&call.receiver, "schema") {
        return Ok(None);
    }
    let sql = match (call.args.len(), call.args.first()) {
        (
            1,
            Some(syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Str(sql),
                ..
            })),
        ) => sql.value(),
        _ => {
            return Err(format!(
                "`schema.raw(..)` takes a string literal the CLI can read: `{}`",
                source.snippet(expression)
            ));
        }
    };
    let awaited = matches!(
        expression,
        syn::Expr::Try(syn::ExprTry { expr, .. })
            if matches!(&**expr, syn::Expr::Await(syn::ExprAwait { base, .. })
                if matches!(&**base, syn::Expr::MethodCall(_)))
    );
    if !awaited || !terminated {
        return Err(format!(
            "`schema.raw(..)` must be followed by `.await?;`: `{}`",
            source.snippet(expression)
        ));
    }
    Ok(Some(sql))
}

/// Whether `expression` is the plain path `name`.
fn is_path(expression: &syn::Expr, name: &str) -> bool {
    matches!(expression, syn::Expr::Path(path) if path.qself.is_none() && path.path.is_ident(name))
}

/// Whether `expression` is `Ok(())`.
fn is_ok_unit(expression: &syn::Expr) -> bool {
    matches!(expression, syn::Expr::Call(call)
    if is_path(&call.func, "Ok")
        && matches!(
            call.args.iter().collect::<Vec<_>>().as_slice(),
            [syn::Expr::Tuple(unit)] if unit.elems.is_empty()
        ))
}

/// Whether `local` is `let _ = schema;`.
fn is_unused_schema(local: &syn::Local) -> bool {
    matches!(local.pat, syn::Pat::Wild(_))
        && local
            .init
            .as_ref()
            .is_some_and(|init| init.diverge.is_none() && is_path(&init.expr, "schema"))
}

/// The last `step` migrations applied, the most recent first: the ledger is
/// read in the order they were applied.
async fn last_applied(config: &TideConfig, step: u32) -> Result<Vec<Migration>, String> {
    let ran = get_ran_migrations(config, &config.paths.migrations).await?;
    Ok(ran.into_iter().rev().take(step as usize).collect())
}

/// Roll `migrations` back in the order given.
async fn roll_back(config: &TideConfig, migrations: &[Migration]) -> Result<(), String> {
    for migration in migrations {
        print!("  Rolling back: {}... ", migration.file_name);
        run_migration_down(config, migration).await?;
        println!("{}", "DONE".green());
    }
    Ok(())
}

/// Refuse, before anything changes, to `action` when one of `migrations`
/// cannot be applied: the ones after it would be left out.
fn ensure_all_run_up(migrations: &[Migration], action: &str) -> Result<(), String> {
    for migration in migrations {
        migration
            .runnable_up()
            .map_err(|reason| format!("{reason} - refusing to {action}"))?;
    }
    Ok(())
}

/// Refuse, before anything is rolled back, to `action` several migrations
/// when one of them cannot be: the newer ones would be left reverted.
fn ensure_all_roll_back(migrations: &[Migration], action: &str) -> Result<(), String> {
    for migration in migrations {
        migration
            .runnable_down()
            .map_err(|reason| format!("{reason} - refusing to {action}"))?;
    }
    Ok(())
}

/// Run a migration up
async fn run_migration_up(config: &TideConfig, migration: &Migration) -> Result<(), String> {
    let up_statements = migration.runnable_up()?;
    run_sql_migration(config, migration, up_statements, true).await
}

/// Apply (`apply`) or revert a migration through TideORM's migrator: holding
/// its lock, with the ledger write in the same transaction as the statements
/// where the backend's DDL is transactional. A migration another run applied
/// (or reverted) meanwhile is left alone.
async fn run_sql_migration(
    config: &TideConfig,
    migration: &Migration,
    statements: Vec<String>,
    apply: bool,
) -> Result<(), String> {
    // SQLite changes foreign key enforcement only outside a transaction, and the
    // migration runs in one, where `PRAGMA foreign_keys = OFF` does nothing: a
    // rebuild's `DROP TABLE` then deletes the rows pointing at the table. A migration
    // asking for them off, through `foreign_keys_off()` as TideORM's `Migrator` reads
    // it or by turning them off itself, runs on a connection of its own that has them
    // off before the transaction begins, and fails, rolling back, if a foreign key
    // then points at no row, as SQLite's own procedure for rebuilding a table checks;
    // one that already did is refused first. The migration's own pragmas still do
    // nothing inside the transaction.
    let foreign_keys_off = config.database.driver_name() == "sqlite"
        && (migration
            .foreign_keys_off
            .clone()
            .map_err(|reason| format!("Migration {}: {reason}", migration.file_name))?
            || statements
                .iter()
                .any(|statement| tideorm::migration::__disables_foreign_keys(statement)));
    let (db, statements) = if foreign_keys_off {
        let db = runtime_db::connect_single_session(config).await?;
        runtime_db::execute_on_db(&db, "PRAGMA foreign_keys = OFF").await?;
        let statements = foreign_key_check(FOREIGN_KEYS_ALREADY_BROKEN)
            .into_iter()
            .chain(statements)
            .chain(foreign_key_check(FOREIGN_KEYS_LEFT_BROKEN))
            .collect();
        (db, statements)
    } else {
        (runtime_db::connect(config).await?, statements)
    };
    tideorm::migration::__run_sql_migration(
        &db,
        &config.migration.table,
        &migration.version,
        &migration.name,
        statements,
        apply,
    )
    .await
    .map(|_| ())
    .map_err(|error| ledger_failure_message(config, migration, error.to_string()))
}

/// What fails a SQLite migration run with foreign keys off whose database had a
/// row's foreign key matching no row before it ran.
const FOREIGN_KEYS_ALREADY_BROKEN: &str = "the database already has a foreign key pointing \
     at no row: fix it before running a migration with foreign keys off";

/// What fails one that leaves such a row.
const FOREIGN_KEYS_LEFT_BROKEN: &str = "the migration leaves a foreign key pointing at no row";

/// Statements failing the transaction of a SQLite migration run with foreign keys off
/// when a row's foreign key matches no row: the insert breaks the check named `failure`.
fn foreign_key_check(failure: &str) -> [String; 3] {
    [
        format!(
            "CREATE TEMP TABLE tideorm_foreign_key_check (holds INTEGER NOT NULL \
             CONSTRAINT \"{failure}\" CHECK (holds))"
        ),
        "INSERT INTO tideorm_foreign_key_check \
         SELECT NOT EXISTS (SELECT 1 FROM pragma_foreign_key_check)"
            .to_string(),
        "DROP TABLE tideorm_foreign_key_check".to_string(),
    ]
}

/// Annotate a failed apply/rollback on backends that cannot roll DDL back.
///
/// MySQL and MariaDB implicitly commit DDL, so the surrounding transaction
/// cannot undo a `CREATE TABLE` whose ledger write failed afterwards. The
/// schema change is then permanent and unrecorded, which every later run trips
/// over, so point at the escape hatch that repairs the ledger.
fn ledger_failure_message(config: &TideConfig, migration: &Migration, error: String) -> String {
    match config.database.driver_name() {
        "mysql" => format!(
            "{}\nMySQL/MariaDB commit DDL implicitly, so the schema change may have been applied without the migration ledger being updated. If so, reconcile it with `tideorm migrate mark --migration {}` (add --unmark for a rollback).",
            error, migration.file_name
        ),
        _ => error,
    }
}

/// Run a migration down
async fn run_migration_down(config: &TideConfig, migration: &Migration) -> Result<(), String> {
    let down_statements = migration.runnable_down()?;
    run_sql_migration(config, migration, down_statements, false).await
}

/// Drop all tables in the database
async fn drop_all_tables(config: &TideConfig) -> Result<(), String> {
    runtime_db::wipe_tables(config, true).await
}

#[cfg(test)]
mod tests {
    use super::{
        Source, find_migration, generate_migration, get_all_migrations, method_statements,
        method_string_value, migrate_mark, render_statements,
    };
    use super::{get_pending_migrations, get_ran_migrations, run, run_migration_down};
    use crate::config::TideConfig;
    use std::fs;
    use tempfile::TempDir;

    fn up_body(body: &str) -> Result<Vec<String>, String> {
        let content = format!(
            "impl Migration for M {{\n    async fn up(&self, schema: &mut Schema) -> \
             tideorm::Result<()> {{\n{body}\n    }}\n\n    async fn down(&self, schema: &mut \
             Schema) -> tideorm::Result<()> {{\n        Ok(())\n    }}\n}}\n"
        );
        method_statements(&Source::new(&content), "up")
    }

    #[test]
    fn empty_migration_lookup_does_not_match_every_migration() {
        let error = find_migration("missing-migrations", "  ")
            .expect_err("a blank selector must fail before matching migrations");
        assert!(error.contains("Migration name must not be empty"));
    }

    #[test]
    fn migration_discovery_only_treats_a_missing_directory_as_empty() {
        assert!(get_all_migrations("missing-migrations").unwrap().is_empty());

        let dir = TempDir::new().expect("temp dir should be created");
        let file_path = dir.path().join("not-a-directory");
        fs::write(&file_path, "").expect("file should be written");

        let error = get_all_migrations(file_path.to_str().expect("path should be utf-8"))
            .expect_err("a non-directory migrations path should report its I/O error");
        assert!(
            error.contains("Failed to read migrations directory"),
            "{error}"
        );
    }

    #[test]
    fn duplicate_versions_are_rejected_before_migration_execution() {
        let dir = TempDir::new().expect("temp dir should be created");
        let version = "20260101000000";
        let first = TEST_MIGRATION.replace("20260321171859", version);
        let second = first
            .replace("create_users_table", "create_posts_table")
            .replace("CreateUsersTable", "CreatePostsTable")
            .replace("users", "posts");
        fs::write(
            dir.path().join(format!("{version}_create_users_table.rs")),
            first,
        )
        .expect("first migration should be written");
        fs::write(
            dir.path().join(format!("{version}_create_posts_table.rs")),
            second,
        )
        .expect("second migration should be written");

        let error = get_all_migrations(dir.path().to_str().expect("path should be utf-8"))
            .expect_err("duplicate migration versions cannot be applied safely");
        assert!(error.contains(version), "{error}");
        assert!(error.contains("create_users_table.rs"), "{error}");
        assert!(error.contains("create_posts_table.rs"), "{error}");
    }

    #[test]
    fn extracts_every_statement_and_keeps_quoted_identifiers_intact() {
        let body = r####"
        schema.raw(r#"ALTER TABLE "users" ADD COLUMN "a" TEXT"#).await?;
        schema
            .raw("ALTER TABLE \"users\" ADD COLUMN \"b\" TEXT")
            .await?;
        // schema.raw(r#"only an example"#).await?;
        /* schema.raw(r#"also only an example"#).await?; */
        schema.raw(r#"
CREATE FUNCTION touch() RETURNS trigger AS $$
BEGIN
  NEW.meta := '{"a": {"b": 1}
}';
  RETURN NEW;
END $$ LANGUAGE plpgsql"#).await?;
        let _ = schema;

        Ok(())
    "####;

        let statements = up_body(body).unwrap();
        assert_eq!(statements.len(), 3);
        assert_eq!(statements[0], "ALTER TABLE \"users\" ADD COLUMN \"a\" TEXT");
        assert_eq!(statements[1], "ALTER TABLE \"users\" ADD COLUMN \"b\" TEXT");
        assert!(
            statements[2].ends_with("END $$ LANGUAGE plpgsql"),
            "{}",
            statements[2]
        );
    }

    #[test]
    fn ignores_method_examples_in_comments_and_string_literals() {
        let content = r####"
        // async fn up(&self, schema: &mut Schema) -> tideorm::Result<()> {
        //     schema.raw(r#"DROP TABLE users"#).await?;
        //     Ok(())
        // }
        const EXAMPLE: &str = r##"async fn up(&self, schema: &mut Schema) {
            schema.raw(r#"DROP TABLE posts"#).await?;
        }"##;
        async fn up(&self, schema: &mut Schema) -> tideorm::Result<()> {
            schema.raw(r#"CREATE TABLE users (id INTEGER)"#).await?;
            Ok(())
        }
        "####;

        assert_eq!(
            method_statements(&Source::new(content), "up").unwrap(),
            vec!["CREATE TABLE users (id INTEGER)".to_string()]
        );
    }

    #[test]
    fn ignores_metadata_examples_in_comments() {
        let content = r####"
        // fn version(&self) -> &str { "99999999999999" }
        fn version(&self) -> &str { "20261017000000" }
        "####;

        assert_eq!(
            method_string_value(&Source::new(content), "version"),
            Some("20261017000000".to_string())
        );
    }

    /// Code the CLI does not run is refused, never skipped: what follows a
    /// nested block still counts, and a builder call is not ignored.
    #[test]
    fn a_method_holding_other_code_is_refused() {
        for body in [
            r####"
        for sql in [r#"CREATE TABLE a (x INT)"#] {
            schema.raw(sql).await?;
        }
        schema.raw(r#"CREATE INDEX i ON a (x)"#).await?;
        Ok(())"####,
            r####"
        schema.raw(r#"CREATE EXTENSION IF NOT EXISTS pg_trgm"#).await?;
        schema.create_table("posts", |t| { t.id(); }).await?;
        Ok(())"####,
            r####"
        let example = r#"this is not a migration"#;
        Ok(())"####,
            r####"
        schema.raw(SQL).await?;
        Ok(())"####,
        ] {
            let error = up_body(body).unwrap_err();
            assert!(
                error.contains("up()") || error.contains("schema.raw"),
                "{error}"
            );
        }
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

    /// Versions of the applied and the pending migrations, in that order.
    async fn applied_and_pending(fixture: &TestProject) -> (Vec<String>, Vec<String>) {
        let config = TideConfig::load(fixture.config_path()).expect("config should load");
        let versions = |migrations: Vec<super::Migration>| {
            migrations
                .into_iter()
                .map(|migration| migration.version)
                .collect::<Vec<_>>()
        };
        let ran = get_ran_migrations(&config, fixture.migrations_path())
            .await
            .expect("ran migrations should load");
        let pending = get_pending_migrations(&config, fixture.migrations_path())
            .await
            .expect("pending migrations should load");
        (versions(ran), versions(pending))
    }

    /// Apply all three migrations, then roll the middle one back, so an older migration
    /// is pending while a newer one is the last applied.
    async fn project_with_an_older_migration_pending() -> TestProject {
        let fixture = TestProject::new();
        fixture.add_table_migration("20260322000000", "posts");
        fixture.add_table_migration("20260323000000", "tags");
        run(fixture.config_path(), None, false, true, None)
            .await
            .expect("migrations should run");

        let config = TideConfig::load(fixture.config_path()).expect("config should load");
        let posts = super::find_migration(fixture.migrations_path(), "create_posts_table")
            .expect("migration should be found");
        run_migration_down(&config, &posts)
            .await
            .expect("rollback should succeed");
        fixture
    }

    /// Rolling several migrations back - `reset`, `redo`, `refresh --step` - checks
    /// first that every one can be: one with nothing in `down()`, or whose file is
    /// gone, used to be reached only after the newer ones were reverted, which were
    /// then left so.
    #[tokio::test]
    async fn rolling_back_several_migrations_checks_them_all_first() {
        let fixture = TestProject::new();
        fixture.add_table_migration("20260322000000", "posts");
        fixture.add_table_migration("20260323000000", "tags");
        let posts = format!(
            "{}/20260322000000_create_posts_table.rs",
            fixture.migrations_path()
        );
        let source = fs::read_to_string(&posts)
            .unwrap()
            .replace("schema.raw(r#\"DROP TABLE IF EXISTS posts\"#).await?;", "");
        fs::write(&posts, source).unwrap();
        run(fixture.config_path(), None, false, true, None)
            .await
            .expect("migrations should run");
        let applied = vec![
            "20260321171859".to_string(),
            "20260322000000".to_string(),
            "20260323000000".to_string(),
        ];

        let redo = super::migrate_redo(fixture.config_path(), 2, false, false, false)
            .await
            .unwrap_err();
        assert!(redo.contains("down()"), "{redo}");
        let refresh = super::migrate_refresh(fixture.config_path(), false, Some(2), false, false)
            .await
            .unwrap_err();
        assert!(refresh.contains("down()"), "{refresh}");
        let reset = super::migrate_reset(fixture.config_path(), false, false, false)
            .await
            .unwrap_err();
        assert!(reset.contains("down()"), "{reset}");
        assert_eq!(applied_and_pending(&fixture).await.0, applied);

        fs::remove_file(&posts).unwrap();
        let reset = super::migrate_reset(fixture.config_path(), false, false, false)
            .await
            .unwrap_err();
        assert!(reset.contains("missing"), "{reset}");
        assert_eq!(applied_and_pending(&fixture).await.0, applied);
    }

    /// A unique field added to a SQLite table migrates, its uniqueness enforced, and
    /// rolls back: SQLite refused the `ADD COLUMN .. UNIQUE` it used to be.
    #[tokio::test]
    async fn a_unique_column_added_on_sqlite_migrates_and_rolls_back() {
        let fixture = TestProject::new();
        let config = TideConfig::load(fixture.config_path()).expect("config should load");
        run(fixture.config_path(), None, false, true, None)
            .await
            .expect("the users table should be created");
        crate::generators::migration::MigrationGenerator::new(&config)
            .generate(
                "add_email_to_users",
                None,
                Some("users".to_string()),
                Some("email:string:nullable:unique".to_string()),
                false,
                false,
            )
            .expect("the alter migration should be generated");

        run(fixture.config_path(), None, false, true, None)
            .await
            .expect("the unique column should be added");
        let db = crate::runtime_db::connect(&config).await.unwrap();
        crate::runtime_db::execute_on_db(
            &db,
            "INSERT INTO users (name, email) VALUES ('a', 'x@y')",
        )
        .await
        .unwrap();
        assert!(
            crate::runtime_db::execute_on_db(
                &db,
                "INSERT INTO users (name, email) VALUES ('b', 'x@y')"
            )
            .await
            .is_err()
        );
        drop(db);

        let added = super::find_migration(fixture.migrations_path(), "add_email_to_users")
            .expect("migration should be found");
        run_migration_down(&config, &added)
            .await
            .expect("the unique column should be dropped");
        let columns = crate::runtime_db::query_json(&config, "PRAGMA table_info(users)")
            .await
            .unwrap();
        assert!(
            columns.iter().all(|column| column["name"] != "email"),
            "{columns:?}"
        );
    }

    /// A SQLite migration opening with `PRAGMA foreign_keys = OFF` rebuilds a table
    /// with foreign keys off, as SQLite's own procedure does: inside the migration's
    /// transaction the pragma did nothing, and the rebuild's `DROP TABLE` deleted the
    /// rows pointing at the table. One that leaves a foreign key pointing at no row is
    /// rolled back, in whichever form it turns them off.
    #[tokio::test]
    async fn a_migration_turning_foreign_keys_off_keeps_the_rows_pointing_at_a_rebuilt_table() {
        let fixture = TestProject::new();
        fixture.add_parents_and_children(None);
        fixture.add_migration(
            "20260323000000",
            "rebuild_parents",
            None,
            r###"        schema.raw(r#"PRAGMA foreign_keys = OFF"#).await?;
        schema.raw(r#"CREATE TABLE parents_new (id INTEGER PRIMARY KEY, name TEXT NOT NULL)"#).await?;
        schema.raw(r#"INSERT INTO parents_new SELECT id, name FROM parents"#).await?;
        schema.raw(r#"DROP TABLE parents"#).await?;
        schema.raw(r#"ALTER TABLE parents_new RENAME TO parents"#).await?;
        schema.raw(r#"PRAGMA foreign_keys = ON"#).await?;"###,
        );
        let config = TideConfig::load(fixture.config_path()).expect("config should load");

        run(fixture.config_path(), None, false, true, None)
            .await
            .expect("migrations should run");
        assert_eq!(row_count(&config, "children").await, 2);

        fixture.add_migration(
            "20260324000000",
            "orphan_children",
            None,
            r###"        schema.raw(r#"PRAGMA foreign_keys = OFF"#).await?;
        schema.raw(r#"DELETE FROM parents WHERE id = 2"#).await?;"###,
        );
        let error = run(fixture.config_path(), None, false, true, None)
            .await
            .unwrap_err();
        assert!(error.contains("pointing at no row"), "{error}");
        assert_eq!(row_count(&config, "parents").await, 2);
        assert_eq!(row_count(&config, "children").await, 2);

        // The same rebuild as one script, turning foreign keys off another way.
        fs::remove_file(format!(
            "{}/20260324000000_orphan_children.rs",
            fixture.migrations_path()
        ))
        .expect("the failed migration should be removed");
        fixture.add_migration(
            "20260325000000",
            "rebuild_parents_again",
            None,
            r###"        schema.raw(r#"PRAGMA main.foreign_keys(0); CREATE TABLE parents_new (id INTEGER PRIMARY KEY, name TEXT); INSERT INTO parents_new SELECT id, name FROM parents; DROP TABLE parents; ALTER TABLE parents_new RENAME TO parents"#).await?;"###,
        );
        run(fixture.config_path(), None, false, true, None)
            .await
            .expect("the scripted rebuild should run");
        assert_eq!(row_count(&config, "parents").await, 2);
        assert_eq!(row_count(&config, "children").await, 2);
        // The project's own connections still enforce foreign keys.
        let db = crate::runtime_db::connect(&config).await.unwrap();
        assert!(
            crate::runtime_db::execute_on_db(&db, "INSERT INTO children VALUES (3, 99)")
                .await
                .is_err()
        );
    }

    /// A migration returning `true` from `foreign_keys_off()` runs with foreign keys
    /// off, as TideORM's `Migrator` runs it: the CLI only read a `PRAGMA foreign_keys`
    /// statement, so the rebuild's `DROP TABLE` deleted the rows pointing at the
    /// table. A body the CLI cannot read is refused, and rows that already pointed at
    /// no row are reported as the database's rather than the migration's.
    #[tokio::test]
    async fn a_migration_returning_true_from_foreign_keys_off_runs_with_them_off() {
        let fixture = TestProject::new();
        fixture.add_parents_and_children(Some("false"));
        let rebuild = r###"        schema.raw(r#"CREATE TABLE parents_new (id INTEGER PRIMARY KEY, name TEXT)"#).await?;
        schema.raw(r#"INSERT INTO parents_new SELECT id, name FROM parents"#).await?;
        schema.raw(r#"DROP TABLE parents"#).await?;
        schema.raw(r#"ALTER TABLE parents_new RENAME TO parents"#).await?;"###;
        fixture.add_migration("20260323000000", "rebuild_parents", Some("true"), rebuild);
        let config = TideConfig::load(fixture.config_path()).expect("config should load");

        run(fixture.config_path(), None, false, true, None)
            .await
            .expect("migrations should run");
        assert_eq!(row_count(&config, "children").await, 2);

        let unreadable = format!(
            "{}/20260324000000_rebuild_unread.rs",
            fixture.migrations_path()
        );
        fixture.add_migration(
            "20260324000000",
            "rebuild_unread",
            Some("std::env::var(\"REBUILD\").is_ok()"),
            rebuild,
        );
        let error = run(fixture.config_path(), None, false, true, None)
            .await
            .unwrap_err();
        assert!(error.contains("foreign_keys_off()"), "{error}");
        assert_eq!(row_count(&config, "children").await, 2);
        fs::remove_file(unreadable).expect("the refused migration should be removed");

        let db = crate::runtime_db::connect_single_session(&config)
            .await
            .unwrap();
        crate::runtime_db::execute_on_db(&db, "PRAGMA foreign_keys = OFF")
            .await
            .unwrap();
        crate::runtime_db::execute_on_db(&db, "INSERT INTO children VALUES (3, 77)")
            .await
            .unwrap();
        drop(db);
        fixture.add_migration(
            "20260325000000",
            "rebuild_parents_again",
            Some("true"),
            rebuild,
        );
        let error = run(fixture.config_path(), None, false, true, None)
            .await
            .unwrap_err();
        assert!(
            error.contains("already has a foreign key pointing at no row"),
            "{error}"
        );
        assert_eq!(row_count(&config, "children").await, 3);
    }

    /// `redo` used to roll back the last migration and then apply the first pending
    /// one, which was another migration.
    #[tokio::test]
    async fn redo_reapplies_the_migrations_it_rolled_back() {
        let fixture = project_with_an_older_migration_pending().await;

        super::migrate_redo(fixture.config_path(), 1, false, false, false)
            .await
            .expect("redo should succeed");

        assert_eq!(
            applied_and_pending(&fixture).await,
            (
                vec!["20260321171859".to_string(), "20260323000000".to_string()],
                vec!["20260322000000".to_string()]
            )
        );
    }

    #[tokio::test]
    async fn refresh_with_step_reapplies_the_migrations_it_rolled_back() {
        let fixture = project_with_an_older_migration_pending().await;

        super::migrate_refresh(fixture.config_path(), false, Some(1), false, false)
            .await
            .expect("refresh should succeed");

        assert_eq!(
            applied_and_pending(&fixture).await,
            (
                vec!["20260321171859".to_string(), "20260323000000".to_string()],
                vec!["20260322000000".to_string()]
            )
        );
    }

    /// `make migration` with `:indexed` fields creates the index the model declares,
    /// and the rollback drops it ahead of the column SQLite would refuse to drop.
    #[tokio::test]
    async fn generated_indexes_are_created_and_rolled_back() {
        let fixture = TestProject::new();
        let config = TideConfig::load(fixture.config_path()).expect("config should load");
        let generator = crate::generators::migration::MigrationGenerator::new(&config);
        generator
            .generate(
                "create_posts_table",
                Some("posts".to_string()),
                None,
                Some("title:string,user_id:i64:indexed".to_string()),
                false,
                false,
            )
            .expect("create migration should be generated");
        // Versions are millisecond timestamps, and two in one millisecond collide.
        std::thread::sleep(std::time::Duration::from_millis(5));
        generator
            .generate(
                "add_slug_to_posts",
                None,
                Some("posts".to_string()),
                Some("slug:string:nullable:indexed".to_string()),
                false,
                false,
            )
            .expect("alter migration should be generated");

        run(fixture.config_path(), None, false, true, None)
            .await
            .expect("migrations should run");

        let indexes = || async {
            crate::runtime_db::table_indexes(&config, "posts")
                .await
                .expect("indexes should be listed")
                .into_iter()
                .map(|index| index.name)
                .collect::<Vec<_>>()
        };
        let created = indexes().await;
        assert!(
            created.contains(&"idx_posts_user_id".to_string()),
            "{created:?}"
        );
        assert!(
            created.contains(&"idx_posts_slug".to_string()),
            "{created:?}"
        );

        super::migrate_down(fixture.config_path(), 1, None, false, false, false)
            .await
            .expect("the alter migration should roll back");
        assert!(!indexes().await.contains(&"idx_posts_slug".to_string()));
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
                "[project]\nname = \"test-project\"\nenvironment = \"development\"\n\n[database]\ndriver = \"sqlite\"\nsqlite_path = \"{}\"\n\n[paths]\nmigrations = \"{}\"\nmodels = \"{}\"\nseeders = \"{}\"\nfactories = \"{}\"\nconfig_file = \"{}\"\n\n[migration]\ntable = \"_migrations\"\ntimestamps = true\n\n[model]\ntimestamps = true\nsoft_deletes = false\ntokenize = false\nprimary_key = \"id\"\nprimary_key_type = \"i64\"\n",
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

        /// Add a migration creating `table`, as `<version>_create_<table>_table.rs`.
        fn add_table_migration(&self, version: &str, table: &str) {
            let source = TEST_MIGRATION
                .replace("20260321171859", version)
                .replace("users", table);
            fs::write(
                format!(
                    "{}/{}_create_{}_table.rs",
                    self.migrations_path, version, table
                ),
                source,
            )
            .expect("migration should be written");
        }

        /// Add a migration of `version` and `name` whose `up()` runs `up`, a
        /// body of `schema.raw(..)` statements, and whose `foreign_keys_off()`,
        /// when there is one, returns `foreign_keys_off`.
        fn add_migration(
            &self,
            version: &str,
            name: &str,
            foreign_keys_off: Option<&str>,
            up: &str,
        ) {
            let foreign_keys_off = foreign_keys_off.map_or_else(String::new, |value| {
                format!("    fn foreign_keys_off(&self) -> bool {{\n        {value}\n    }}\n\n")
            });
            let source = format!(
                "use tideorm::prelude::*;\n\npub struct M;\n\n#[async_trait]\nimpl Migration for M {{\n    fn version(&self) -> &str {{\n        \"{version}\"\n    }}\n\n    fn name(&self) -> &str {{\n        \"{name}\"\n    }}\n\n{foreign_keys_off}    async fn up(&self, schema: &mut Schema) -> tideorm::Result<()> {{\n{up}\n        Ok(())\n    }}\n\n    async fn down(&self, _schema: &mut Schema) -> tideorm::Result<()> {{\n        Ok(())\n    }}\n}}\n"
            );
            fs::write(
                format!("{}/{version}_{name}.rs", self.migrations_path),
                source,
            )
            .expect("migration should be written");
        }

        /// Add the migration creating `parents` and `children`, two rows each,
        /// every child pointing at a parent.
        fn add_parents_and_children(&self, foreign_keys_off: Option<&str>) {
            self.add_migration(
                "20260322000000",
                "create_parents_and_children",
                foreign_keys_off,
                r###"        schema.raw(r#"CREATE TABLE parents (id INTEGER PRIMARY KEY, name TEXT)"#).await?;
        schema.raw(r#"CREATE TABLE children (id INTEGER PRIMARY KEY, parent_id INTEGER REFERENCES parents (id) ON DELETE CASCADE)"#).await?;
        schema.raw(r#"INSERT INTO parents VALUES (1, 'p'), (2, 'q')"#).await?;
        schema.raw(r#"INSERT INTO children VALUES (1, 1), (2, 2)"#).await?;"###,
            );
        }
    }

    /// The number of rows in `table`.
    async fn row_count(config: &TideConfig, table: &str) -> usize {
        crate::runtime_db::query_json(config, &format!("SELECT id FROM {table}"))
            .await
            .expect("the table should be readable")
            .len()
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
