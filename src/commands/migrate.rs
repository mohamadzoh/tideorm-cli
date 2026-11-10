//! Migration commands for TideORM CLI
//!
//! Migrations are project code, so the project's migration runner,
//! `src/bin/<package>-migrate.rs`, runs them through TideORM's `Migrator` on
//! the database this CLI resolved. The commands here pick what it runs and
//! report what it did.

use crate::MigrateCommands;
use crate::commands::runner::{Role, Runner};
use crate::config::TideConfig;
use crate::generators::migration::MigrationGenerator;
use crate::runtime_db;
use crate::utils::{self, print_info, print_success, print_warning};
use colored::Colorize;

mod runner;

pub(crate) use runner::MigrateRunner;
use runner::versions;

/// Run pending migrations
pub async fn run(
    config_path: &str,
    pretend: bool,
    force: bool,
    step: Option<u32>,
) -> Result<(), String> {
    let config = TideConfig::load(config_path)?;

    config.guard_production(force, "run migrations")?;

    run_pending(&MigrateRunner::build(config_path, &config)?, pretend, step)
}

/// Run the pending migrations, or the first `step` of them.
fn run_pending(runner: &MigrateRunner, pretend: bool, step: Option<u32>) -> Result<(), String> {
    if !pretend {
        println!("\n{}", "Running migrations...".cyan());
        let ran = match step {
            Some(step) => runner.change("run-steps", &[&step.to_string()])?,
            None => runner.change("run", &[])?,
        };
        if ran.is_empty() {
            print_success("Nothing to migrate");
        } else {
            print_success(&format!("Ran {} migration(s) successfully", ran.len()));
        }
        return Ok(());
    }

    print_warning("Running in pretend mode - no changes will be made");
    let pending: Vec<_> = runner
        .status()?
        .into_iter()
        .filter(|migration| migration.state == "pending")
        .take(step.map_or(usize::MAX, |step| step as usize))
        .collect();
    if pending.is_empty() {
        print_success("Nothing to migrate");
        return Ok(());
    }

    println!(
        "\n{} Migrations to run:",
        format!("[{}]", pending.len()).cyan()
    );
    for (index, migration) in pending.iter().enumerate() {
        println!("  {}. {}", index + 1, migration.file_name().yellow());
    }
    println!("\n{}", "Pretend mode - showing SQL:".cyan());
    runner.pretend(&pending, true)
}

/// Handle migration subcommands
pub async fn handle_subcommand(
    config_path: &str,
    cmd: MigrateCommands,
    verbose: bool,
) -> Result<(), String> {
    match cmd {
        MigrateCommands::Run {
            pretend,
            force,
            step,
        } => run(config_path, pretend, force, step).await,
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
        } => migrate_down(
            config_path,
            step.unwrap_or(1),
            migration,
            pretend,
            force,
            verbose,
        ),
        MigrateCommands::Redo {
            step,
            pretend,
            force,
        } => migrate_redo(config_path, step, pretend, force, verbose),
        MigrateCommands::Fresh {
            seed,
            seeder,
            force,
        } => migrate_fresh(config_path, seed, seeder, force, verbose).await,
        MigrateCommands::Reset { force, pretend } => {
            migrate_reset(config_path, force, pretend, verbose)
        }
        MigrateCommands::Refresh { seed, step, force } => {
            migrate_refresh(config_path, seed, step, force, verbose).await
        }
        MigrateCommands::Mark {
            migration,
            unmark,
            force,
        } => migrate_mark(config_path, &migration, unmark, force, verbose),
        MigrateCommands::Status => migration_status(config_path, verbose),
        MigrateCommands::History { limit } => migration_history(config_path, limit, verbose),
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

    config.guard_production(force, "run migrate:up")?;

    if verbose {
        print_info("Running migration up...");
    }

    let Some(migration_name) = migration else {
        return run(config_path, pretend, force, step).await;
    };
    print_info(&format!("Running specific migration: {}", migration_name));

    let runner = MigrateRunner::build(config_path, &config)?;
    let migration = runner.find(&migration_name)?;
    if pretend {
        println!("\n{}", "Pretend mode - SQL to execute:".cyan());
        return runner.pretend(&[migration], true);
    }

    runner.change("up", &[&migration.version])?;
    print_success(&format!("Migration {} completed", migration.file_name()));
    Ok(())
}

/// Run migration down (rollback)
fn migrate_down(
    config_path: &str,
    step: u32,
    migration: Option<String>,
    pretend: bool,
    force: bool,
    verbose: bool,
) -> Result<(), String> {
    let config = TideConfig::load(config_path)?;

    config.guard_production(force, "run migrate:down")?;

    if verbose {
        print_info(&format!("Rolling back {} migration(s)...", step));
    }

    let runner = MigrateRunner::build(config_path, &config)?;
    if let Some(migration_name) = migration {
        let migration = runner.find(&migration_name)?;
        if pretend {
            println!("\n{}", "Pretend mode - SQL to execute:".cyan());
            return runner.pretend(&[migration], false);
        }
        runner.change("down", &[&migration.version])?;
        print_success(&format!("Rolled back migration: {}", migration.file_name()));
        return Ok(());
    }

    if pretend {
        let migrations = runner.last_applied(step as usize)?;
        if migrations.is_empty() {
            print_info("Nothing to rollback");
            return Ok(());
        }
        println!("\n{}", "Pretend mode - migrations to rollback:".cyan());
        return runner.pretend(&migrations, false);
    }

    let rolled_back = runner.change("rollback-steps", &[&step.to_string()])?;
    if rolled_back.is_empty() {
        print_info("Nothing to rollback");
    } else {
        print_success(&format!("Rolled back {} migration(s)", rolled_back.len()));
    }
    Ok(())
}

/// Redo last migration(s)
fn migrate_redo(
    config_path: &str,
    step: u32,
    pretend: bool,
    force: bool,
    verbose: bool,
) -> Result<(), String> {
    let config = TideConfig::load(config_path)?;

    config.guard_production(force, "run migrate:redo")?;

    if verbose {
        print_info(&format!("Redoing {} migration(s)...", step));
    }

    let runner = MigrateRunner::build(config_path, &config)?;
    let redone = redo_last(&runner, step, pretend)?;
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
fn redo_last(runner: &MigrateRunner, step: u32, pretend: bool) -> Result<usize, String> {
    let newest_first = runner.last_applied(step as usize)?;

    if newest_first.is_empty() {
        print_info("Nothing to redo");
        return Ok(0);
    }
    let oldest_first: Vec<_> = newest_first.iter().rev().cloned().collect();

    if pretend {
        println!("\n{}", "Pretend mode - migrations to redo:".cyan());
        runner.pretend(&newest_first, false)?;
        runner.pretend(&oldest_first, true)?;
    } else {
        runner.change("down", &versions(&newest_first))?;
        runner.change("up", &versions(&oldest_first))?;
    }
    Ok(newest_first.len())
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

    config.guard_production(force, "run migrate:fresh")?;

    if verbose {
        print_warning("This will drop ALL tables and re-run all migrations!");
    }

    // A project that cannot seed is refused before anything is dropped, rather
    // than wiping the database and only then failing on the seed step.
    if seed {
        let runner = Runner::build(config_path, Role::Seed).map_err(|reason| {
            format!(
                "{reason}\nRun `tideorm migrate fresh` without --seed if you only need to drop tables and re-run migrations."
            )
        })?;
        if let Some(name) = &seeder {
            runner.resolve_seed(name)?;
        }
    }

    // Checked BEFORE dropping anything: a project whose runner registers no
    // migration would be wiped and left empty.
    let runner = MigrateRunner::build(config_path, &config)?;
    if runner.status()?.is_empty() {
        return Err(
            "No migrations are registered in `migrations::all()` - refusing to drop all tables."
                .to_string(),
        );
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
    runtime_db::wipe_tables(&config, true).await?;
    print_success("Dropped all tables");

    run_pending(&runner, false, None)?;

    if seed {
        print_info("Running seeders...");
        crate::commands::db::seed(config_path, seeder, true, verbose).await?;
    }

    print_success("Database refreshed successfully");

    Ok(())
}

/// Reset all migrations (rollback all)
fn migrate_reset(
    config_path: &str,
    force: bool,
    pretend: bool,
    verbose: bool,
) -> Result<(), String> {
    let config = TideConfig::load(config_path)?;

    config.guard_production(force, "run migrate:reset")?;

    if verbose {
        print_warning("This will rollback ALL migrations!");
    }

    let runner = MigrateRunner::build(config_path, &config)?;
    if pretend {
        let migrations = runner.last_applied(usize::MAX)?;
        if migrations.is_empty() {
            print_info("Nothing to reset");
            return Ok(());
        }
        println!("\n{}", "Pretend mode - migrations to rollback:".cyan());
        for migration in &migrations {
            println!("  - {}", migration.file_name());
        }
        return Ok(());
    }

    let rolled_back = runner.change("reset", &[])?;
    if rolled_back.is_empty() {
        print_info("Nothing to reset");
    } else {
        print_success(&format!("Reset {} migration(s)", rolled_back.len()));
    }
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

    config.guard_production(force, "run migrate:refresh")?;

    if seed {
        Runner::build(config_path, Role::Seed).map_err(|reason| {
            format!(
                "{reason}\nRun `tideorm migrate refresh` without --seed if you only need to re-run migrations."
            )
        })?;
    }

    let runner = MigrateRunner::build(config_path, &config)?;
    match step {
        Some(count) => {
            redo_last(&runner, count, false)?;
        }
        None => {
            runner.change("refresh", &[])?;
        }
    }

    if seed {
        runtime_db::clear_seed_ledger(&config).await?;
        print_info("Running seeders...");
        crate::commands::db::seed(config_path, None, true, verbose).await?;
    }

    print_success("Database refreshed successfully");

    Ok(())
}

/// Record a migration as applied (or not applied) without running it
///
/// MySQL and MariaDB commit DDL implicitly, so an apply whose ledger write fails
/// leaves the schema changed but unrecorded: every later run then hard-fails on
/// "table already exists" with nothing to reconcile it. Marking the migration as
/// applied is that escape hatch, and `--unmark` covers the mirror case where a
/// rollback dropped the schema but could not delete the ledger row.
fn migrate_mark(
    config_path: &str,
    migration_name: &str,
    unmark: bool,
    force: bool,
    verbose: bool,
) -> Result<(), String> {
    let config = TideConfig::load(config_path)?;

    config.guard_production(force, "run migrate:mark")?;

    let runner = MigrateRunner::build(config_path, &config)?;
    let migration = runner.find(migration_name)?;

    if verbose {
        print_info(&format!(
            "Updating the migration ledger for: {}",
            migration.file_name()
        ));
    }

    let (command, state) = if unmark {
        ("unmark", "not applied")
    } else {
        ("mark", "applied")
    };
    runner.call(command, &[&migration.version])?;

    print_warning("No migration SQL was executed - only the migration ledger was changed");
    print_success(&format!("Marked {} as {}", migration.file_name(), state));

    Ok(())
}

/// Show migration status
fn migration_status(config_path: &str, verbose: bool) -> Result<(), String> {
    let config = TideConfig::load(config_path)?;

    if verbose {
        print_info("Checking migration status...");
    }

    let migrations = MigrateRunner::build(config_path, &config)?.status()?;

    println!("\n{}", "Migration Status:".cyan().bold());
    println!("{}", "─".repeat(60));

    if migrations.is_empty() {
        print_info("No migrations found");
        return Ok(());
    }

    let mut ran = 0;
    for migration in &migrations {
        let status = if migration.state == "applied" {
            ran += 1;
            "Ran".green()
        } else {
            "Pending".yellow()
        };
        println!("  {} {}", status, migration.file_name());
    }

    println!("{}", "─".repeat(60));
    println!(
        "  Total: {} | Ran: {} | Pending: {}",
        migrations.len(),
        ran,
        migrations.len() - ran
    );

    Ok(())
}

/// Show migration history
fn migration_history(config_path: &str, limit: u32, verbose: bool) -> Result<(), String> {
    let config = TideConfig::load(config_path)?;

    if verbose {
        print_info(&format!("Showing last {} migrations...", limit));
    }

    let migrations = MigrateRunner::build(config_path, &config)?.last_applied(limit as usize)?;

    println!("\n{}", "Migration History:".cyan().bold());
    println!("{}", "─".repeat(80));
    println!(
        "  {:<16} {:<40} {:<20}",
        "Version", "Migration", "Applied At"
    );
    println!("{}", "─".repeat(80));

    if migrations.is_empty() {
        print_info("No migrations have been run");
        return Ok(());
    }

    for migration in &migrations {
        println!(
            "  {:<16} {:<40} {:<20}",
            migration.version,
            migration.file_name(),
            migration.applied_at.as_deref().unwrap_or("N/A")
        );
    }

    println!("{}", "─".repeat(80));

    Ok(())
}

#[cfg(test)]
#[path = "../../tests/unit/commands_migrate_tests.rs"]
mod tests;
