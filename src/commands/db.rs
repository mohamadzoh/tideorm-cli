//! Database commands for TideORM CLI

use crate::DbCommands;
use crate::config::TideConfig;
use crate::runtime_db;
use crate::utils::{
    confirm_destructive, package_name, print_info, print_success, print_warning, to_snake_case,
};
use colored::Colorize;
use std::path::{Path, PathBuf};

/// The binary target `tideorm db seed` runs; seeders are project code, so only
/// the project itself can run them. It is named after the package, so two
/// projects installed with `cargo install` do not collide on it.
pub(crate) fn seed_runner_bin(package: &str) -> String {
    format!("{package}-seed")
}

/// Where `tideorm init` scaffolds that binary, relative to the project root.
pub(crate) fn seed_runner_path(package: &str) -> String {
    format!("src/bin/{}.rs", seed_runner_bin(package))
}

/// How to add the runner to a project `tideorm init` did not scaffold.
pub(crate) const SEED_RUNNER_SETUP_HINT: &str = "`tideorm db seed` runs your seeders through src/bin/<package>-seed.rs, a binary named after your package that connects TideORM and runs `seeders::all()`. Projects created by `tideorm init` include it; see \"Seeding\" in the tideorm-cli README to add it to this one.";

/// A project's seeder runner, once it is known to build.
pub(crate) struct SeedRunner {
    manifest: PathBuf,
    bin: String,
}

impl SeedRunner {
    /// `cargo run` of the runner, before its own arguments. `--manifest-path`
    /// keeps the working directory, so a relative SQLite path means the same
    /// file to the runner as to the rest of this CLI.
    fn command(&self) -> std::process::Command {
        let mut command = std::process::Command::new("cargo");
        command
            .args(["run", "--quiet", "--manifest-path"])
            .arg(&self.manifest)
            .args(["--bin", self.bin.as_str(), "--"]);
        command
    }

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

/// The seed of `seeds` that `name` refers to; see [`SeedRunner::resolve_seed`].
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

    if config.is_production() && !force {
        return Err("Cannot run seeders in production without --force flag".to_string());
    }

    let runner = ensure_seeding_supported(config_path)?;
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

    if config.is_production() && !force {
        return Err("Cannot run db:fresh in production without --force flag".to_string());
    }

    // db:fresh always re-seeds, so a project that cannot seed is refused here
    // instead of dropping every table and only then aborting on the seeding
    // step, which would leave it with an empty, unseeded database.
    ensure_seeding_supported(config_path).map_err(|reason| {
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
        "sqlite" => {
            let path = config
                .database
                .sqlite_path
                .as_deref()
                .unwrap_or("database.db");
            println!("  Path:       {}", path);
            let exists = Path::new(path).exists();
            println!(
                "  Status:     {}",
                if exists {
                    "EXISTS".green()
                } else {
                    "NOT FOUND".yellow()
                }
            );
        }
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
    match test_connection(&config).await {
        Ok(()) => println!("{}", "OK".green()),
        Err(e) => println!("{} ({})", "FAILED".red(), e),
    }

    println!("{}", "─".repeat(50));

    Ok(())
}

/// Initialize TideORM metadata tables for the current database
async fn check(config_path: &str, verbose: bool) -> Result<(), String> {
    let config = TideConfig::load(config_path)?;

    if verbose {
        print_info("Initializing TideORM metadata tables...");
    }

    runtime_db::ensure_metadata_tables(&config, &config.migration.table).await?;

    println!("\n{}", "Database Metadata:".cyan().bold());
    println!("{}", "─".repeat(50));
    println!("  Migration table: {}", config.migration.table.green());
    println!(
        "  Seeder table:    {}",
        runtime_db::DEFAULT_SEEDERS_TABLE.green()
    );
    println!("{}", "─".repeat(50));

    print_success("TideORM metadata tables are ready");
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

    if config.is_production() && !force {
        return Err("Cannot drop database in production without --force flag".to_string());
    }

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
/// This drops every table, schema included - it is not a `TRUNCATE`. The drop
/// semantics are load bearing: `migrate fresh` goes through the same
/// `runtime_db::wipe_tables` and relies on them to rebuild from the migrations.
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

    let columns = runtime_db::table_columns(&config, table_name).await?;

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

    let tables = runtime_db::list_tables(&config).await?;

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

/// The project's seeder runner, once it is known to build.
///
/// Commands that seed after a destructive step check this first, so a project
/// without a runner is refused before its tables are dropped.
pub(crate) fn ensure_seeding_supported(config_path: &str) -> Result<SeedRunner, String> {
    let root = Path::new(config_path)
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let manifest = root.join("Cargo.toml");
    let package = package_name(&manifest)
        .map_err(|reason| format!("No seeder runner: {}. {}", reason, SEED_RUNNER_SETUP_HINT))?;

    let runner_path = root.join(seed_runner_path(&package));
    if !runner_path.is_file() {
        return Err(format!(
            "No seeder runner at {}. {}",
            runner_path.display(),
            SEED_RUNNER_SETUP_HINT
        ));
    }

    // Built before anything is dropped: a seeder that does not compile would
    // otherwise only show once the tables were gone.
    let bin = seed_runner_bin(&package);
    let status = std::process::Command::new("cargo")
        .args(["build", "--quiet", "--manifest-path"])
        .arg(&manifest)
        .args(["--bin", bin.as_str()])
        .status()
        .map_err(|error| format!("Failed to run cargo: {}", error))?;
    if !status.success() {
        return Err(format!(
            "The seeder runner does not build ({}); nothing was changed.",
            status
        ));
    }
    Ok(SeedRunner { manifest, bin })
}

/// Resolve the database a `db create` / `db drop` invocation should act on.
///
/// For SQLite the database *is* a file, so an explicitly supplied name is the
/// target file path (relative names resolve against the current directory) and
/// the configured `sqlite_path` is only the fallback for when no name was
/// given. Other drivers fall back to the configured database name.
fn resolve_target_database(config: &TideConfig, name: Option<&str>) -> Result<String, String> {
    let explicit = name.map(str::trim).filter(|value| !value.is_empty());

    if config.database.driver == "sqlite" {
        return Ok(explicit
            .or(config.database.sqlite_path.as_deref())
            .unwrap_or("database.db")
            .to_string());
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
    matches!(config.database.driver.as_str(), "postgres" | "postgresql")
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

/// Drop every table in the database (and, on PostgreSQL, the enum types)
async fn wipe_tables(config: &TideConfig, drop_types: bool) -> Result<(), String> {
    runtime_db::wipe_tables(config, drop_types).await
}

#[cfg(test)]
mod tests {
    use super::{check, drop_types_supported, pick_seed};
    use crate::config::TideConfig;
    use crate::runtime_db;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn a_seeder_is_found_by_its_own_name_or_its_model_name() {
        let seeds = ["database_seeder", "user_seeder", "demo"];
        assert_eq!(pick_seed(&seeds, "UserSeeder").unwrap(), "user_seeder");
        assert_eq!(pick_seed(&seeds, "user_seeder").unwrap(), "user_seeder");
        assert_eq!(pick_seed(&seeds, "User").unwrap(), "user_seeder");
        // A seed named exactly so wins over one with `_seeder` added.
        assert_eq!(pick_seed(&seeds, "Demo").unwrap(), "demo");

        let unknown = pick_seed(&seeds, "Nope").unwrap_err();
        assert!(unknown.contains("No seeder named `Nope`"), "{unknown}");
        assert!(
            unknown.contains("database_seeder, user_seeder, demo"),
            "{unknown}"
        );
        assert!(
            pick_seed(&[], "User")
                .unwrap_err()
                .contains("registers none")
        );
    }

    #[test]
    fn drop_types_is_reported_as_postgres_only() {
        let fixture = TempDbProject::new();
        let mut config = TideConfig::load(fixture.config_path()).expect("config should load");

        config.database.driver = "sqlite".to_string();
        assert!(!drop_types_supported(&config));

        config.database.driver = "mysql".to_string();
        assert!(!drop_types_supported(&config));

        config.database.driver = "postgres".to_string();
        assert!(drop_types_supported(&config));

        config.database.driver = "postgresql".to_string();
        assert!(drop_types_supported(&config));
    }

    #[tokio::test]
    async fn check_creates_metadata_tables_for_sqlite() {
        let fixture = TempDbProject::new();

        check(fixture.config_path(), false)
            .await
            .expect("check should initialize metadata tables");

        let config = TideConfig::load(fixture.config_path()).expect("config should load");
        let tables = runtime_db::list_tables(&config)
            .await
            .expect("tables should be listed");

        assert!(tables.iter().any(|table| table == "_migrations"));
        assert!(
            tables
                .iter()
                .any(|table| table == runtime_db::DEFAULT_SEEDERS_TABLE)
        );
    }

    /// `a_parents` is dropped first, while `b_children` still points at it, so
    /// the drops need the foreign-key switch of their own session.
    #[tokio::test]
    async fn wipe_drops_a_table_other_rows_still_reference() {
        let fixture = TempDbProject::new();
        let config = TideConfig::load(fixture.config_path()).expect("config should load");
        let db = runtime_db::connect(&config).await.expect("connect");
        for sql in [
            "CREATE TABLE a_parents (id INTEGER PRIMARY KEY)",
            "CREATE TABLE b_children (id INTEGER PRIMARY KEY, parent_id INTEGER NOT NULL REFERENCES a_parents(id))",
            "INSERT INTO a_parents (id) VALUES (1)",
            "INSERT INTO b_children (id, parent_id) VALUES (1, 1)",
        ] {
            runtime_db::execute_on_db(&db, sql).await.expect(sql);
        }

        runtime_db::wipe_tables(&config, false)
            .await
            .expect("wipe should drop every table");

        let tables = runtime_db::list_tables(&config)
            .await
            .expect("tables should be listed");
        assert!(tables.is_empty(), "{tables:?}");
    }

    struct TempDbProject {
        _dir: TempDir,
        config_path: String,
    }

    impl TempDbProject {
        fn new() -> Self {
            let dir = TempDir::new().expect("temp dir should be created");
            let root = dir.path();
            let db_path = slash_path(root.join("check.sqlite3"));
            let config_path = root.join("tideorm.toml");

            let config_contents = format!(
                "[project]\nname = \"test-project\"\nenvironment = \"development\"\n\n[database]\ndriver = \"sqlite\"\nsqlite_path = \"{}\"\n\n[paths]\nmigrations = \"src/migrations\"\nmodels = \"src/models\"\nseeders = \"src/seeders\"\nfactories = \"src/factories\"\nconfig_file = \"src/config.rs\"\n\n[migration]\ntable = \"_migrations\"\ntimestamps = true\n\n[model]\ntimestamps = true\nsoft_deletes = false\ntokenize = false\nprimary_key = \"id\"\nprimary_key_type = \"i64\"\n",
                db_path,
            );

            fs::write(&config_path, config_contents).expect("config should be written");

            Self {
                _dir: dir,
                config_path: slash_path(config_path),
            }
        }

        fn config_path(&self) -> &str {
            &self.config_path
        }
    }

    fn slash_path(path: impl AsRef<std::path::Path>) -> String {
        path.as_ref().to_string_lossy().replace('\\', "/")
    }
}
