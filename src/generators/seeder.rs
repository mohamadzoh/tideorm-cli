//! Seeder generator for TideORM CLI

use crate::config::TideConfig;
use crate::utils::{ensure_directory, to_snake_case};

/// Seeder generator
pub struct SeederGenerator<'a> {
    config: &'a TideConfig,
}

impl<'a> SeederGenerator<'a> {
    /// Create a new seeder generator
    pub fn new(config: &'a TideConfig) -> Self {
        Self { config }
    }

    /// Generate a seeder file
    pub fn generate(
        &self,
        name: &str,
        model: Option<String>,
        count: u32,
    ) -> Result<String, String> {
        ensure_directory(&self.config.paths.seeders)?;

        let seeder_name = if name.ends_with("Seeder") {
            to_pascal_case(name)
        } else {
            format!("{}Seeder", to_pascal_case(name))
        };

        let file_name = format!("{}.rs", to_snake_case(&seeder_name));
        let file_path = format!("{}/{}", self.config.paths.seeders, file_name);

        let content = if let Some(model_name) = model {
            self.generate_model_seeder(&seeder_name, &model_name, count)
        } else {
            self.generate_basic_seeder(&seeder_name)
        };

        std::fs::write(&file_path, content)
            .map_err(|e| format!("Failed to write seeder file: {}", e))?;

        // Update mod.rs
        self.update_mod_file(&seeder_name)?;

        Ok(file_path)
    }

    /// Generate a seeder for a specific model
    fn generate_model_seeder(&self, seeder_name: &str, model_name: &str, count: u32) -> String {
        let model_snake = to_snake_case(model_name);
        let model_pascal = to_pascal_case(model_name);

        format!(
            r#"//! {} Seeder
//!
//! Seeds the database with {} records.

use tideorm::prelude::*;
use crate::models::{model_pascal};

/// {} seeder
pub struct {seeder_name};

impl {seeder_name} {{
    /// Run the seeder
    pub async fn run() -> tideorm::Result<()> {{
        println!("Seeding {model_snake}s...");

        for i in 1..={count} {{
            let {model_snake} = {model_pascal} {{
                id: 0, // Will be auto-generated
                // TODO: Fill in the model fields
                // Example:
                // name: format!("{model_pascal} {{}}", i),
                // email: format!("{model_snake}{{}}@example.com", i),
                ..Default::default()
            }};

            {model_snake}.save().await?;
        }}

        println!("Seeded {count} {model_snake}(s)");
        Ok(())
    }}

    /// Run the seeder with a factory
    pub async fn run_with_factory() -> tideorm::Result<()> {{
        println!("Seeding {model_snake}s with factory...");

        // TODO: Use factory pattern
        // Example:
        // {model_pascal}Factory::create_many({count}).await?;

        Self::run().await
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
pub struct {};

impl {} {{
    /// Run the seeder
    pub async fn run() -> tideorm::Result<()> {{
        println!("Running {}...");

        // TODO: Add your seeding logic here
        // Example:
        // 
        // let user = User {{
        //     id: 0,
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
            seeder_name, seeder_name, seeder_name, seeder_name, seeder_name, seeder_name
        )
    }

    /// Update mod.rs with new seeder
    fn update_mod_file(&self, seeder_name: &str) -> Result<(), String> {
        let mod_path = format!("{}/mod.rs", self.config.paths.seeders);
        let module_name = to_snake_case(seeder_name);

        let existing = std::fs::read_to_string(&mod_path).unwrap_or_default();

        let module_decl = format!("pub mod {};", module_name);
        if existing.contains(&module_decl) {
            return Ok(());
        }

        let new_content = format!(
            "{}{}\npub use {}::{};\n",
            existing, module_decl, module_name, seeder_name
        );

        std::fs::write(&mod_path, new_content)
            .map_err(|e| format!("Failed to update mod.rs: {}", e))?;

        Ok(())
    }
}

/// Convert string to PascalCase
fn to_pascal_case(s: &str) -> String {
    heck::AsPascalCase(s).to_string()
}
