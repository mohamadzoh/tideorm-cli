//! Configuration module for TideORM CLI
//!
//! Handles loading and parsing of tideorm.toml configuration files.

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::LazyLock;

/// A `${VAR}` or `$VAR` placeholder; the first or the second group names the variable.
static PLACEHOLDER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\$\{([^}]+)\}|\$([A-Z_][A-Z0-9_]*)").unwrap());

/// TideORM CLI Configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TideConfig {
    /// Project configuration
    #[serde(default)]
    pub project: ProjectConfig,

    /// Database configuration
    #[serde(default)]
    pub database: DatabaseConfig,

    /// Paths configuration
    #[serde(default)]
    pub paths: PathsConfig,

    /// Migration configuration
    #[serde(default)]
    pub migration: MigrationConfig,

    /// Seeder configuration
    #[serde(default)]
    pub seeder: SeederConfig,

    /// Model generation configuration
    #[serde(default)]
    pub model: ModelGenConfig,
}

/// Project configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectConfig {
    /// Project name
    #[serde(default = "default_project_name")]
    pub name: String,

    /// Environment (development, production, test)
    #[serde(default = "default_environment")]
    pub environment: String,

    /// Environment file used for variable expansion
    #[serde(default = "default_env_file")]
    pub env_file: String,
}

impl Default for ProjectConfig {
    fn default() -> Self {
        Self {
            name: default_project_name(),
            environment: default_environment(),
            env_file: default_env_file(),
        }
    }
}

fn default_project_name() -> String {
    "tideorm-project".to_string()
}

fn default_environment() -> String {
    std::env::var("TIDEORM_ENV").unwrap_or_else(|_| "development".to_string())
}

fn default_env_file() -> String {
    ".env".to_string()
}

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

    /// Connection pool size
    #[serde(default = "default_pool_size")]
    pub pool_size: u32,

    /// Connection timeout in seconds
    #[serde(default = "default_timeout")]
    pub timeout: u64,
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
            pool_size: default_pool_size(),
            timeout: default_timeout(),
        }
    }
}

fn default_driver() -> String {
    "postgres".to_string()
}

fn default_host() -> String {
    "localhost".to_string()
}

fn default_pool_size() -> u32 {
    5
}

fn default_timeout() -> u64 {
    30
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
            Some(SqliteTarget::File(path)) => Some(path),
            Some(SqliteTarget::Memory) => None,
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

    fn resolve_with_env(&mut self, env_values: &HashMap<String, String>) -> Result<(), String> {
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
                    Some(url)
                }
            }
            None => lookup_env("DATABASE_URL", env_values),
        }
        .filter(|url| !url.trim().is_empty());

        if let Some(url) = self.url.clone() {
            self.apply_connection_url(&url);
        }
        Ok(())
    }

    fn apply_connection_url(&mut self, url: &str) {
        let normalized = expand_env_vars_with_sources(url, &HashMap::new());

        if let Some(target) = sqlite_url_target(&normalized) {
            self.driver = "sqlite".to_string();
            self.sqlite_path = match target {
                SqliteTarget::File(path) => Some(path),
                SqliteTarget::Memory => None,
            };
            self.host.clear();
            self.port = None;
            self.database = None;
            self.username = None;
            self.password = None;
            self.url = Some(normalized);
            return;
        }

        let pattern = regex::Regex::new(
            r"^(?P<driver>[a-zA-Z0-9+]+)://(?:(?P<username>[^:/?#@]+)(?::(?P<password>[^@/?#]*))?@)?(?P<host>\[[^\]]+\]|[^:/?#]+)?(?::(?P<port>\d+))?(?:/(?P<database>[^?#]+))?",
        )
        .unwrap();

        if let Some(captures) = pattern.captures(&normalized) {
            let driver = captures
                .name("driver")
                .map(|value| value.as_str())
                .unwrap_or("postgres");
            self.driver = normalize_driver(driver);
            self.host = captures
                .name("host")
                .map(|value| value.as_str().trim_matches(['[', ']']).to_string())
                .unwrap_or_else(default_host);
            self.port = captures
                .name("port")
                .and_then(|value| value.as_str().parse::<u16>().ok());
            self.database = captures
                .name("database")
                .map(|value| value.as_str().to_string());
            self.username = captures
                .name("username")
                .map(|value| value.as_str().to_string());
            self.password = captures
                .name("password")
                .map(|value| value.as_str().to_string());
            self.sqlite_path = None;
            self.url = Some(normalized);
        }
    }
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

/// The database a `sqlite:` URL opens, read the way sqlx (which TideORM connects
/// through) and SQLite read it; `None` for any other scheme.
///
/// sqlx strips `sqlite://` or `sqlite:`, cuts the query string and percent-decodes the
/// rest, so `sqlite:///var/app.db` is the absolute `/var/app.db`, and both
/// `sqlite://app.db?mode=rwc` and `sqlite:app.db` are `app.db`. `:memory:`, an empty path
/// and `mode=memory` open no file. On Windows SQLite drops the slash in front of a drive
/// letter, so `sqlite:///C:/data/app.db` is `C:/data/app.db`.
pub fn sqlite_url_target(url: &str) -> Option<SqliteTarget> {
    let (database, query) = sqlite_url_parts(url)?;
    if database.is_empty()
        || database == ":memory:"
        || sqlite_modes(&query).any(|mode| mode == "memory")
    {
        return Some(SqliteTarget::Memory);
    }

    let path = percent_decode(database);
    let path = match path.strip_prefix('/') {
        Some(rest) if cfg!(windows) && starts_with_drive_letter(rest) => rest.to_string(),
        _ => path,
    };
    Some(SqliteTarget::File(path))
}

/// Whether sqlx may create the file a `sqlite:` URL names: it opens one
/// read-only when any `mode` is `ro`, and creates one only for `mode=rwc`,
/// which the CLI takes a URL naming no mode to mean.
pub(crate) fn sqlite_url_creates_file(url: &str) -> bool {
    let Some((_, query)) = sqlite_url_parts(url) else {
        return true;
    };
    let modes: Vec<_> = sqlite_modes(&query).collect();
    modes.is_empty() || (modes.contains(&"rwc") && !modes.contains(&"ro"))
}

/// The path of a `sqlite:` URL as written, and its query's decoded pairs;
/// `None` for any other scheme.
fn sqlite_url_parts(url: &str) -> Option<(&str, Vec<(String, String)>)> {
    let (scheme, rest) = url.split_once(':')?;
    if !scheme.eq_ignore_ascii_case("sqlite") {
        return None;
    }

    let rest = rest.strip_prefix("//").unwrap_or(rest);
    let (database, query) = rest.split_once('?').unwrap_or((rest, ""));
    let pairs = query
        .split('&')
        .map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            (form_url_decode(key), form_url_decode(value))
        })
        .collect();
    Some((database, pairs))
}

/// Every `mode` a SQLite URL's query sets; sqlx applies them all.
fn sqlite_modes(query: &[(String, String)]) -> impl Iterator<Item = &str> {
    query
        .iter()
        .filter(|(key, _)| key == "mode")
        .map(|(_, value)| value.as_str())
}

fn form_url_decode(value: &str) -> String {
    percent_decode(&value.replace('+', " "))
}

/// Whether `path` starts with a Windows drive such as `C:`.
fn starts_with_drive_letter(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

/// Decode `%XX` escapes, leaving a malformed one as written, as sqlx does.
fn percent_decode(value: &str) -> String {
    fn hex(byte: u8) -> Option<u8> {
        char::from(byte).to_digit(16).map(|digit| digit as u8)
    }

    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && let (Some(high), Some(low)) = (
                bytes.get(index + 1).copied().and_then(hex),
                bytes.get(index + 2).copied().and_then(hex),
            )
        {
            decoded.push(high << 4 | low);
            index += 3;
            continue;
        }
        decoded.push(bytes[index]);
        index += 1;
    }

    String::from_utf8_lossy(&decoded).into_owned()
}

/// Render `scheme://[user[:password]@]host[:port]/database`.
///
/// The credentials are percent-encoded - a password containing `@`, `:`, `/`, `?` or `#`
/// would otherwise close the userinfo component early and corrupt the URL - and an IPv6
/// host is bracketed, or its colons would read as a port.
pub(crate) fn server_url(
    scheme: &str,
    username: &str,
    password: &str,
    host: &str,
    port: Option<u16>,
    database: &str,
) -> String {
    let host = if host.contains(':') && !host.starts_with('[') {
        format!("[{}]", host)
    } else {
        host.to_string()
    };
    let authority = match port {
        Some(port) => format!("{}:{}", host, port),
        None => host,
    };

    format!(
        "{}://{}{}/{}",
        scheme,
        userinfo_prefix(username, password),
        authority,
        database
    )
}

/// Render the `user[:password]@` prefix of a URL authority, or nothing when there are no
/// credentials to carry.
fn userinfo_prefix(username: &str, password: &str) -> String {
    if username.is_empty() && password.is_empty() {
        return String::new();
    }

    let username = encode_userinfo(username);
    if password.is_empty() {
        format!("{}@", username)
    } else {
        format!("{}:{}@", username, encode_userinfo(password))
    }
}

/// Percent-encode a URL userinfo component (a username or a password).
///
/// Everything outside the RFC 3986 unreserved set is escaped, so credentials containing
/// `@`, `:`, `/` or `#` survive a round trip through a connection URL instead of
/// truncating it. Consumers (sqlx, and therefore SeaORM) percent-decode userinfo, so the
/// original value is what reaches the server.
fn encode_userinfo(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";

    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(byte as char);
            }
            _ => {
                encoded.push('%');
                encoded.push(HEX[usize::from(byte >> 4)] as char);
                encoded.push(HEX[usize::from(byte & 0x0f)] as char);
            }
        }
    }

    encoded
}

/// Resolve the project environment, letting `TIDEORM_ENV` win.
///
/// Production guards are only useful when they can be switched on without editing a
/// checked-in file, so an explicit `TIDEORM_ENV` (process environment or env file)
/// overrides whatever `tideorm.toml` declares. Scaffolds emit
/// `environment = "${TIDEORM_ENV}"`, which resolves to the development default when the
/// variable is unset instead of leaking the placeholder through.
fn resolve_environment(raw: &str, env_values: &HashMap<String, String>) -> String {
    if let Some(environment) = lookup_env("TIDEORM_ENV", env_values) {
        let environment = environment.trim();
        if !environment.is_empty() {
            return environment.to_string();
        }
    }

    let expanded = expand_env_vars_with_sources(raw, env_values);
    let expanded = expanded.trim();
    if expanded.is_empty() || expanded.starts_with('$') {
        return "development".to_string();
    }

    expanded.to_string()
}

/// Whether `value` still holds a supported environment-variable placeholder after
/// expansion, i.e. names a variable nothing defines.
/// The variables the placeholders left in `value` name, in order.
fn unresolved_placeholders(value: &str) -> Vec<&str> {
    PLACEHOLDER
        .captures_iter(value)
        .filter_map(|captures| captures.get(1).or_else(|| captures.get(2)))
        .map(|name| name.as_str())
        .collect()
}

/// Whether `value` is one placeholder and nothing else.
fn is_lone_placeholder(value: &str) -> bool {
    let value = value.trim();
    PLACEHOLDER
        .find(value)
        .is_some_and(|found| found.range() == (0..value.len()))
}

fn expand_env_vars_with_sources(s: &str, env_values: &HashMap<String, String>) -> String {
    let mut result = s.to_string();

    for cap in PLACEHOLDER.captures_iter(s) {
        let var_name = cap.get(1).or_else(|| cap.get(2)).unwrap().as_str();
        if let Some(value) = lookup_env(var_name, env_values) {
            result = result.replace(cap.get(0).unwrap().as_str(), &value);
        }
    }

    result
}

fn lookup_env(name: &str, env_values: &HashMap<String, String>) -> Option<String> {
    std::env::var(name)
        .ok()
        .or_else(|| env_values.get(name).cloned())
}

fn load_env_file(
    config_dir: &Path,
    env_file_name: &str,
) -> Result<HashMap<String, String>, String> {
    env_file_values(&config_dir.join(env_file_name))
}

/// The values the env file at `env_path` sets, read as dotenvy reads them;
/// none when there is no such file.
pub(crate) fn env_file_values(env_path: &Path) -> Result<HashMap<String, String>, String> {
    let content = match std::fs::read_to_string(env_path) {
        Ok(content) => content,
        Err(error) if crate::utils::is_missing_path(env_path, &error) => {
            return Ok(HashMap::new());
        }
        Err(error) => {
            return Err(format!(
                "Failed to read env file {}: {}",
                env_path.display(),
                error
            ));
        }
    };

    let mut values = HashMap::new();
    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };

        let key = key.trim();
        if key.is_empty() {
            continue;
        }

        let (value, expand) = parse_env_value(value);
        // As dotenvy does, a value may use `${VAR}` from the process environment or from
        // the lines above it, so a `DATABASE_URL` can be assembled from parts.
        let value = if expand {
            expand_env_vars_with_sources(&value, &values)
        } else {
            value
        };
        values.insert(key.to_string(), value);
    }

    Ok(values)
}

/// Parse the value of a `KEY=value` line as dotenvy does, returning it with whether its
/// `${VAR}` references are expanded.
///
/// A quoted value runs to its closing quote; an unquoted one ends at a ` #` comment, so
/// `TIDEORM_ENV=production  # live` is `production`, while `p#ss` keeps its `#`. Only a
/// single-quoted value is literal.
fn parse_env_value(value: &str) -> (String, bool) {
    let value = value.trim();

    for quote in ['"', '\''] {
        let Some(inner) = value.strip_prefix(quote) else {
            continue;
        };
        let closing = inner
            .char_indices()
            .find(|&(index, character)| {
                character == quote && (quote == '\'' || !inner[..index].ends_with('\\'))
            })
            .map(|(index, _)| index);
        if let Some(end) = closing {
            let rest = inner[end + 1..].trim_start();
            if rest.is_empty() || rest.starts_with('#') {
                return (inner[..end].to_string(), quote == '"');
            }
        }
    }

    let end = value
        .char_indices()
        .find(|&(index, character)| {
            character.is_whitespace() && value[index..].trim_start().starts_with('#')
        })
        .map_or(value.len(), |(index, _)| index);
    (value[..end].trim_end().to_string(), true)
}

/// The `sqlite:` URL that opens the database file at `path`.
///
/// `%` and `?` are escaped, since sqlx reads the URL back percent-decoded and cuts it at
/// the first `?`: a file name holding either would otherwise open a different file. A
/// space is escaped too: SeaORM parses the URL first and refuses one in a host, and
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

/// Paths configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathsConfig {
    /// Models directory
    #[serde(default = "default_models_path")]
    pub models: String,

    /// Migrations directory
    #[serde(default = "default_migrations_path")]
    pub migrations: String,

    /// Seeders directory
    #[serde(default = "default_seeders_path")]
    pub seeders: String,

    /// Factories directory
    #[serde(default = "default_factories_path")]
    pub factories: String,

    /// Config file that exports TideORM configuration
    #[serde(default = "default_config_file")]
    pub config_file: String,
}

impl Default for PathsConfig {
    fn default() -> Self {
        Self {
            models: default_models_path(),
            migrations: default_migrations_path(),
            seeders: default_seeders_path(),
            factories: default_factories_path(),
            config_file: default_config_file(),
        }
    }
}

fn default_models_path() -> String {
    "src/models".to_string()
}

fn default_migrations_path() -> String {
    "src/migrations".to_string()
}

fn default_seeders_path() -> String {
    "src/seeders".to_string()
}

fn default_factories_path() -> String {
    "src/factories".to_string()
}

fn default_config_file() -> String {
    "src/config.rs".to_string()
}

/// Migration configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationConfig {
    /// Migration table name
    #[serde(default = "default_migration_table")]
    pub table: String,

    /// Use timestamps in migration names
    #[serde(default = "default_true")]
    pub timestamps: bool,

    /// Migration file template
    #[serde(default)]
    pub template: Option<String>,
}

impl Default for MigrationConfig {
    fn default() -> Self {
        Self {
            table: default_migration_table(),
            timestamps: true,
            template: None,
        }
    }
}

fn default_migration_table() -> String {
    "_migrations".to_string()
}

fn default_true() -> bool {
    true
}

/// Seeder configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SeederConfig {
    /// Seeder file template
    #[serde(default)]
    pub template: Option<String>,
}

/// Model generation configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelGenConfig {
    /// Default timestamps (created_at, updated_at)
    #[serde(default = "default_true")]
    pub timestamps: bool,

    /// Default soft deletes
    #[serde(default)]
    pub soft_deletes: bool,

    /// Default tokenization
    #[serde(default)]
    pub tokenize: bool,

    /// Model file template
    #[serde(default)]
    pub template: Option<String>,

    /// Primary key field name
    #[serde(default = "default_primary_key")]
    pub primary_key: String,

    /// Primary key type
    #[serde(default = "default_primary_key_type")]
    pub primary_key_type: String,
}

impl Default for ModelGenConfig {
    fn default() -> Self {
        Self {
            timestamps: true,
            soft_deletes: false,
            tokenize: false,
            template: None,
            primary_key: default_primary_key(),
            primary_key_type: default_primary_key_type(),
        }
    }
}

fn default_primary_key() -> String {
    "id".to_string()
}

fn default_primary_key_type() -> String {
    "i64".to_string()
}

impl TideConfig {
    /// Load configuration from a file
    pub fn load(path: &str) -> Result<Self, String> {
        let path = Path::new(path);
        let content = match std::fs::read_to_string(path) {
            Ok(content) => content,
            Err(error) if crate::utils::is_missing_path(path, &error) => {
                return Err(format!(
                    "Configuration file not found: {}. Run 'tideorm init' to create one.",
                    path.display()
                ));
            }
            Err(error) => return Err(format!("Failed to read config file: {}", error)),
        };

        let mut config: Self =
            toml::from_str(&content).map_err(|e| format!("Failed to parse config file: {}", e))?;

        let env_values = load_env_file(
            path.parent().unwrap_or_else(|| Path::new(".")),
            &config.project.env_file,
        )?;

        config.project.name = expand_env_vars_with_sources(&config.project.name, &env_values);
        config.project.environment = resolve_environment(&config.project.environment, &env_values);
        config.project.env_file =
            expand_env_vars_with_sources(&config.project.env_file, &env_values);
        config.paths.models = expand_env_vars_with_sources(&config.paths.models, &env_values);
        config.paths.migrations =
            expand_env_vars_with_sources(&config.paths.migrations, &env_values);
        config.paths.seeders = expand_env_vars_with_sources(&config.paths.seeders, &env_values);
        config.paths.factories = expand_env_vars_with_sources(&config.paths.factories, &env_values);
        config.paths.config_file =
            expand_env_vars_with_sources(&config.paths.config_file, &env_values);
        config.migration.table = expand_env_vars_with_sources(&config.migration.table, &env_values);
        config.migration.template = config
            .migration
            .template
            .as_ref()
            .map(|value| expand_env_vars_with_sources(value, &env_values));
        config.seeder.template = config
            .seeder
            .template
            .as_ref()
            .map(|value| expand_env_vars_with_sources(value, &env_values));
        config.model.template = config
            .model
            .template
            .as_ref()
            .map(|value| expand_env_vars_with_sources(value, &env_values));
        config.model.primary_key =
            expand_env_vars_with_sources(&config.model.primary_key, &env_values);
        config.model.primary_key_type =
            expand_env_vars_with_sources(&config.model.primary_key_type, &env_values);
        config.database.resolve_with_env(&env_values)?;

        Ok(config)
    }

    /// Load configuration, falling back to defaults only when the file does not exist.
    ///
    /// A missing `tideorm.toml` is a legitimate "no project configuration" state for the
    /// commands that only generate files, so it yields [`TideConfig::default`]. A file that
    /// exists but cannot be read or parsed is always an error: silently replacing it with
    /// the built-in (Postgres) defaults would emit code for the wrong backend into the
    /// wrong directories.
    pub fn load_or_default(path: &str) -> Result<Self, String> {
        match std::fs::metadata(path) {
            Ok(_) => Self::load(path),
            Err(error) if crate::utils::is_missing_path(Path::new(path), &error) => {
                Ok(Self::default())
            }
            Err(error) => Err(format!("Failed to inspect config file {}: {}", path, error)),
        }
    }

    /// Check if running in production
    ///
    /// The name is matched in any case: `TIDEORM_ENV=Production` must not switch the
    /// production guards off.
    pub fn is_production(&self) -> bool {
        self.project
            .environment
            .trim()
            .eq_ignore_ascii_case("production")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_default_config() {
        let config = TideConfig::default();
        assert_eq!(config.database.driver, "postgres");
        assert_eq!(config.paths.models, "src/models");
    }

    #[test]
    fn test_connection_url_postgres() {
        let config = DatabaseConfig {
            driver: "postgres".to_string(),
            username: Some("user".to_string()),
            password: Some("pass".to_string()),
            database: Some("mydb".to_string()),
            ..Default::default()
        };

        let url = config.connection_url();
        assert!(url.starts_with("postgres://"));
        assert!(url.contains("user"));
        assert!(url.contains("mydb"));
    }

    #[test]
    fn test_connection_url_sqlite() {
        let config = DatabaseConfig {
            driver: "sqlite".to_string(),
            sqlite_path: Some("test.db".to_string()),
            ..Default::default()
        };

        let url = config.connection_url();
        assert_eq!(url, "sqlite://test.db");
    }

    #[test]
    fn test_connection_url_sqlite_windows_absolute_path() {
        let config = DatabaseConfig {
            driver: "sqlite".to_string(),
            sqlite_path: Some("C:\\Users\\alice\\project\\data.db".to_string()),
            ..Default::default()
        };

        let url = config.connection_url();
        assert_eq!(url, "sqlite:///C:/Users/alice/project/data.db");
    }

    #[test]
    fn test_load_uses_database_url_from_dotenv() {
        let fixture = TempDir::new().unwrap();
        let config_path = fixture.path().join("tideorm.toml");
        let env_path = fixture.path().join(".env");

        fs::write(
            &config_path,
            r#"[project]
name = "demo"

[database]
driver = "postgres"
host = "localhost"
port = 5432
database = "tideorm_db"
username = "postgres"
password = ""
"#,
        )
        .unwrap();
        fs::write(
            &env_path,
            "DATABASE_URL=postgres://postgres:postgres@localhost:5432/test_tide_ormx\n",
        )
        .unwrap();

        let config = TideConfig::load(config_path.to_str().unwrap()).unwrap();

        assert_eq!(
            config.database.url.as_deref(),
            Some("postgres://postgres:postgres@localhost:5432/test_tide_ormx")
        );
        assert_eq!(config.database.driver, "postgres");
        assert_eq!(config.database.host, "localhost");
        assert_eq!(config.database.port, Some(5432));
        assert_eq!(config.database.database.as_deref(), Some("test_tide_ormx"));
        assert_eq!(config.database.username.as_deref(), Some("postgres"));
        assert_eq!(config.database.password.as_deref(), Some("postgres"));
    }

    #[test]
    fn test_load_expands_database_url_placeholder_from_dotenv() {
        let fixture = TempDir::new().unwrap();
        let config_path = fixture.path().join("tideorm.toml");
        let env_path = fixture.path().join(".env");

        fs::write(
            &config_path,
            r#"[database]
driver = "postgres"
url = "${DATABASE_URL}"
"#,
        )
        .unwrap();
        fs::write(
            &env_path,
            "DATABASE_URL=postgres://postgres:secret@db.example.com:5433/app_db\n",
        )
        .unwrap();

        let config = TideConfig::load(config_path.to_str().unwrap()).unwrap();

        assert_eq!(
            config.database.connection_url(),
            "postgres://postgres:secret@db.example.com:5433/app_db"
        );
        assert_eq!(config.database.host, "db.example.com");
        assert_eq!(config.database.port, Some(5433));
        assert_eq!(config.database.database.as_deref(), Some("app_db"));
        assert_eq!(config.database.username.as_deref(), Some("postgres"));
    }

    #[test]
    fn test_connection_url_percent_encodes_credentials() {
        let config = DatabaseConfig {
            driver: "postgres".to_string(),
            username: Some("a@b".to_string()),
            password: Some("p@ss:w/rd#1".to_string()),
            database: Some("mydb".to_string()),
            ..Default::default()
        };

        assert_eq!(
            config.connection_url(),
            "postgres://a%40b:p%40ss%3Aw%2Frd%231@localhost:5432/mydb"
        );
    }

    #[test]
    fn test_connection_url_supports_mariadb() {
        let config = DatabaseConfig {
            driver: "mariadb".to_string(),
            database: Some("shop".to_string()),
            ..Default::default()
        };

        let url = config.connection_url();
        assert_eq!(url, "mysql://root@localhost:3306/shop");
    }

    #[test]
    fn test_connection_url_does_not_panic_on_unknown_driver() {
        let config = DatabaseConfig {
            driver: "oracle".to_string(),
            database: Some("app".to_string()),
            ..Default::default()
        };

        assert!(config.try_connection_url().is_err());
        assert_eq!(config.connection_url(), "oracle://localhost/app");
    }

    #[test]
    fn resolve_environment_prefers_tideorm_env() {
        let mut env_values = HashMap::new();
        env_values.insert("TIDEORM_ENV".to_string(), "production".to_string());
        let resolved = resolve_environment("development", &env_values);

        assert_eq!(resolved, "production");
    }

    #[test]
    fn resolve_environment_falls_back_for_unresolved_placeholder() {
        assert_eq!(
            resolve_environment("${TIDEORM_ENV}", &HashMap::new()),
            default_environment()
        );
    }

    #[test]
    fn test_load_environment_placeholder_enables_production_guards() {
        let fixture = TempDir::new().unwrap();
        let config_path = fixture.path().join("tideorm.toml");

        fs::write(
            &config_path,
            r#"[project]
name = "demo"
environment = "${TIDEORM_ENV}"

[database]
driver = "sqlite"
sqlite_path = "app.db"
"#,
        )
        .unwrap();
        fs::write(fixture.path().join(".env"), "TIDEORM_ENV=production\n").unwrap();

        let config = TideConfig::load(config_path.to_str().unwrap()).unwrap();

        assert_eq!(config.project.environment, "production");
        assert!(config.is_production());
    }

    #[test]
    fn test_load_or_default_reports_broken_config() {
        let fixture = TempDir::new().unwrap();
        let config_path = fixture.path().join("tideorm.toml");
        fs::write(&config_path, "this is not = valid toml [\n").unwrap();

        let path = config_path.to_str().unwrap();
        assert!(TideConfig::load_or_default(path).is_err());

        let absent = fixture.path().join("missing.toml");
        let absent = absent.to_str().unwrap();
        assert!(TideConfig::load_or_default(absent).is_ok());
    }

    #[test]
    fn test_load_or_default_does_not_fall_back_to_postgres_for_a_broken_config() {
        let fixture = TempDir::new().unwrap();
        let config_path = fixture.path().join("tideorm.toml");
        fs::write(
            &config_path,
            "[database]\ndriver = \"sqlite\"\nsqlite_path = \n",
        )
        .unwrap();

        let error = TideConfig::load_or_default(config_path.to_str().unwrap())
            .expect_err("a config file that exists but cannot be parsed must be reported");
        assert!(error.contains("Failed to parse config file"), "{}", error);
    }

    #[test]
    fn load_or_default_does_not_treat_a_blocked_config_path_as_missing() {
        let fixture = TempDir::new().unwrap();
        let blocker = fixture.path().join("not-a-directory");
        fs::write(&blocker, "").unwrap();
        let config_path = blocker.join("tideorm.toml");

        let error = TideConfig::load_or_default(config_path.to_str().unwrap())
            .expect_err("an unreadable config path must not silently use default settings");
        assert!(error.contains("Failed to inspect config file"), "{error}");
    }

    #[test]
    fn test_load_uses_custom_env_file_name() {
        let fixture = TempDir::new().unwrap();
        let config_path = fixture.path().join("tideorm.toml");
        let env_path = fixture.path().join(".env.local");

        fs::write(
            &config_path,
            r#"[project]
name = "demo"
env_file = ".env.local"

[database]
driver = "postgres"
"#,
        )
        .unwrap();
        fs::write(
            &env_path,
            "DATABASE_URL=postgres://postgres:postgres@localhost:5432/custom_env_db\n",
        )
        .unwrap();

        let config = TideConfig::load(config_path.to_str().unwrap()).unwrap();
        assert_eq!(config.project.env_file, ".env.local");
        assert_eq!(config.database.database.as_deref(), Some("custom_env_db"));
    }

    #[test]
    fn load_reports_an_env_file_path_blocked_by_a_file() {
        let fixture = TempDir::new().unwrap();
        let config_path = fixture.path().join("tideorm.toml");
        let blocker = fixture.path().join("not-a-directory");
        fs::write(&blocker, "").unwrap();
        fs::write(
            &config_path,
            "[project]\nenv_file = \"not-a-directory/.env\"\n\n[database]\ndriver = \"sqlite\"\n",
        )
        .unwrap();

        let error = TideConfig::load(config_path.to_str().unwrap())
            .expect_err("a broken env-file path must not be silently ignored");
        assert!(error.contains("Failed to read env file"), "{error}");
    }

    fn applied(url: &str) -> DatabaseConfig {
        let mut config = DatabaseConfig::default();
        config.apply_connection_url(url);
        config
    }

    #[test]
    fn sqlite_urls_name_the_file_sqlx_opens() {
        let absolute = if cfg!(windows) {
            "C:/data/app.db"
        } else {
            "/C:/data/app.db"
        };
        for (url, file) in [
            ("sqlite://app.db?mode=rwc", "app.db"),
            ("SQLite://app.db", "app.db"),
            ("sqlite:app.db", "app.db"),
            ("sqlite:data/app.db?mode=rwc&cache=shared", "data/app.db"),
            ("sqlite:///var/data/app.db", "/var/data/app.db"),
            ("sqlite:///C:/data/app.db", absolute),
            ("sqlite://my%20data/app%3F.db", "my data/app?.db"),
        ] {
            let config = applied(url);
            assert_eq!(config.driver, "sqlite", "{url}");
            assert_eq!(config.sqlite_path.as_deref(), Some(file), "{url}");
            assert_eq!(config.sqlite_file().as_deref(), Some(file), "{url}");
        }

        for url in [
            "sqlite::memory:",
            "sqlite://:memory:",
            "sqlite://app.db?mode=memory",
            "sqlite://app.db?mode=%6Demory",
            "sqlite://app.db?%6Dode=memory",
        ] {
            let config = applied(url);
            assert_eq!(config.driver, "sqlite", "{url}");
            assert_eq!(config.sqlite_file(), None, "{url}");
        }
        for (url, creates) in [
            ("sqlite://app.db?cache=shared", true),
            ("sqlite://app.db?mode=rwc", true),
            ("sqlite://app.db?mode=rwc&mode=rw", true),
            ("sqlite://app.db?mode=ro", false),
            ("sqlite://app.db?%6Dode=rw", false),
            ("sqlite://app.db?mode=ro&mode=rwc", false),
        ] {
            assert_eq!(sqlite_url_creates_file(url), creates, "{url}");
        }
    }

    #[test]
    fn unresolved_database_url_placeholders_cover_both_supported_forms() {
        assert_eq!(
            unresolved_placeholders("sqlite://$DATABASE_URL"),
            ["DATABASE_URL"]
        );
        assert_eq!(
            unresolved_placeholders("sqlite://${DATABASE_URL}"),
            ["DATABASE_URL"]
        );
        assert!(unresolved_placeholders("sqlite://database.db").is_empty());
    }

    #[test]
    fn sqlite_paths_survive_a_round_trip_through_their_url() {
        // sqlx decodes `%20` and cuts at `?`, so both must be escaped.
        assert_eq!(sqlite_connection_url("a%20b.db"), "sqlite://a%2520b.db");
        assert_eq!(sqlite_connection_url("what?.db"), "sqlite://what%3F.db");
        assert_eq!(sqlite_connection_url("my app.db"), "sqlite://my%20app.db");
        for path in ["a%20b.db", "what?.db", "my app.db", "data/app.db"] {
            let config = applied(&sqlite_connection_url(path));
            assert_eq!(config.sqlite_file().as_deref(), Some(path));
        }
    }

    #[test]
    fn a_sqlite_url_wins_over_the_configured_path() {
        let fixture = TempDir::new().unwrap();
        let config_path = fixture.path().join("tideorm.toml");
        fs::write(
            &config_path,
            "[database]\ndriver = \"sqlite\"\nsqlite_path = \"decoy.db\"\nurl = \"${TIDEORM_TEST_SQLITE_URL}\"\n",
        )
        .unwrap();
        fs::write(
            fixture.path().join(".env"),
            "TIDEORM_TEST_SQLITE_URL=sqlite:real.db?mode=rwc\n",
        )
        .unwrap();

        let config = TideConfig::load(config_path.to_str().unwrap()).unwrap();
        assert_eq!(config.database.sqlite_file().as_deref(), Some("real.db"));
    }

    #[test]
    fn every_spelling_of_mariadb_and_mysql_is_the_mysql_driver() {
        for url in [
            "mariadb://root:secret@db:3306/shop",
            "MariaDB://root@db/shop",
            "MySQL://root@db/shop",
        ] {
            let config = applied(url);
            assert_eq!(config.driver, "mysql", "{url}");
            assert_eq!(config.database.as_deref(), Some("shop"), "{url}");
        }
        assert_eq!(applied("PostgreSQL://u@db/app").driver, "postgres");

        let mut config = DatabaseConfig {
            driver: "MariaDB".to_string(),
            ..Default::default()
        };
        config.resolve_with_env(&HashMap::new()).unwrap();
        assert_eq!(config.driver, "mysql");
    }

    #[test]
    fn server_urls_bracket_ipv6_hosts() {
        let config = DatabaseConfig {
            driver: "postgres".to_string(),
            host: "::1".to_string(),
            username: Some("app".to_string()),
            password: Some("p?ss".to_string()),
            database: Some("app_db".to_string()),
            ..Default::default()
        };

        assert_eq!(
            config.connection_url(),
            "postgres://app:p%3Fss@[::1]:5432/app_db"
        );
        assert_eq!(
            config.server_url_for_database("postgres").unwrap(),
            "postgres://app:p%3Fss@[::1]:5432/postgres"
        );
        // A URL's bracketed host round-trips.
        assert_eq!(
            applied("postgres://app@[::1]:5433/app_db").host,
            "::1".to_string()
        );
    }

    #[test]
    fn env_file_values_drop_inline_comments() {
        assert_eq!(
            parse_env_value("production  # live"),
            ("production".to_string(), true)
        );
        assert_eq!(parse_env_value("p#ss"), ("p#ss".to_string(), true));
        assert_eq!(
            parse_env_value("\"a # b\" # note"),
            ("a # b".to_string(), true)
        );
        assert_eq!(parse_env_value("'${X}'"), ("${X}".to_string(), false));
        assert_eq!(parse_env_value("\"x\""), ("x".to_string(), true));
    }

    #[test]
    fn production_guards_hold_with_a_commented_or_capitalised_env() {
        for line in ["TIDEORM_ENV=production  # live", "TIDEORM_ENV=Production"] {
            let fixture = TempDir::new().unwrap();
            let config_path = fixture.path().join("tideorm.toml");
            fs::write(
                &config_path,
                "[project]\nenvironment = \"${TIDEORM_ENV}\"\n\n[database]\ndriver = \"sqlite\"\n",
            )
            .unwrap();
            fs::write(fixture.path().join(".env"), format!("{line}\n")).unwrap();

            let config = TideConfig::load(config_path.to_str().unwrap()).unwrap();
            assert!(config.is_production(), "{line}");
        }
    }

    #[test]
    fn an_unresolved_url_placeholder_keeps_the_declarative_settings() {
        let fixture = TempDir::new().unwrap();
        let config_path = fixture.path().join("tideorm.toml");
        fs::write(
            &config_path,
            "[database]\ndriver = \"sqlite\"\nsqlite_path = \"app.db\"\nurl = \"${TIDEORM_TEST_UNSET_URL}\"\n",
        )
        .unwrap();

        let config = TideConfig::load(config_path.to_str().unwrap()).unwrap();
        assert_eq!(config.database.url, None);
        assert_eq!(config.database.connection_url(), "sqlite://app.db");
    }

    /// A URL whose password placeholder nothing sets used to be dropped, and
    /// the CLI then connected with the `[database]` settings or the defaults:
    /// `migrate --force` ran against another database.
    #[test]
    fn a_partly_unresolved_url_placeholder_is_an_error() {
        let fixture = TempDir::new().unwrap();
        let config_path = fixture.path().join("tideorm.toml");
        fs::write(
            &config_path,
            "[database]\ndriver = \"postgres\"\nurl = \"postgres://app:${TIDEORM_TEST_UNSET_PASSWORD}@db.internal/app\"\n",
        )
        .unwrap();

        let error = TideConfig::load(config_path.to_str().unwrap()).unwrap_err();
        assert!(error.contains("TIDEORM_TEST_UNSET_PASSWORD"), "{error}");
    }

    #[test]
    fn env_file_values_expand_variables_defined_above_them() {
        let fixture = TempDir::new().unwrap();
        let config_path = fixture.path().join("tideorm.toml");
        fs::write(
            &config_path,
            "[database]\ndriver = \"postgres\"\nurl = \"${TIDEORM_TEST_URL}\"\n",
        )
        .unwrap();
        fs::write(
            fixture.path().join(".env"),
            "TIDEORM_TEST_HOST=db.internal\nTIDEORM_TEST_URL=postgres://app@${TIDEORM_TEST_HOST}:5433/app_db\n",
        )
        .unwrap();

        let config = TideConfig::load(config_path.to_str().unwrap()).unwrap();
        assert_eq!(config.database.host, "db.internal");
        assert_eq!(config.database.port, Some(5433));
    }
}
