//! The answers `tideorm init` collects: from the prompts, from `TIDEORM_PROMPT_SCRIPT`,
//! or the defaults of a non-interactive run.

use super::read_env_value;
use crate::config::{server_url, sqlite_connection_url};
use crate::utils::{confirm, file_exists, print_info};
use colored::Colorize;
use dialoguer::{Input, Password};
use std::collections::VecDeque;
use std::io::IsTerminal;
use std::sync::{LazyLock, Mutex};

static PROMPT_SCRIPT: LazyLock<Mutex<Option<VecDeque<String>>>> =
    LazyLock::new(|| Mutex::new(None));

#[derive(Debug, Clone)]
pub(super) struct InitOptions {
    pub(super) env_file_name: String,
    pub(super) overwrite_config: bool,
    pub(super) database: DatabaseInit,
    /// Whether a `DATABASE_URL` already present in the env file may be replaced.
    pub(super) replace_database_url: bool,
    pub(super) create_database_now: bool,
    pub(super) run_migrations_now: bool,
}

#[derive(Debug, Clone)]
pub(super) enum DatabaseInit {
    Sqlite {
        path: String,
    },
    Postgres {
        host: String,
        port: u16,
        database: String,
        username: String,
        password: String,
    },
    MySql {
        host: String,
        port: u16,
        database: String,
        username: String,
        password: String,
    },
}

impl DatabaseInit {
    fn driver(&self) -> &'static str {
        match self {
            Self::Sqlite { .. } => "sqlite",
            Self::Postgres { .. } => "postgres",
            Self::MySql { .. } => "mysql",
        }
    }

    fn display_name(&self) -> &'static str {
        database_label(self.driver())
    }

    pub(super) fn connection_url(&self) -> String {
        match self {
            Self::Sqlite { path } => sqlite_connection_url(path),
            Self::Postgres {
                host,
                port,
                database,
                username,
                password,
            } => server_url("postgres", username, password, host, Some(*port), database),
            Self::MySql {
                host,
                port,
                database,
                username,
                password,
            } => server_url("mysql", username, password, host, Some(*port), database),
        }
    }
}

/// The setup answers for `database`, a name
/// [`normalize_database`](super::normalize_database) returned.
pub(super) fn collect_init_options(database: &str, verbose: bool) -> Result<InitOptions, String> {
    reset_prompt_script();

    // `RUST_TEST_THREADS` is exported globally by plenty of developers, and treating it as
    // "this is a test run" silently turned a real interactive `init` into all-defaults.
    // Only an actual test build, or an explicit opt-out, counts.
    let running_under_rust_tests = cfg!(test);
    let forced_noninteractive =
        std::env::var_os("TIDEORM_NONINTERACTIVE").is_some() || std::env::var_os("CI").is_some();
    let interactive = has_prompt_script()
        || (!running_under_rust_tests
            && !forced_noninteractive
            && std::io::stdin().is_terminal()
            && std::io::stdout().is_terminal());

    if interactive {
        print_setup_intro(database);
    }

    let env_file_name = if interactive {
        prompt_text("Environment file", ".env")?
    } else {
        ".env".to_string()
    };

    let overwrite_config = if file_exists("tideorm.toml") {
        if interactive {
            prompt_confirm("Found tideorm.toml. Replace it with the new setup answers?")?
        } else {
            false
        }
    } else {
        true
    };

    let database = match database {
        "sqlite" => collect_sqlite_options(interactive)?,
        "mysql" => collect_mysql_options(interactive)?,
        "postgres" => collect_postgres_options(interactive)?,
        _ => unreachable!("normalize_database only returns supported drivers"),
    };

    // `tideorm init` is routinely re-run inside an existing checkout, and the env file has
    // no backup: an already-configured DATABASE_URL is only replaced when the user says
    // so, and never in a non-interactive run.
    let env_path = std::path::Path::new(&env_file_name);
    let existing_database_url = read_env_value(env_path, "DATABASE_URL")?;
    let replace_database_url = match existing_database_url {
        Some(existing) if !existing.trim().is_empty() && existing != database.connection_url() => {
            if interactive {
                prompt_confirm(&format!(
                    "{} already defines DATABASE_URL. Replace it with the answers above?",
                    env_file_name
                ))?
            } else {
                false
            }
        }
        _ => true,
    };

    let create_database_now = match &database {
        DatabaseInit::Sqlite { .. } => true,
        _ if interactive => {
            let use_existing_database = prompt_confirm(&format!(
                "Use an existing {} database?",
                database.display_name()
            ))?;
            if use_existing_database {
                false
            } else {
                prompt_confirm("Create the database during setup?")?
            }
        }
        _ => false,
    };

    let run_migrations_now = if interactive {
        prompt_confirm("Run pending migrations during setup?")?
    } else {
        false
    };

    if verbose {
        print_info(&format!(
            "Using environment file {} with database driver {}",
            env_file_name,
            database.driver()
        ));
    }

    Ok(InitOptions {
        env_file_name,
        overwrite_config,
        database,
        replace_database_url,
        create_database_now,
        run_migrations_now,
    })
}

fn collect_sqlite_options(interactive: bool) -> Result<DatabaseInit, String> {
    let path = if interactive {
        prompt_text("SQLite file path", "database.db")?
    } else {
        "database.db".to_string()
    };

    Ok(DatabaseInit::Sqlite { path })
}

fn collect_postgres_options(interactive: bool) -> Result<DatabaseInit, String> {
    let host = if interactive {
        prompt_text("Postgres host", "localhost")?
    } else {
        "localhost".to_string()
    };
    let port = if interactive {
        prompt_u16("Postgres port", 5432)?
    } else {
        5432
    };
    let database = if interactive {
        prompt_text("Postgres database", "tideorm_db")?
    } else {
        "tideorm_db".to_string()
    };
    let username = if interactive {
        prompt_text("Postgres user", "postgres")?
    } else {
        "postgres".to_string()
    };
    let password = if interactive {
        prompt_password("Postgres password (leave blank to skip)")?
    } else {
        String::new()
    };

    Ok(DatabaseInit::Postgres {
        host,
        port,
        database,
        username,
        password,
    })
}

fn collect_mysql_options(interactive: bool) -> Result<DatabaseInit, String> {
    let host = if interactive {
        prompt_text("MySQL host", "localhost")?
    } else {
        "localhost".to_string()
    };
    let port = if interactive {
        prompt_u16("MySQL port", 3306)?
    } else {
        3306
    };
    let database = if interactive {
        prompt_text("MySQL database", "tideorm_db")?
    } else {
        "tideorm_db".to_string()
    };
    let username = if interactive {
        prompt_text("MySQL user", "root")?
    } else {
        "root".to_string()
    };
    let password = if interactive {
        prompt_password("MySQL password (leave blank to skip)")?
    } else {
        String::new()
    };

    Ok(DatabaseInit::MySql {
        host,
        port,
        database,
        username,
        password,
    })
}

fn prompt_text(prompt: &str, default_value: &str) -> Result<String, String> {
    if let Some(value) = next_prompt_script_value()? {
        return Ok(if value.is_empty() {
            default_value.to_string()
        } else {
            value
        });
    }

    Input::<String>::new()
        .with_prompt(prompt)
        .default(default_value.to_string())
        .interact_text()
        .map_err(|error| format!("Failed to read {}: {}", prompt, error))
}

fn prompt_u16(prompt: &str, default_value: u16) -> Result<u16, String> {
    if let Some(value) = next_prompt_script_value()? {
        if value.is_empty() {
            return Ok(default_value);
        }

        return value
            .parse::<u16>()
            .map_err(|error| format!("Failed to parse {}: {}", prompt, error));
    }

    Input::<u16>::new()
        .with_prompt(prompt)
        .default(default_value)
        .interact_text()
        .map_err(|error| format!("Failed to read {}: {}", prompt, error))
}

fn prompt_password(prompt: &str) -> Result<String, String> {
    if let Some(value) = next_prompt_script_value()? {
        return Ok(value);
    }

    Password::new()
        .with_prompt(prompt)
        .allow_empty_password(true)
        .interact()
        .map_err(|error| format!("Failed to read {}: {}", prompt, error))
}

fn prompt_confirm(prompt: &str) -> Result<bool, String> {
    if let Some(value) = next_prompt_script_value()? {
        return parse_prompt_confirm(prompt, &value);
    }

    Ok(confirm(prompt))
}

fn parse_prompt_confirm(prompt: &str, value: &str) -> Result<bool, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "" | "n" | "no" | "false" | "0" => Ok(false),
        "y" | "yes" | "true" | "1" => Ok(true),
        other => Err(format!("Failed to parse {} response: {}", prompt, other)),
    }
}

/// Whether the scripted-answer test hook is active.
///
/// `TIDEORM_PROMPT_SCRIPT` short-circuits every interactivity check, so it is confined to
/// builds with debug assertions on (which is what the test binaries are). A released
/// binary ignores the variable entirely.
fn has_prompt_script() -> bool {
    cfg!(debug_assertions) && std::env::var_os("TIDEORM_PROMPT_SCRIPT").is_some()
}

/// Drop any answers left over from a previous `run()` in this process.
///
/// The queue is filled lazily from the environment and never emptied on its own, so a
/// second in-process `init` would otherwise fail with "ran out of answers".
fn reset_prompt_script() {
    if let Ok(mut script) = PROMPT_SCRIPT.lock() {
        *script = None;
    }
}

fn next_prompt_script_value() -> Result<Option<String>, String> {
    if !has_prompt_script() {
        return Ok(None);
    }

    let mut script = PROMPT_SCRIPT
        .lock()
        .map_err(|_| "Prompt script lock poisoned".to_string())?;
    if script.is_none() {
        let raw = std::env::var("TIDEORM_PROMPT_SCRIPT").unwrap_or_default();
        let values = raw
            .split('\n')
            .map(|value| value.trim_end_matches('\r').to_string())
            .collect::<VecDeque<_>>();
        *script = Some(values);
    }

    match script.as_mut().and_then(|values| values.pop_front()) {
        Some(value) => Ok(Some(value)),
        None => Err("TIDEORM_PROMPT_SCRIPT ran out of answers".to_string()),
    }
}

fn print_setup_intro(database: &str) {
    println!("\n{}", "Setup".cyan().bold());
    println!(
        "{}",
        format!(
            "Answer a few questions to configure your {} project. Press Enter to accept the default value.",
            database_label(database)
        )
        .dimmed()
    );
}

fn database_label(database: &str) -> &'static str {
    match database {
        "sqlite" => "SQLite",
        "mysql" => "MySQL",
        _ => "Postgres",
    }
}
