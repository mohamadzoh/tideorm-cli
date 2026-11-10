//! Configuration module for TideORM CLI
//!
//! Handles loading and parsing of tideorm.toml configuration files.

use serde::{Deserialize, Serialize};
use std::path::Path;

mod database;
mod env;

pub use database::DatabaseConfig;
pub(crate) use database::{canonical_driver, server_url, sqlite_connection_url};
pub(crate) use env::{env_assignment, env_file_values};
use env::{expand_env_vars_with_sources, load_env_file, resolve_environment};

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
}

impl Default for PathsConfig {
    fn default() -> Self {
        Self {
            models: default_models_path(),
            migrations: default_migrations_path(),
            seeders: default_seeders_path(),
            factories: default_factories_path(),
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

    /// Refuse to `action` in production unless `--force` was given.
    pub fn guard_production(&self, force: bool, action: &str) -> Result<(), String> {
        if self.is_production() && !force {
            return Err(format!(
                "Cannot {action} in production without --force flag"
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "../tests/unit/config_tests.rs"]
mod tests;
