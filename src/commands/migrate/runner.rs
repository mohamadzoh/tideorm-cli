//! The project's migration runner, as the migrate commands call it: the
//! commands it takes and the lines it prints.

use crate::commands::runner::{Role, Runner};
use crate::config::TideConfig;
use colored::Colorize;

/// A migration as the runner reports it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Migration {
    /// `applied`, `pending` or `rolled_back`.
    pub(super) state: String,
    pub(super) version: String,
    name: String,
    pub(super) applied_at: Option<String>,
}

impl Migration {
    /// `<version>_<name>`, the stem of the file `tideorm make migration` writes.
    pub(super) fn file_name(&self) -> String {
        format!("{}_{}", self.version, self.name)
    }
}

/// The project's migration runner, with the database and ledger table this
/// CLI resolved.
pub(crate) struct MigrateRunner {
    runner: Runner,
    table: String,
    url: String,
    /// MySQL and MariaDB commit DDL as it runs.
    mysql: bool,
}

impl MigrateRunner {
    /// The runner of the project `config_path` is in, once it builds.
    pub(crate) fn build(config_path: &str, config: &TideConfig) -> Result<Self, String> {
        Ok(Self {
            url: config.database.try_connection_url()?,
            runner: Runner::build(config_path, Role::Migrate)?,
            table: config.migration.table.clone(),
            mysql: config.database.driver_name() == "mysql",
        })
    }

    /// The registered migrations in version order, each `applied` or
    /// `pending`. Reading them creates the ledger when it is missing.
    pub(crate) fn status(&self) -> Result<Vec<Migration>, String> {
        Ok(parse_migrations(&self.call("status", &[])?))
    }

    /// The last `count` migrations applied, newest first, with when.
    pub(super) fn last_applied(&self, count: usize) -> Result<Vec<Migration>, String> {
        let applied = parse_migrations(&self.call("history", &[])?);
        Ok(applied.into_iter().rev().take(count).collect())
    }

    /// The registered migration `name` picks; see [`pick_migration`].
    pub(super) fn find(&self, name: &str) -> Result<Migration, String> {
        pick_migration(self.status()?, name)
    }

    /// Print the statements `migrations` would run, up or down, without
    /// running them.
    pub(super) fn pretend(&self, migrations: &[Migration], up: bool) -> Result<(), String> {
        let command = if up { "pretend-up" } else { "pretend-down" };
        print!("{}", self.call(command, &versions(migrations))?);
        Ok(())
    }

    /// Run `command`, which applies or reverts migrations, printing each it
    /// did, those done before a failure included, and returning them.
    pub(super) fn change(&self, command: &str, args: &[&str]) -> Result<Vec<Migration>, String> {
        let (stdout, failure) = self.invoke(command, args)?;
        let done = parse_migrations(&stdout);
        for migration in &done {
            let action = if migration.state == "applied" {
                "Migrating"
            } else {
                "Rolling back"
            };
            println!(
                "  {}: {}... {}",
                action,
                migration.file_name(),
                "DONE".green()
            );
        }
        match failure {
            None => Ok(done),
            // MySQL and MariaDB cannot roll back the DDL of a migration whose
            // ledger write failed, which every later run then trips over.
            Some(error) if self.mysql => Err(format!(
                "Migration failed: {error}\nMySQL/MariaDB commit DDL implicitly, so the schema change may have been applied without the migration ledger being updated. If so, reconcile it with `tideorm migrate mark --migration <name>` (add --unmark for a rollback)."
            )),
            Some(error) => Err(format!("Migration failed: {error}")),
        }
    }

    /// Run `command` with `args`, returning what it printed.
    pub(super) fn call(&self, command: &str, args: &[&str]) -> Result<String, String> {
        match self.invoke(command, args)? {
            (stdout, None) => Ok(stdout),
            (_, Some(error)) => Err(error),
        }
    }

    /// Run `command` with `args`: what it printed, and why it failed if it
    /// did. What it wrote to standard error on success is passed on.
    fn invoke(&self, command: &str, args: &[&str]) -> Result<(String, Option<String>), String> {
        let output = self
            .runner
            .command()
            .env("DATABASE_URL", &self.url)
            .arg(&self.table)
            .arg(command)
            .args(args)
            .output()
            .map_err(|error| format!("Failed to run cargo: {}", error))?;
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if output.status.success() {
            if !stderr.is_empty() {
                eprintln!("{stderr}");
            }
            return Ok((stdout, None));
        }
        let error = if stderr.is_empty() {
            format!("The migration runner failed ({})", output.status)
        } else {
            stderr
        };
        Ok((stdout, Some(error)))
    }
}

/// The versions of `migrations`, which the runner names them by.
pub(super) fn versions(migrations: &[Migration]) -> Vec<&str> {
    migrations
        .iter()
        .map(|migration| migration.version.as_str())
        .collect()
}

/// The migrations the runner's lines report: `<state>\t<version>\t<name>`,
/// then when it was applied for `history`. Other lines are not its own.
fn parse_migrations(output: &str) -> Vec<Migration> {
    output
        .lines()
        .filter_map(|line| {
            let mut fields = line.split('\t');
            let state = fields
                .next()
                .filter(|state| ["applied", "pending", "rolled_back"].contains(state))?;
            Some(Migration {
                state: state.to_string(),
                version: fields.next()?.to_string(),
                name: fields.next()?.to_string(),
                applied_at: fields.next().map(str::to_string),
            })
        })
        .collect()
}

/// The migration of `migrations` that `name` picks.
///
/// An exact file name, version or logical name always wins. Otherwise a
/// substring match is accepted only when it identifies exactly one migration -
/// an ambiguous name is rejected instead of silently picking the first match.
fn pick_migration(migrations: Vec<Migration>, name: &str) -> Result<Migration, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Migration name must not be empty".to_string());
    }

    let exact = |migration: &Migration| {
        migration.file_name() == name || migration.version == name || migration.name == name
    };
    let mut matches: Vec<Migration> = if migrations.iter().any(exact) {
        migrations.into_iter().filter(exact).collect()
    } else {
        migrations
            .into_iter()
            .filter(|migration| migration.file_name().contains(name))
            .collect()
    };

    match matches.len() {
        0 => Err(format!("Migration not found: {}", name)),
        1 => Ok(matches.remove(0)),
        _ => {
            let candidates = matches
                .iter()
                .map(|migration| format!("  - {}", migration.file_name()))
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

#[cfg(test)]
#[path = "../../../tests/unit/commands_migrate_runner_tests.rs"]
mod tests;
