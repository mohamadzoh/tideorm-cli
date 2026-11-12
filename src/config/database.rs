//! The `[database]` section of `tideorm.toml` and the connection URLs it resolves to.

use super::env::{
    env_source, expand_env_vars_with_sources, is_lone_placeholder, lookup_env,
    unresolved_placeholders,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tideorm::prelude::{MySqlConnectOptions, PgConnectOptions, SqliteConnectOptions};
use url::Url;

/// Database configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseConfig {
    /// Database driver (postgres, mysql, sqlite)
    #[serde(default = "default_driver")]
    pub driver: String,

    /// Database host
    #[serde(default = "default_host")]
    pub host: String,

    /// Database port
    #[serde(default)]
    pub port: Option<u16>,

    /// Database name
    #[serde(default)]
    pub database: Option<String>,

    /// Database username
    #[serde(default)]
    pub username: Option<String>,

    /// Database password
    #[serde(default)]
    pub password: Option<String>,

    /// Connection URL (overrides individual settings)
    #[serde(default)]
    pub url: Option<String>,

    /// SQLite database path
    #[serde(default)]
    pub sqlite_path: Option<String>,

    /// Where the connection URL came from, for `db status`; none when the
    /// declarative settings make it.
    #[serde(skip)]
    pub url_source: Option<String>,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            driver: default_driver(),
            host: default_host(),
            port: None,
            database: None,
            username: None,
            password: None,
            url: None,
            sqlite_path: None,
            url_source: None,
        }
    }
}

fn default_driver() -> String {
    "postgres".to_string()
}

fn default_host() -> String {
    "localhost".to_string()
}

impl DatabaseConfig {
    /// The configured driver's canonical name, or its own name when the CLI
    /// does not know it.
    pub(crate) fn driver_name(&self) -> &str {
        canonical_driver(&self.driver).unwrap_or(&self.driver)
    }

    /// Build connection URL from configuration
    ///
    /// This never fails: a driver the CLI does not recognise is rendered with its own name
    /// as the URL scheme so that read-only callers (`tideorm config`) keep working, and the
    /// driver is reported by whoever tries to open a connection. Use
    /// [`DatabaseConfig::try_connection_url`] to surface the unsupported driver directly.
    pub fn connection_url(&self) -> String {
        self.try_connection_url()
            .unwrap_or_else(|_| self.server_url(&self.driver, None, "", None))
    }

    /// Build connection URL from configuration, rejecting unsupported drivers.
    pub fn try_connection_url(&self) -> Result<String, String> {
        if let Some(url) = &self.url {
            // Replace environment variables in URL
            return Ok(self.expand_env_vars(url));
        }

        if self.driver_name() == "sqlite" {
            let path = self.sqlite_path.as_deref().unwrap_or("database.db");
            return Ok(sqlite_connection_url(path));
        }

        match driver_url_defaults(&self.driver) {
            Some((scheme, port, user)) => Ok(self.server_url(scheme, Some(port), user, None)),
            None => Err(format!("Unsupported database driver: {}", self.driver)),
        }
    }

    /// The file the configured SQLite database lives in, or `None` for an in-memory one.
    ///
    /// A connection URL is what the connection opens, so its file wins over `sqlite_path`:
    /// creating, inspecting or dropping any other file would act on the wrong database.
    pub fn sqlite_file(&self) -> Option<String> {
        let url = self.url.as_deref().map(|url| self.expand_env_vars(url));
        match url.as_deref().and_then(sqlite_url_target) {
            Some(Ok(SqliteTarget::File(path))) => Some(path),
            // `load` refuses a URL the driver cannot read; acting on no file is safe.
            Some(Ok(SqliteTarget::Memory) | Err(_)) => None,
            None => Some(
                self.sqlite_path
                    .clone()
                    .unwrap_or_else(|| "database.db".to_string()),
            ),
        }
    }

    /// The URL of `database` on the configured server, for the statements that must run
    /// outside the target database: `db create` and `db drop` connect to the server's own
    /// database (`postgres`, `mysql`) to create or drop it.
    pub(crate) fn server_url_for_database(&self, database: &str) -> Result<String, String> {
        let (scheme, port, user) = driver_url_defaults(&self.driver)
            .ok_or_else(|| format!("Unsupported database driver: {}", self.driver))?;
        Ok(self.server_url(scheme, Some(port), user, Some(database)))
    }

    /// Render the URL of `database`, the configured one by default, on the
    /// configured server; see [`server_url`].
    fn server_url(
        &self,
        scheme: &str,
        default_port: Option<u16>,
        default_user: &str,
        database: Option<&str>,
    ) -> String {
        server_url(
            scheme,
            self.username.as_deref().unwrap_or(default_user),
            self.password.as_deref().unwrap_or(""),
            &self.host,
            self.port.or(default_port),
            database.or(self.database.as_deref()).unwrap_or("tideorm"),
        )
    }

    /// Expand environment variables in a string
    fn expand_env_vars(&self, s: &str) -> String {
        expand_env_vars_with_sources(s, &HashMap::new())
    }

    pub(super) fn resolve_with_env(
        &mut self,
        env_values: &HashMap<String, String>,
    ) -> Result<(), String> {
        // Everything downstream matches on "postgres" and "mysql", so the
        // aliases are folded here; MariaDB takes MySQL's DDL.
        self.driver = normalize_driver(&expand_env_vars_with_sources(&self.driver, env_values));
        self.host = expand_env_vars_with_sources(&self.host, env_values);
        self.database = self
            .database
            .as_ref()
            .map(|value| expand_env_vars_with_sources(value, env_values));
        self.username = self
            .username
            .as_ref()
            .map(|value| expand_env_vars_with_sources(value, env_values));
        self.password = self
            .password
            .as_ref()
            .map(|value| expand_env_vars_with_sources(value, env_values));
        self.sqlite_path = self
            .sqlite_path
            .as_ref()
            .map(|value| expand_env_vars_with_sources(value, env_values));

        self.url = match &self.url {
            Some(value) => {
                let url = expand_env_vars_with_sources(value, env_values);
                let unresolved = unresolved_placeholders(&url);
                if is_lone_placeholder(&url) {
                    // A placeholder nothing defines - `${DATABASE_URL}` in a fresh clone that
                    // has no `.env` yet - is no URL: the declarative settings stay in force
                    // instead of being replaced by the placeholder's literal text.
                    None
                } else if !unresolved.is_empty() {
                    // Part of a URL left unresolved would connect somewhere else: to the
                    // declarative settings' database, or the defaults'.
                    return Err(format!(
                        "The database URL in tideorm.toml refers to {}, which neither the \
                         environment nor the env file sets",
                        unresolved.join(", ")
                    ));
                } else {
                    self.url_source = match unresolved_placeholders(value)[..] {
                        [name] if is_lone_placeholder(value) => env_source(name, env_values),
                        _ => Some("database.url in tideorm.toml".to_string()),
                    };
                    Some(url)
                }
            }
            None => {
                self.url_source = env_source("DATABASE_URL", env_values);
                lookup_env("DATABASE_URL", env_values)
            }
        }
        .filter(|url| !url.trim().is_empty());
        if self.url.is_none() {
            self.url_source = None;
        }

        if let Some(url) = self.url.clone() {
            self.apply_connection_url(&url)?;
        }
        Ok(())
    }

    /// Take the driver, server, database and credentials from `url`, read by the
    /// driver that opens it, so what the CLI shows, creates and drops is what a
    /// connection uses. A scheme the CLI does not know sets only the driver name,
    /// which connecting reports.
    pub(super) fn apply_connection_url(&mut self, url: &str) -> Result<(), String> {
        let normalized = expand_env_vars_with_sources(url, &HashMap::new());
        let Some((scheme, rest)) = normalized.split_once(':') else {
            return Ok(());
        };
        let Some(driver) = canonical_driver(scheme) else {
            if rest.starts_with("//") {
                self.driver = normalize_driver(scheme);
                self.url = Some(normalized.clone());
            }
            return Ok(());
        };

        match driver {
            "sqlite" => {
                self.sqlite_path = match sqlite_url_target(&normalized) {
                    Some(Ok(SqliteTarget::File(path))) => Some(path),
                    Some(Err(error)) => return Err(error),
                    Some(Ok(SqliteTarget::Memory)) | None => None,
                };
                self.host.clear();
                self.port = None;
                self.database = None;
                self.username = None;
                self.password = None;
            }
            "postgres" => {
                let options: PgConnectOptions = normalized
                    .parse()
                    .map_err(|error| invalid_url(driver, error))?;
                self.set_server(
                    options.get_host(),
                    options.get_port(),
                    options.get_database(),
                    options.get_username(),
                    options.get_password(),
                );
            }
            _ => {
                let options: MySqlConnectOptions = normalized
                    .parse()
                    .map_err(|error| invalid_url(driver, error))?;
                self.set_server(
                    options.get_host(),
                    options.get_port(),
                    options.get_database(),
                    options.get_username(),
                    options.get_password(),
                );
            }
        }
        self.driver = driver.to_string();
        self.url = Some(normalized.clone());
        Ok(())
    }

    fn set_server(
        &mut self,
        host: &str,
        port: u16,
        database: Option<&str>,
        username: &str,
        password: Option<&str>,
    ) {
        // The URL keeps an IPv6 host's brackets; `server_url` puts them back.
        self.host = host.trim_matches(['[', ']']).to_string();
        self.port = Some(port);
        self.database = database.map(str::to_string);
        self.username = Some(username.to_string());
        self.password = password.map(str::to_string);
        self.sqlite_path = None;
    }
}

/// The error for a URL its driver cannot read. The URL is left out: it may hold
/// a password.
fn invalid_url(driver: &str, error: impl std::fmt::Display) -> String {
    format!("The database URL is not a valid {driver} URL: {error}")
}

/// The driver a configured driver name or URL scheme stands for, in any case:
/// `postgresql` is `postgres`, and `mariadb` is `mysql`, whose protocol it speaks and
/// whose DDL it takes. `None` for a driver the CLI does not know.
///
/// Every path that derives a driver goes through this, so `mariadb://` or `MySQL` in one
/// place cannot mean SQLite in another.
pub(crate) fn canonical_driver(name: &str) -> Option<&'static str> {
    match name.trim().to_ascii_lowercase().as_str() {
        "postgres" | "postgresql" => Some("postgres"),
        "mysql" | "mariadb" => Some("mysql"),
        "sqlite" => Some("sqlite"),
        _ => None,
    }
}

/// [`canonical_driver`], keeping an unknown driver's name (lower-cased) so that whoever
/// opens a connection can report it.
pub(crate) fn normalize_driver(name: &str) -> String {
    canonical_driver(name).map_or_else(|| name.trim().to_ascii_lowercase(), str::to_string)
}

/// Connection-URL defaults (scheme, port, username) for the drivers the CLI can build a
/// URL for.
///
/// MariaDB speaks the MySQL wire protocol and has no scheme of its own, so it maps onto
/// MySQL's defaults rather than being rejected.
fn driver_url_defaults(driver: &str) -> Option<(&'static str, u16, &'static str)> {
    match canonical_driver(driver) {
        Some("postgres") => Some(("postgres", 5432, "postgres")),
        Some("mysql") => Some(("mysql", 3306, "root")),
        _ => None,
    }
}

/// What a `sqlite:` connection URL opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SqliteTarget {
    /// A database file, as a path the CLI can open.
    File(String),
    /// An in-memory or temporary database, which has no file.
    Memory,
}

/// The database a `sqlite:` URL opens, as the driver reads it; `None` for any
/// other scheme, and the driver's reason for a URL it refuses.
///
/// `:memory:`, an empty path and `mode=memory` open no file. On Windows SQLite drops
/// the slash in front of a drive letter, so `sqlite:///C:/data/app.db` is
/// `C:/data/app.db`.
pub fn sqlite_url_target(url: &str) -> Option<Result<SqliteTarget, String>> {
    Some(sqlite_url_options(url)?.map(|options| {
        if options.is_transient() {
            return SqliteTarget::Memory;
        }
        let path = options.get_filename().to_string_lossy().into_owned();
        SqliteTarget::File(match path.strip_prefix('/') {
            Some(rest) if cfg!(windows) && starts_with_drive_letter(rest) => rest.to_string(),
            _ => path,
        })
    }))
}

/// A `sqlite:` URL read by the driver; `None` for any other scheme.
fn sqlite_url_options(url: &str) -> Option<Result<SqliteConnectOptions, String>> {
    let (scheme, _) = url.split_once(':')?;
    (canonical_driver(scheme) == Some("sqlite"))
        .then(|| url.parse().map_err(|error| invalid_url("sqlite", error)))
}

/// Whether `path` starts with a Windows drive such as `C:`.
fn starts_with_drive_letter(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

/// Render `scheme://[user[:password]@]host[:port]/database` with the URL library the
/// drivers read it with, which percent-encodes the credentials and the database name.
///
/// An IPv6 host is bracketed, or its colons would read as a port.
pub(crate) fn server_url(
    scheme: &str,
    username: &str,
    password: &str,
    host: &str,
    port: Option<u16>,
    database: &str,
) -> String {
    let host = if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_string()
    };
    let Ok(mut url) = Url::parse(&format!("{scheme}://{host}")) else {
        // Not a URL any driver reads; connecting reports it.
        return format!("{scheme}://{host}/{database}");
    };
    // These fail only for a URL without a host, which `Url::parse` gave none of.
    let _ = url.set_port(port);
    let _ = url.set_username(username);
    let _ = url.set_password((!password.is_empty()).then_some(password));
    url.set_path(&format!("/{database}"));
    url.into()
}

/// The `sqlite:` URL that opens the database file at `path`.
///
/// `%` and `?` are escaped, since the driver reads the URL back percent-decoded and cuts it at
/// the first `?`: a file name holding either would otherwise open a different file. A
/// space is escaped too: TideORM parses the URL first and refuses one in a host, and
/// dotenvy refuses an unquoted one in the `.env` that `tideorm init` writes the URL to.
pub(crate) fn sqlite_connection_url(path: &str) -> String {
    let normalized = path
        .replace('\\', "/")
        .replace('%', "%25")
        .replace('?', "%3F")
        .replace(' ', "%20");
    if starts_with_drive_letter(&normalized) && normalized[2..].starts_with('/') {
        format!("sqlite:///{}", normalized)
    } else {
        format!("sqlite://{}", normalized)
    }
}
