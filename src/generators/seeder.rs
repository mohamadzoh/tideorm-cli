//! Seeder generator for TideORM CLI

use crate::config::TideConfig;
use crate::utils::{
    crate_module_path, ensure_directory, ensure_writable, escape_ident, to_snake_case,
};

/// Seeder generator
pub struct SeederGenerator<'a> {
    config: &'a TideConfig,
    output_dir: Option<String>,
}

impl<'a> SeederGenerator<'a> {
    /// Create a new seeder generator
    pub fn new(config: &'a TideConfig) -> Self {
        Self {
            config,
            output_dir: None,
        }
    }

    /// Override the directory seeders are written to (the `--output` flag).
    ///
    /// `None` keeps the configured `[paths] seeders` directory.
    ///
    /// This only moves the generated file: the model import is still derived from
    /// `[paths] models`, so relocating a seeder does not break it.
    pub fn output_dir(mut self, dir: Option<&str>) -> Self {
        self.output_dir = dir
            .map(str::trim)
            .filter(|dir| !dir.is_empty())
            .map(ToOwned::to_owned);
        self
    }

    /// Directory the generated seeder and its `mod.rs` belong to.
    fn seeders_dir(&self) -> &str {
        self.output_dir
            .as_deref()
            .unwrap_or(&self.config.paths.seeders)
    }

    /// Generate a seeder file
    pub fn generate(
        &self,
        name: &str,
        model: Option<String>,
        count: u32,
    ) -> Result<String, String> {
        ensure_directory(self.seeders_dir())?;

        let seeder_name = if name.ends_with("Seeder") {
            to_pascal_case(name)
        } else {
            format!("{}Seeder", to_pascal_case(name))
        };

        let file_name = format!("{}.rs", to_snake_case(&seeder_name));
        let file_path = format!("{}/{}", self.seeders_dir(), file_name);

        let content = if let Some(model_name) = model {
            self.generate_model_seeder(&seeder_name, &model_name, count)
        } else {
            self.generate_basic_seeder(&seeder_name)
        };

        ensure_writable(&file_path)?;

        std::fs::write(&file_path, content)
            .map_err(|e| format!("Failed to write seeder file: {}", e))?;

        // Update mod.rs
        self.update_mod_file(&seeder_name)?;

        Ok(file_path)
    }

    /// Generate a seeder for a specific model
    ///
    /// The model is imported through the configured `[paths] models` directory rather than
    /// a hardcoded `crate::models`, so a project that keeps its models elsewhere still gets
    /// a seeder that compiles.
    fn generate_model_seeder(&self, seeder_name: &str, model_name: &str, count: u32) -> String {
        let model_snake = to_snake_case(model_name);
        let model_pascal = to_pascal_case(model_name);
        let model_module = escape_ident(&model_snake);
        let models_path = crate_module_path(&self.config.paths.models);
        let factories_path = crate_module_path(&self.config.paths.factories);

        format!(
            r#"//! {} Seeder
//!
//! Seeds the database with {} records.

use tideorm::prelude::*;
use {models_path}::{model_module}::{model_pascal};

/// {} seeder
#[derive(Default)]
pub struct {seeder_name};

#[async_trait]
impl Seed for {seeder_name} {{
    fn name(&self) -> &str {{
        "{model_snake}_seeder"
    }}

    async fn run(&self, _db: &Database) -> tideorm::Result<()> {{
        println!("Seeding {model_snake}s...");

        for _i in 1..={count} {{
            let {model_snake} = {model_pascal} {{
                // TODO: Fill in the model fields
                // Example:
                // name: format!("{model_pascal} {{}}", _i),
                // email: format!("{model_snake}{{}}@example.com", _i),
                ..Default::default()
            }};

            {model_snake}.save().await?;
        }}

        println!("Seeded {count} {model_snake}(s)");
        Ok(())
    }}
}}

impl {seeder_name} {{
    /// Run the seeder with a factory
    pub async fn run_with_factory() -> tideorm::Result<()> {{
        println!("Seeding {model_snake}s with factory...");

        // TODO: Use factory pattern
        // Example:
        // {factories_path}::{model_snake}_factory::{model_pascal}Factory::create_many({count}).await?;

        Self::default().run(db()).await
    }}
}}

#[cfg(test)]
mod tests {{
    use super::*;

    #[tokio::test]
    async fn test_seeder() {{
        // Set up test database
        // Run seeder
        // Verify records were created
    }}
}}
"#,
            seeder_name,
            model_name,
            model_name,
            seeder_name = seeder_name,
            model_pascal = model_pascal,
            model_snake = model_snake,
            model_module = model_module,
            models_path = models_path,
            factories_path = factories_path,
            count = count,
        )
    }

    /// Generate a basic seeder
    fn generate_basic_seeder(&self, seeder_name: &str) -> String {
        format!(
            r#"//! {} Seeder
//!
//! Custom database seeder.

use tideorm::prelude::*;

/// {}
#[derive(Default)]
pub struct {};

#[async_trait]
impl Seed for {} {{
    fn name(&self) -> &str {{
        "{}"
    }}

    async fn run(&self, _db: &Database) -> tideorm::Result<()> {{
        println!("Running {}...");

        // TODO: Add your seeding logic here
        // Example:
        // 
        // let user = User {{
        //     name: "Admin".to_string(),
        //     email: "admin@example.com".to_string(),
        //     ..Default::default()
        // }};
        // user.save().await?;

        println!("{} completed!");
        Ok(())
    }}
}}

#[cfg(test)]
mod tests {{
    use super::*;

    #[tokio::test]
    async fn test_seeder() {{
        // Set up test database
        // Run seeder
        // Verify records were created
    }}
}}
"#,
            seeder_name,
            seeder_name,
            seeder_name,
            seeder_name,
            to_snake_case(seeder_name),
            seeder_name,
            seeder_name
        )
    }

    /// Update mod.rs with new seeder
    fn update_mod_file(&self, seeder_name: &str) -> Result<(), String> {
        let mod_path = format!("{}/mod.rs", self.seeders_dir());
        let module_name = to_snake_case(seeder_name);

        let existing = std::fs::read_to_string(&mod_path).unwrap_or_default();

        let module_decl = format!("pub mod {};", module_name);
        if existing.contains(&module_decl) {
            return Ok(());
        }

        let new_content = format!("{}{}\n", existing, module_decl);

        std::fs::write(&mod_path, new_content)
            .map_err(|e| format!("Failed to update mod.rs: {}", e))?;

        Ok(())
    }
}

/// Convert string to PascalCase
fn to_pascal_case(s: &str) -> String {
    heck::AsPascalCase(s).to_string()
}

#[cfg(test)]
mod tests {
    use super::SeederGenerator;
    use crate::config::TideConfig;

    #[test]
    fn model_seeder_uses_global_db_helper_without_double_reference() {
        let config = TideConfig::default();
        let generator = SeederGenerator::new(&config);
        let content = generator.generate_model_seeder("UserSeeder", "User", 10);

        assert!(content.contains("Self::default().run(db()).await"));
        assert!(!content.contains("run(&db())"));
    }

    #[test]
    fn model_seeder_imports_from_the_configured_models_path() {
        let mut config = TideConfig::default();
        let generator = SeederGenerator::new(&config);
        let content = generator.generate_model_seeder("UserSeeder", "User", 10);
        assert!(content.contains("use crate::models::user::User;"));

        config.paths.models = "src/domain/models".to_string();
        config.paths.factories = "src/domain/factories".to_string();
        let generator = SeederGenerator::new(&config);
        let content = generator.generate_model_seeder("UserSeeder", "User", 10);

        assert!(content.contains("use crate::domain::models::user::User;"));
        assert!(!content.contains("use crate::models::"));
        assert!(content.contains("// crate::domain::factories::user_factory::UserFactory"));
    }

    #[test]
    fn seeder_output_dir_overrides_the_configured_path() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("custom").to_string_lossy().into_owned();

        let mut config = TideConfig::default();
        config.paths.seeders = dir.path().join("configured").to_string_lossy().into_owned();

        let path = SeederGenerator::new(&config)
            .output_dir(Some(&output))
            .generate("Demo", None, 1)
            .unwrap();

        assert!(path.starts_with(&output), "{}", path);
    }
}
