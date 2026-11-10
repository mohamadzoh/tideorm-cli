//! The project binaries the CLI runs: seeders and migrations are project
//! code, so only the project itself can run them.

use crate::utils::package_name;
use std::path::{Path, PathBuf};

/// What a runner binary runs.
#[derive(Clone, Copy)]
pub(crate) enum Role {
    /// `seeders::all()`, for `tideorm db seed`.
    Seed,
    /// `migrations::all()`, for `tideorm migrate`.
    Migrate,
}

impl Role {
    /// The binary target, named after the package, so two projects installed
    /// with `cargo install` do not collide on it.
    pub(crate) fn bin(self, package: &str) -> String {
        match self {
            Role::Seed => format!("{package}-seed"),
            Role::Migrate => format!("{package}-migrate"),
        }
    }

    /// Where `tideorm init` scaffolds the binary, relative to the project root.
    pub(crate) fn path(self, package: &str) -> String {
        format!("src/bin/{}.rs", self.bin(package))
    }

    /// How to add the runner to a project `tideorm init` did not scaffold.
    pub(crate) fn setup_hint(self) -> &'static str {
        match self {
            Role::Seed => {
                "`tideorm db seed` runs your seeders through src/bin/<package>-seed.rs, a binary named after your package that connects TideORM and runs `seeders::all()`. Projects created by `tideorm init` include it; see \"Seeding\" in the tideorm-cli README to add it to this one."
            }
            Role::Migrate => {
                "`tideorm migrate` runs your migrations through src/bin/<package>-migrate.rs, a binary named after your package that connects TideORM and runs `migrations::all()`. Projects created by `tideorm init` include it; see \"Migrations\" in the tideorm-cli README to add it to this one."
            }
        }
    }

    fn noun(self) -> &'static str {
        match self {
            Role::Seed => "seeder",
            Role::Migrate => "migration",
        }
    }

    /// The runner's source, for the library crate `library`.
    pub(crate) fn source(self, library: &str) -> String {
        match self {
            Role::Seed => SEED_RUNNER,
            Role::Migrate => MIGRATE_RUNNER,
        }
        .replace("{library}", library)
    }
}

/// A project's runner, once it is known to build.
pub(crate) struct Runner {
    pub(crate) manifest: PathBuf,
    pub(crate) bin: String,
}

impl Runner {
    /// Build the project's `role` runner. Commands check this before a
    /// destructive step, so a project without a runner, or whose runner does
    /// not compile, is refused before its tables are dropped.
    pub(crate) fn build(config_path: &str, role: Role) -> Result<Self, String> {
        let noun = role.noun();
        let root = Path::new(config_path)
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let manifest = root.join("Cargo.toml");
        let package = package_name(&manifest)
            .map_err(|reason| format!("No {noun} runner: {reason}. {}", role.setup_hint()))?;

        let runner_path = root.join(role.path(&package));
        if !runner_path.is_file() {
            return Err(format!(
                "No {noun} runner at {}. {}",
                runner_path.display(),
                role.setup_hint()
            ));
        }

        let bin = role.bin(&package);
        let status = std::process::Command::new("cargo")
            .args(["build", "--quiet", "--manifest-path"])
            .arg(&manifest)
            .args(["--bin", bin.as_str()])
            .status()
            .map_err(|error| format!("Failed to run cargo: {}", error))?;
        if !status.success() {
            return Err(format!(
                "The {noun} runner does not build ({status}); nothing was changed."
            ));
        }
        Ok(Self { manifest, bin })
    }

    /// `cargo run` of the runner, before its own arguments. `--manifest-path`
    /// keeps the working directory, so a relative SQLite path means the same
    /// file to the runner as to the rest of this CLI.
    pub(crate) fn command(&self) -> std::process::Command {
        let mut command = std::process::Command::new("cargo");
        command
            .args(["run", "--quiet", "--manifest-path"])
            .arg(&self.manifest)
            .args(["--bin", self.bin.as_str(), "--"]);
        command
    }
}

const SEED_RUNNER: &str = r#"//! Seeder runner: `tideorm db seed` runs this binary.
//!
//! With no argument it runs every seeder in `seeders::all()` that has not run
//! yet; given a seeder name, it runs that seeder again. `--list` prints the
//! seeders' names, which `tideorm` checks a name against before it changes
//! anything.

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let seeders = {library}::seeders::all();
    let name = std::env::args().nth(1);
    if name.as_deref() == Some("--list") {
        for seed in seeders.names() {
            println!("{seed}");
        }
        return Ok(());
    }

    // Without the application's startup work: a `force_sync` there would drop
    // the tables before the seeders fill them.
    {library}::config::tideorm_config().connect_only().await?;
    let outcome = match name {
        Some(name) => Ok(seeders.run_seed(&name).await?),
        None => seeders.run().await,
    };
    // A failed run still lists the seeders it ran before the failure.
    let (result, outcome) = match outcome {
        Ok(result) => (result, Ok(())),
        Err(failure) => (*failure.result, Err(failure.error)),
    };

    for seed in &result.executed {
        println!("  ran      {}", seed.name);
    }
    for seed in &result.skipped {
        println!("  skipped  {} (already ran)", seed.name);
    }
    outcome?;
    Ok(())
}
"#;

const MIGRATE_RUNNER: &str = r#"//! Migration runner: `tideorm migrate` runs this binary.
//!
//! It runs `migrations::all()`, recording them in the ledger table its first
//! argument names, and prints what a command did as tab-separated lines that
//! `tideorm` reads back: `applied`, `pending` or `rolled_back`, then the
//! version and the name.

use std::process::ExitCode;
use tideorm::prelude::*;

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

async fn run(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let [table, command, names @ ..] = args else {
        return Err("usage: <ledger table> <command> [migration ..]".into());
    };
    let steps = || -> Result<usize, Box<dyn std::error::Error>> {
        Ok(names.first().ok_or("missing the number of steps")?.parse()?)
    };
    // Without the application's startup work, which would sync, migrate or
    // seed before this command runs.
    {library}::config::tideorm_config().connect_only().await?;
    let migrator = {library}::migrations::all().migrations_table(table.as_str());

    match command.as_str() {
        "run" => report(migrator.run().await)?,
        "run-steps" => report(migrator.run_steps(steps()?).await)?,
        "rollback-steps" => report(migrator.rollback_steps(steps()?).await)?,
        "reset" => report(migrator.reset().await)?,
        "refresh" => report(migrator.refresh().await)?,
        "up" => {
            for name in names {
                report(Ok(migrator.run_one(name).await?))?;
            }
        }
        "down" => {
            for name in names {
                report(Ok(migrator.rollback_one(name).await?))?;
            }
        }
        "mark" | "unmark" => {
            for name in names {
                migrator.mark(name, command == "mark").await?;
            }
        }
        "pretend-up" | "pretend-down" => {
            let up = command == "pretend-up";
            for name in names {
                println!("\n-- Migration: {name}\n-- {}:", if up { "Up" } else { "Down" });
                for statement in migrator.pretend(name, up).await? {
                    println!("{statement};");
                }
            }
        }
        "status" => {
            for migration in migrator.status().await? {
                let state = if migration.applied { "applied" } else { "pending" };
                println!("{state}\t{}\t{}", migration.version, migration.name);
            }
        }
        "history" => {
            for migration in migrator.applied().await? {
                println!(
                    "applied\t{}\t{}\t{}",
                    migration.version, migration.name, migration.applied_at
                );
            }
        }
        other => return Err(format!("unknown command `{other}`").into()),
    }
    Ok(())
}

/// Print what a command rolled back and applied, a failed one's work before
/// its failure included, and pass the failure on.
fn report(outcome: Result<MigrationResult, MigrationFailure>) -> tideorm::Result<()> {
    let (result, outcome) = match outcome {
        Ok(result) => (result, Ok(())),
        Err(failure) => (*failure.result, Err(failure.error)),
    };
    for migration in &result.rolled_back {
        println!("rolled_back\t{}\t{}", migration.version, migration.name);
    }
    for migration in &result.applied {
        println!("applied\t{}\t{}", migration.version, migration.name);
    }
    outcome
}
"#;
