//! Configuration module for TideORM CLI
//!
//! Handles loading and parsing of tideorm.toml configuration files.

use serde::{Deserialize, Serialize};
use std::path::Path;

/// TideORM CLI Configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
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

impl Default for TideConfig {
    fn default() -> Self {
        Self {
            project: ProjectConfig::default(),
            database: DatabaseConfig::default(),
            paths: PathsConfig::default(),
            migration: MigrationConfig::default(),
            seeder: SeederConfig::default(),
            model: ModelGenConfig::default(),
        }
    }
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
}

impl Default for ProjectConfig {
    fn default() -> Self {
        Self {
            name: default_project_name(),
            environment: default_environment(),
        }
    }
}

fn default_project_name() -> String {
    "tideorm-project".to_string()
}

fn default_environment() -> String {
    std::env::var("TIDEORM_ENV").unwrap_or_else(|_| "development".to_string())
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
                format!("sqlite://{}", path)
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
        let mut result = s.to_string();
        let re = regex::Regex::new(r"\$\{([^}]+)\}|\$([A-Z_][A-Z0-9_]*)").unwrap();

        for cap in re.captures_iter(s) {
            let var_name = cap.get(1).or_else(|| cap.get(2)).unwrap().as_str();
            if let Ok(value) = std::env::var(var_name) {
                result = result.replace(cap.get(0).unwrap().as_str(), &value);
            }
        }

        result
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
    "_tideorm_migrations".to_string()
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

        toml::from_str(&content).map_err(|e| format!("Failed to parse config file: {}", e))
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

/// Generate default tideorm.toml content
pub fn default_config_content(database: &str) -> String {
    let db_config = match database {
        "sqlite" => r#"
driver = "sqlite"
sqlite_path = "database.db"
"#,
        "mysql" => r#"
driver = "mysql"
host = "localhost"
port = 3306
database = "tideorm_db"
username = "root"
password = ""
# Or use a connection URL:
# url = "mysql://root:password@localhost:3306/tideorm_db"
"#,
        _ => r#"
driver = "postgres"
host = "localhost"
port = 5432
database = "tideorm_db"
username = "postgres"
password = ""
# Or use a connection URL:
# url = "postgres://postgres:password@localhost:5432/tideorm_db"
# You can also use environment variables:
# url = "${DATABASE_URL}"
"#,
    };

    format!(
        r#"# TideORM Configuration File
# This file configures the TideORM CLI and runtime behavior.

[project]
name = "my-tideorm-project"
environment = "development"  # development, production, test

[database]{db_config}pool_size = 5
timeout = 30

[paths]
models = "src/models"
migrations = "src/migrations"
seeders = "src/seeders"
factories = "src/factories"
config_file = "src/config.rs"

[migration]
table = "_tideorm_migrations"
timestamps = true

[seeder]
default_seeder = "DatabaseSeeder"

[model]
timestamps = true
soft_deletes = false
tokenize = false
primary_key = "id"
primary_key_type = "i64"
"#,
        db_config = db_config
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = TideConfig::default();
        assert_eq!(config.database.driver, "postgres");
        assert_eq!(config.paths.models, "src/models");
    }

    #[test]
    fn test_connection_url_postgres() {
        let mut config = DatabaseConfig::default();
        config.driver = "postgres".to_string();
        config.username = Some("user".to_string());
        config.password = Some("pass".to_string());
        config.database = Some("mydb".to_string());

        let url = config.connection_url();
        assert!(url.starts_with("postgres://"));
        assert!(url.contains("user"));
        assert!(url.contains("mydb"));
    }

    #[test]
    fn test_connection_url_sqlite() {
        let mut config = DatabaseConfig::default();
        config.driver = "sqlite".to_string();
        config.sqlite_path = Some("test.db".to_string());

        let url = config.connection_url();
        assert_eq!(url, "sqlite://test.db");
    }
}
