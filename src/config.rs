//! Configuration module for TideORM CLI
//!
//! Handles loading and parsing of tideorm.toml configuration files.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

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
    /// Build connection URL from configuration
    pub fn connection_url(&self) -> String {
        if let Some(url) = &self.url {
            // Replace environment variables in URL
            return self.expand_env_vars(url);
        }

        match self.driver.as_str() {
            "sqlite" => {
                let path = self.sqlite_path.as_deref().unwrap_or("database.db");
                sqlite_connection_url(path)
            }
            "postgres" | "postgresql" => {
                let user = self.username.as_deref().unwrap_or("postgres");
                let pass = self.password.as_deref().unwrap_or("");
                let host = &self.host;
                let port = self.port.unwrap_or(5432);
                let db = self.database.as_deref().unwrap_or("tideorm");

                if pass.is_empty() {
                    format!("postgres://{}@{}:{}/{}", user, host, port, db)
                } else {
                    format!("postgres://{}:{}@{}:{}/{}", user, pass, host, port, db)
                }
            }
            "mysql" => {
                let user = self.username.as_deref().unwrap_or("root");
                let pass = self.password.as_deref().unwrap_or("");
                let host = &self.host;
                let port = self.port.unwrap_or(3306);
                let db = self.database.as_deref().unwrap_or("tideorm");

                if pass.is_empty() {
                    format!("mysql://{}@{}:{}/{}", user, host, port, db)
                } else {
                    format!("mysql://{}:{}@{}:{}/{}", user, pass, host, port, db)
                }
            }
            _ => panic!("Unsupported database driver: {}", self.driver),
        }
    }

    /// Expand environment variables in a string
    fn expand_env_vars(&self, s: &str) -> String {
        expand_env_vars_with_sources(s, &HashMap::new())
    }

    fn resolve_with_env(&mut self, env_values: &HashMap<String, String>) {
        self.driver = expand_env_vars_with_sources(&self.driver, env_values);
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
            Some(value) => Some(expand_env_vars_with_sources(value, env_values)),
            None => lookup_env("DATABASE_URL", env_values),
        };

        if let Some(url) = self.url.clone() {
            self.apply_connection_url(&url);
        }
    }

    fn apply_connection_url(&mut self, url: &str) {
        let normalized = expand_env_vars_with_sources(url, &HashMap::new());

        if let Some(path) = normalized.strip_prefix("sqlite:///") {
            self.driver = "sqlite".to_string();
            self.sqlite_path = Some(path.to_string());
            self.host.clear();
            self.port = None;
            self.database = None;
            self.username = None;
            self.password = None;
            self.url = Some(normalized);
            return;
        }

        if let Some(path) = normalized.strip_prefix("sqlite://") {
            self.driver = "sqlite".to_string();
            self.sqlite_path = Some(path.to_string());
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
            self.driver = match driver {
                "postgresql" => "postgres".to_string(),
                other => other.to_string(),
            };
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

fn expand_env_vars_with_sources(s: &str, env_values: &HashMap<String, String>) -> String {
    let mut result = s.to_string();
    let re = regex::Regex::new(r"\$\{([^}]+)\}|\$([A-Z_][A-Z0-9_]*)").unwrap();

    for cap in re.captures_iter(s) {
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

fn load_env_file(config_dir: &Path, env_file_name: &str) -> Result<HashMap<String, String>, String> {
    let env_path = config_dir.join(env_file_name);
    if !env_path.exists() {
        return Ok(HashMap::new());
    }

    let content = std::fs::read_to_string(&env_path)
        .map_err(|error| format!("Failed to read env file {}: {}", env_path.display(), error))?;

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

        let value = parse_env_value(value.trim());
        values.insert(key.to_string(), value);
    }

    Ok(values)
}

fn parse_env_value(value: &str) -> String {
    let mut parsed = value.trim().to_string();
    if parsed.len() >= 2 {
        let quoted_with_double = parsed.starts_with('"') && parsed.ends_with('"');
        let quoted_with_single = parsed.starts_with('\'') && parsed.ends_with('\'');
        if quoted_with_double || quoted_with_single {
            parsed = parsed[1..parsed.len() - 1].to_string();
        }
    }

    parsed
}

fn sqlite_connection_url(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    let bytes = normalized.as_bytes();
    let is_windows_absolute = bytes.len() >= 3 && bytes[1] == b':' && bytes[2] == b'/';

    if is_windows_absolute {
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeederConfig {
    /// Default seeder class
    #[serde(default = "default_seeder_class")]
    pub default_seeder: String,

    /// Seeder file template
    #[serde(default)]
    pub template: Option<String>,
}

impl Default for SeederConfig {
    fn default() -> Self {
        Self {
            default_seeder: default_seeder_class(),
            template: None,
        }
    }
}

fn default_seeder_class() -> String {
    "DatabaseSeeder".to_string()
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

        if !path.exists() {
            return Err(format!(
                "Configuration file not found: {}. Run 'tideorm init' to create one.",
                path.display()
            ));
        }

        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read config file: {}", e))?;

        let mut config: Self =
            toml::from_str(&content).map_err(|e| format!("Failed to parse config file: {}", e))?;

        let env_values = load_env_file(
            path.parent().unwrap_or_else(|| Path::new(".")),
            &config.project.env_file,
        )?;

        config.project.name = expand_env_vars_with_sources(&config.project.name, &env_values);
        config.project.environment =
            expand_env_vars_with_sources(&config.project.environment, &env_values);
        config.project.env_file = expand_env_vars_with_sources(&config.project.env_file, &env_values);
        config.paths.models = expand_env_vars_with_sources(&config.paths.models, &env_values);
        config.paths.migrations =
            expand_env_vars_with_sources(&config.paths.migrations, &env_values);
        config.paths.seeders = expand_env_vars_with_sources(&config.paths.seeders, &env_values);
        config.paths.factories =
            expand_env_vars_with_sources(&config.paths.factories, &env_values);
        config.paths.config_file =
            expand_env_vars_with_sources(&config.paths.config_file, &env_values);
        config.migration.table = expand_env_vars_with_sources(&config.migration.table, &env_values);
        config.migration.template = config
            .migration
            .template
            .as_ref()
            .map(|value| expand_env_vars_with_sources(value, &env_values));
        config.seeder.default_seeder =
            expand_env_vars_with_sources(&config.seeder.default_seeder, &env_values);
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
        config.database.resolve_with_env(&env_values);

        Ok(config)
    }

    /// Load configuration or return default
    pub fn load_or_default(path: &str) -> Self {
        Self::load(path).unwrap_or_default()
    }

    /// Check if running in production
    pub fn is_production(&self) -> bool {
        self.project.environment == "production"
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
}
