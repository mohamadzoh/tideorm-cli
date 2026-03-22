//! Migration commands for TideORM CLI

use crate::config::TideConfig;
use crate::generators::migration::MigrationGenerator;
use crate::runtime_db;
use crate::utils::{self, print_info, print_success, print_warning};
use crate::MigrateCommands;
use colored::Colorize;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use tideorm::internal::{ConnectionTrait, Statement};

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

    if pretend {
        print_warning("Running in pretend mode - no changes will be made");
    }

    let migrations = get_pending_migrations(&config, migrations_path).await?;

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
            println!("{}", migration.up_sql);
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
    verbose: bool,
) -> Result<(), String> {
    let config = TideConfig::load(config_path)?;

    if verbose {
        print_info("Running migration up...");
    }

    if let Some(migration_name) = migration {
        print_info(&format!("Running specific migration: {}", migration_name));

        let migration = find_migration(&config.paths.migrations, &migration_name)?;

        if pretend {
            println!("\n{}", "Pretend mode - SQL to execute:".cyan());
            println!("{}", migration.up_sql);
            return Ok(());
        }

        let ran_migrations = get_ran_migrations(&config, &config.paths.migrations).await?;
        if ran_migrations.iter().any(|ran| ran.version == migration.version) {
            return Err(format!("Migration already ran: {}", migration.file_name));
        }

        run_migration_up(&config, &migration).await?;
        print_success(&format!("Migration {} completed", migration_name));
    } else {
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
        let migration = find_migration(&config.paths.migrations, &migration_name)?;

        if pretend {
            println!("\n{}", "Pretend mode - SQL to execute:".cyan());
            println!("{}", migration.down_sql);
            return Ok(());
        }

        let ran_migrations = get_ran_migrations(&config, &config.paths.migrations).await?;
        if !ran_migrations.iter().any(|ran| ran.version == migration.version) {
            return Err(format!("Migration has not been run: {}", migration.file_name));
        }

        run_migration_down(&config, &migration).await?;
        print_success(&format!("Rolled back migration: {}", migration_name));
    } else {
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
                println!("{}", migration.down_sql);
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
    verbose: bool,
) -> Result<(), String> {
    if verbose {
        print_info(&format!("Redoing {} migration(s)...", step));
    }

    migrate_down(config_path, step, None, pretend, verbose).await?;
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
        return Err("Cannot run migrate:fresh in production without --force flag".to_string());
    }

    if verbose {
        print_warning("This will drop ALL tables and re-run all migrations!");
    }

    if config.is_production()
        && !utils::confirm("Are you sure you want to drop all tables in PRODUCTION?")
    {
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
        migrate_down(config_path, count, None, false, verbose).await?;
        migrate_up(config_path, Some(count), None, false, verbose).await?;
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

    let ran_migrations = get_ran_migrations(&config, &config.paths.migrations).await?;

    println!("\n{}", "Migration History:".cyan().bold());
    println!("{}", "─".repeat(80));
    println!("  {:<16} {:<40} {:<20}", "Version", "Migration", "Applied At");
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
    pub up_sql: String,
    pub down_sql: String,
    pub applied_at: Option<String>,
}

/// Get all migrations from the migrations directory
fn get_all_migrations(migrations_path: &str) -> Result<Vec<Migration>, String> {
    let path = Path::new(migrations_path);

    if !path.exists() {
        return Ok(vec![]);
    }

    let mut migrations = Vec::new();

    for entry in
        fs::read_dir(path).map_err(|error| format!("Failed to read migrations directory: {}", error))?
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
            let (up_sql, down_sql) = parse_migration_content(&content);

            migrations.push(Migration {
                file_name: name,
                version,
                name: logical_name,
                up_sql,
                down_sql,
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
    let ran_versions: HashSet<_> = ran.iter().map(|migration| migration.version.as_str()).collect();

    Ok(all
        .into_iter()
        .filter(|migration| !ran_versions.contains(migration.version.as_str()))
        .collect())
}

/// Get migrations that have been run
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
            up_sql: String::new(),
            down_sql: String::new(),
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

fn migration_records_query(config: &TideConfig, table_name: &str) -> String {
    let table = quoted_identifier(config, table_name);
    let version = quoted_identifier(config, "version");
    let name = quoted_identifier(config, "name");
    let applied_at = quoted_identifier(config, "applied_at");
    let applied_at_expr = match config.database.driver.as_str() {
        "mysql" => format!("CAST({} AS CHAR) AS {}", applied_at, applied_at),
        _ => format!("CAST({} AS TEXT) AS {}", applied_at, applied_at),
    };

    format!(
        "SELECT {}, {}, {} FROM {} ORDER BY {} ASC",
        version, name, applied_at_expr, table, version
    )
}

/// Find a specific migration
fn find_migration(migrations_path: &str, name: &str) -> Result<Migration, String> {
    let migrations = get_all_migrations(migrations_path)?;

    migrations
        .into_iter()
        .find(|migration| {
            migration.file_name.contains(name)
                || migration.version.contains(name)
                || migration.name.contains(name)
        })
        .ok_or_else(|| format!("Migration not found: {}", name))
}

fn parse_migration_metadata(file_name: &str, content: &str) -> (String, String) {
    let version_pattern = regex::Regex::new(r#"fn\s+version\s*\([^)]*\)\s*->\s*&str\s*\{\s*\"([^\"]+)\""#)
        .unwrap();
    let name_pattern = regex::Regex::new(r#"fn\s+name\s*\([^)]*\)\s*->\s*&str\s*\{\s*\"([^\"]+)\""#)
        .unwrap();

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

/// Parse migration file content to extract up/down SQL
fn parse_migration_content(content: &str) -> (String, String) {
    let mut up_sql = String::new();
    let mut down_sql = String::new();

    let up_pattern = regex::Regex::new(r#"fn\s+up\s*\([^)]*\)[^{]*\{([\s\S]*?)\n\s*\}"#).unwrap();
    let down_pattern = regex::Regex::new(r#"fn\s+down\s*\([^)]*\)[^{]*\{([\s\S]*?)\n\s*\}"#).unwrap();

    if let Some(captures) = up_pattern.captures(content) {
        up_sql = extract_sql_from_method(&captures[1]);
    }

    if let Some(captures) = down_pattern.captures(content) {
        down_sql = extract_sql_from_method(&captures[1]);
    }

    (up_sql, down_sql)
}

/// Extract SQL from method body
fn extract_sql_from_method(method_body: &str) -> String {
    let sql_pattern = regex::Regex::new(r##"r#?\"([^\"]*)\"#?"##).unwrap();
    let mut sqls = Vec::new();

    for captures in sql_pattern.captures_iter(method_body) {
        sqls.push(captures[1].to_string());
    }

    sqls.join("\n")
}

/// Run a migration up
async fn run_migration_up(config: &TideConfig, migration: &Migration) -> Result<(), String> {
    let up_sql = migration.up_sql.trim();
    if up_sql.is_empty() {
        return Err(format!(
            "Migration {} does not contain executable SQL in up()",
            migration.file_name
        ));
    }

    let db = runtime_db::connect(config).await?;
    runtime_db::ensure_migration_table_on_db(&db, config, &config.migration.table).await?;
    let up_sql = up_sql.to_string();
    let insert_sql = format!(
        "INSERT INTO {} ({}, {}) VALUES ({}, {})",
        quoted_identifier(config, &config.migration.table),
        quoted_identifier(config, "version"),
        quoted_identifier(config, "name"),
        sql_string(&migration.version),
        sql_string(&migration.name)
    );

    db.transaction(|tx| {
        Box::pin(async move {
            execute_on_transaction(tx.connection(), &up_sql).await?;
            execute_on_transaction(tx.connection(), &insert_sql).await?;
            Ok(())
        })
    })
    .await
    .map_err(|error| error.to_string())
}

/// Run a migration down
async fn run_migration_down(config: &TideConfig, migration: &Migration) -> Result<(), String> {
    let down_sql = migration.down_sql.trim();
    if down_sql.is_empty() {
        return Err(format!(
            "Migration {} does not contain executable SQL in down()",
            migration.file_name
        ));
    }

    let db = runtime_db::connect(config).await?;
    runtime_db::ensure_migration_table_on_db(&db, config, &config.migration.table).await?;
    let down_sql = down_sql.to_string();
    let delete_sql = format!(
        "DELETE FROM {} WHERE {} = {}",
        quoted_identifier(config, &config.migration.table),
        quoted_identifier(config, "version"),
        sql_string(&migration.version)
    );

    db.transaction(|tx| {
        Box::pin(async move {
            execute_on_transaction(tx.connection(), &down_sql).await?;
            execute_on_transaction(tx.connection(), &delete_sql).await?;
            Ok(())
        })
    })
    .await
    .map_err(|error| error.to_string())
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
    use super::{get_pending_migrations, get_ran_migrations, run, run_migration_down};
    use crate::config::TideConfig;
    use std::fs;
    use tempfile::TempDir;

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
        crate::runtime_db::execute(
            &config,
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