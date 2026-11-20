//! Init command for TideORM CLI

use crate::commands::runner::Role;
use crate::config::{TideConfig, canonical_driver};
use crate::generators::migration::MIGRATIONS_MOD_RS;
use crate::generators::seeder::{SEEDERS_MOD_RS, register_seeder};
use crate::runtime_db;
use crate::utils::{
    ensure_directory, file_exists, package_name, print_info, print_success, print_warning,
};
use colored::Colorize;

mod options;
mod templates;

use options::{InitOptions, collect_init_options};
use templates::{
    LIB_RS, generate_cargo_toml, generate_config_rs, generate_database_seeder, generate_gitignore,
    generate_main_rs, generate_tideorm_toml,
};

/// Version of the `tideorm` crate pinned into every scaffolded `Cargo.toml`.
///
/// Single source of truth: the template and its test both read this, so they cannot drift
/// apart. Bump it alongside a `tideorm` release.
const SCAFFOLD_TIDEORM_VERSION: &str = "0.18.2";

/// Initialize a new TideORM project
pub async fn run(name: &str, database: &str, verbose: bool) -> Result<(), String> {
    let database = normalize_database(database)?;
    let project_path = if name == "." {
        std::env::current_dir()
            .map_err(|error| format!("Failed to get current directory: {}", error))?
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

    if !project_path.exists() {
        std::fs::create_dir_all(&project_path)
            .map_err(|error| format!("Failed to create project directory: {}", error))?;
        print_success(&format!("Created directory: {}", project_path.display()));
    }

    let _cwd_guard = WorkingDirectoryGuard::change_to(&project_path)?;

    let init_options = collect_init_options(database, verbose)?;
    let should_write_project_config = !file_exists("tideorm.toml") || init_options.overwrite_config;

    if should_write_project_config {
        write_env_file(&init_options)?;
        write_tideorm_config(&init_options)?;
    } else if verbose {
        print_warning("Keeping existing tideorm.toml and skipping env/database initialization");
    }

    let scaffold_config = TideConfig::load("tideorm.toml")?;
    let scaffold_database = scaffold_config.database.driver;
    let scaffold_env_file = scaffold_config.project.env_file;

    create_project_structure(verbose)?;
    create_scaffold_files(&project_path, &scaffold_database, &scaffold_env_file)?;

    if should_write_project_config && init_options.create_database_now {
        create_database_from_config("tideorm.toml", verbose).await?;
    }

    if should_write_project_config && init_options.run_migrations_now {
        crate::commands::migrate::run("tideorm.toml", false, true, None).await?;
    }

    println!("{}", "─".repeat(50));
    println!(
        "\n{}",
        "✓ TideORM project initialized successfully!".green().bold()
    );

    print_next_steps(&init_options, should_write_project_config);

    Ok(())
}

struct WorkingDirectoryGuard {
    original_dir: std::path::PathBuf,
}

impl WorkingDirectoryGuard {
    fn change_to(path: &std::path::Path) -> Result<Self, String> {
        let original_dir = std::env::current_dir()
            .map_err(|error| format!("Failed to get current directory: {}", error))?;
        std::env::set_current_dir(path)
            .map_err(|error| format!("Failed to change directory: {}", error))?;
        Ok(Self { original_dir })
    }
}

impl Drop for WorkingDirectoryGuard {
    fn drop(&mut self) {
        let _ = std::env::set_current_dir(&self.original_dir);
    }
}

fn print_next_steps(options: &InitOptions, wrote_project_config: bool) {
    println!("\n{}", "Next steps:".cyan().bold());

    if wrote_project_config {
        println!(
            "  1. Review {} and {}",
            options.env_file_name.yellow(),
            "tideorm.toml".yellow()
        );
    } else {
        println!("  1. Review your existing {}", "tideorm.toml".yellow());
    }

    println!("  2. Create your first model:");
    println!(
        "     {}",
        "tideorm make model User --fields=\"name:string,email:string:unique\" --migration".yellow()
    );

    if wrote_project_config && options.run_migrations_now {
        println!("  3. Migrations were already run during setup.");
    } else {
        println!("  3. Run migrations:");
        println!("     {}", "tideorm migrate run".yellow());
    }

    println!("  4. Seed the database:");
    println!("     {}", "tideorm db seed".yellow());
}

/// Normalize supported `--database` names and reject typos instead of silently scaffolding
/// a project for a different database.
fn normalize_database(database: &str) -> Result<&'static str, String> {
    canonical_driver(database).ok_or_else(|| {
        format!(
            "Unsupported database type '{}'. Choose postgres, mysql, or sqlite.",
            database
        )
    })
}

fn write_env_file(options: &InitOptions) -> Result<(), String> {
    if !options.replace_database_url {
        let message = format!("Kept existing DATABASE_URL in {}", options.env_file_name);
        print_warning(&message);
        return Ok(());
    }

    let env_path = std::path::Path::new(&options.env_file_name);
    let existed = env_path.exists();
    upsert_env_value(env_path, "DATABASE_URL", &options.database.connection_url())?;

    if existed {
        print_success(&format!("Updated {}", options.env_file_name));
    } else {
        print_success(&format!("Created {}", options.env_file_name));
    }

    Ok(())
}

fn write_tideorm_config(options: &InitOptions) -> Result<(), String> {
    if file_exists("tideorm.toml") && !options.overwrite_config {
        print_warning("tideorm.toml already exists, keeping current file...");
        return Ok(());
    }

    std::fs::write("tideorm.toml", generate_tideorm_toml(options))
        .map_err(|error| format!("Failed to create config file: {}", error))?;
    print_success("Created tideorm.toml");
    Ok(())
}

fn create_project_structure(verbose: bool) -> Result<(), String> {
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

    let mod_files = [
        ("src/models/mod.rs", "//! Database models\n"),
        ("src/migrations/mod.rs", MIGRATIONS_MOD_RS),
        ("src/seeders/mod.rs", SEEDERS_MOD_RS),
        ("src/factories/mod.rs", "//! Model factories\n"),
    ];

    for (path, content) in mod_files {
        if !file_exists(path) {
            std::fs::write(path, content)
                .map_err(|error| format!("Failed to create {}: {}", path, error))?;
        }
    }

    print_success("Created directory structure");
    Ok(())
}

fn create_scaffold_files(
    project_path: &std::path::Path,
    database: &str,
    env_file_name: &str,
) -> Result<(), String> {
    if !file_exists(".gitignore") {
        std::fs::write(".gitignore", generate_gitignore(env_file_name))
            .map_err(|error| format!("Failed to create .gitignore: {}", error))?;
        print_success("Created .gitignore");
    }

    if !file_exists("Cargo.toml") {
        let package_name = infer_package_name(project_path);
        let cargo_toml_content = generate_cargo_toml(&package_name, database);
        std::fs::write("Cargo.toml", cargo_toml_content)
            .map_err(|error| format!("Failed to create Cargo.toml: {}", error))?;
        print_success("Created Cargo.toml");
    }

    // The seeder and migration runners are binaries of their own, so the
    // modules live in a library every binary uses. An existing crate keeps its
    // own layout.
    let package = package_name(std::path::Path::new("Cargo.toml"));
    let runners = [Role::Seed, Role::Migrate];
    if file_exists("src/main.rs") || file_exists("src/lib.rs") {
        for role in runners {
            if !package
                .as_ref()
                .is_ok_and(|package| file_exists(&role.path(package)))
            {
                print_info(role.setup_hint());
            }
        }
    } else {
        match package {
            Ok(package) => {
                let library = package.replace('-', "_");
                ensure_directory("src/bin")?;
                let files = [
                    ("src/lib.rs".to_string(), LIB_RS.to_string()),
                    ("src/main.rs".to_string(), generate_main_rs(&library)),
                ]
                .into_iter()
                .chain(runners.map(|role| (role.path(&package), role.source(&library))));
                for (path, content) in files {
                    std::fs::write(&path, content)
                        .map_err(|error| format!("Failed to create {}: {}", path, error))?;
                    print_success(&format!("Created {}", path));
                }
                ensure_default_run(&package)?;
            }
            Err(reason) => print_warning(&format!(
                "Not creating src/main.rs: {}. {} {}",
                reason,
                Role::Seed.setup_hint(),
                Role::Migrate.setup_hint()
            )),
        }
    }

    if !file_exists("src/config.rs") {
        std::fs::write("src/config.rs", generate_config_rs(database, env_file_name))
            .map_err(|error| format!("Failed to create config.rs: {}", error))?;
        print_success("Created src/config.rs");
    }

    if !file_exists("src/seeders/database_seeder.rs") {
        std::fs::write("src/seeders/database_seeder.rs", generate_database_seeder())
            .map_err(|error| format!("Failed to create DatabaseSeeder: {}", error))?;
        register_seeder("src/seeders", "database_seeder", "DatabaseSeeder")?;
        print_success("Created DatabaseSeeder");
    }

    Ok(())
}

async fn create_database_from_config(config_path: &str, verbose: bool) -> Result<(), String> {
    let config = TideConfig::load(config_path)?;

    match config.database.driver_name() {
        "sqlite" => {
            if let Some(path) = config.database.sqlite_file() {
                if verbose {
                    print_info(&format!("Creating SQLite database file: {}", path));
                }
                runtime_db::create_database(&config, &path).await?;
                print_success(&format!("Created SQLite database '{}'", path));
            }
        }
        "postgres" | "mysql" => {
            let database_name = config.database.database.as_deref().ok_or_else(|| {
                "Cannot create the configured server database: no database name is present in the resolved connection URL".to_string()
            })?;
            if verbose {
                print_info(&format!("Creating database: {}", database_name));
            }
            runtime_db::create_database(&config, database_name).await?;
            print_success(&format!("Created database '{}'", database_name));
        }
        _ => {
            return Err(format!(
                "Unsupported database driver: {}",
                config.database.driver
            ));
        }
    }

    Ok(())
}

/// Read a single `KEY=value` entry from an env file, if the file has one.
fn read_env_value(path: &std::path::Path, key: &str) -> Result<Option<String>, String> {
    Ok(crate::config::env_file_values(path)?.remove(key))
}

fn upsert_env_value(path: &std::path::Path, key: &str, value: &str) -> Result<(), String> {
    let entry = format!("{}={}", key, value);
    let existing = if path.exists() {
        std::fs::read_to_string(path)
            .map_err(|error| format!("Failed to read {}: {}", path.display(), error))?
    } else {
        String::new()
    };

    let mut replaced = false;
    let mut lines = Vec::new();
    for line in existing.lines() {
        if crate::config::env_assignment(line).is_some_and(|(existing, _)| existing == key) {
            lines.push(entry.clone());
            replaced = true;
        } else {
            lines.push(line.to_string());
        }
    }

    if !replaced {
        lines.push(entry);
    }

    let mut content = lines.join("\n");
    if !content.ends_with('\n') {
        content.push('\n');
    }

    crate::utils::write_atomically(path, &content)
}

/// The package name for a project directory: its name, snake-cased.
///
/// Cargo refuses a package name that starts with a digit, and the library of one that is
/// a keyword cannot be named in `main.rs`, so either gets an `app_` prefix
/// (`2048-game` is `app_2048_game`).
fn infer_package_name(project_path: &std::path::Path) -> String {
    let name = project_path
        .file_name()
        .and_then(|name| name.to_str())
        .map(crate::utils::to_snake_case)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "tideorm_app".to_string());

    if name.starts_with(|character: char| character.is_ascii_digit())
        || crate::utils::escape_ident(&name) != name
    {
        format!("app_{}", name)
    } else {
        name
    }
}

/// Name the app's binary as `default-run` in a `Cargo.toml` that names none,
/// now that the crate has the seeder runner too: without it, a plain
/// `cargo run` cannot tell which binary to start.
fn ensure_default_run(package_name: &str) -> Result<(), String> {
    let manifest = std::fs::read_to_string("Cargo.toml")
        .map_err(|error| format!("Failed to read Cargo.toml: {}", error))?;
    let parsed: toml::Value = toml::from_str(&manifest)
        .map_err(|error| format!("Failed to parse Cargo.toml: {}", error))?;
    if parsed
        .get("package")
        .and_then(|package| package.get("default-run"))
        .is_some()
    {
        return Ok(());
    }
    let Some(header) = manifest
        .lines()
        .position(|line| line.split('#').next().unwrap_or(line).trim() == "[package]")
    else {
        return Ok(());
    };

    let newline = if manifest.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let entry = format!("default-run = \"{}\"", package_name);
    let mut lines: Vec<&str> = manifest.lines().collect();
    lines.insert(header + 1, &entry);
    let mut updated = lines.join(newline);
    updated.push_str(newline);
    crate::utils::write_atomically(std::path::Path::new("Cargo.toml"), &updated)?;
    print_success(&format!(
        "Set default-run = \"{}\" in Cargo.toml",
        package_name
    ));
    Ok(())
}

#[cfg(test)]
#[path = "../../tests/unit/commands_init_tests.rs"]
mod tests;
