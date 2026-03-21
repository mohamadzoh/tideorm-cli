//! Factory generator for TideORM CLI

use crate::config::TideConfig;
use crate::utils::{ensure_directory, to_snake_case};

/// Factory generator
pub struct FactoryGenerator<'a> {
    config: &'a TideConfig,
}

impl<'a> FactoryGenerator<'a> {
    /// Create a new factory generator
    pub fn new(config: &'a TideConfig) -> Self {
        Self { config }
    }

    /// Generate a factory file
    pub fn generate(&self, name: &str, model: Option<String>) -> Result<String, String> {
        ensure_directory(&self.config.paths.factories)?;

        let factory_name = if name.ends_with("Factory") {
            to_pascal_case(name)
        } else {
            format!("{}Factory", to_pascal_case(name))
        };

        let file_name = format!("{}.rs", to_snake_case(&factory_name));
        let file_path = format!("{}/{}", self.config.paths.factories, file_name);

        let model_name = model.unwrap_or_else(|| {
            factory_name.strip_suffix("Factory").unwrap_or(&factory_name).to_string()
        });

        let content = self.generate_factory(&factory_name, &model_name);

        std::fs::write(&file_path, content)
            .map_err(|e| format!("Failed to write factory file: {}", e))?;

        // Update mod.rs
        self.update_mod_file(&factory_name)?;

        Ok(file_path)
    }

    /// Generate factory content
    fn generate_factory(&self, factory_name: &str, model_name: &str) -> String {
        let model_pascal = to_pascal_case(model_name);
        let model_snake = to_snake_case(model_name);

        format!(
            r#"//! {} Factory
//!
//! Factory for creating {} instances for testing and seeding.

use tideorm::prelude::*;
use crate::models::{model_snake}::{model_pascal};

/// Factory for creating {model_pascal} instances
pub struct {factory_name};

impl {factory_name} {{
    /// Create a new {model_pascal} with default values
    pub fn definition() -> {model_pascal} {{
        {model_pascal} {{
            // TODO: Add default field values
            // Example:
            // name: "Example".to_string(),
            // email: "example@example.com".to_string(),
            ..Default::default()
        }}
    }}

    /// Create and save a single {model_pascal}
    pub async fn create() -> tideorm::Result<{model_pascal}> {{
        Self::definition().save().await
    }}

    /// Create and save multiple {model_pascal}s
    pub async fn create_many(count: usize) -> tideorm::Result<Vec<{model_pascal}>> {{
        let mut records = Vec::with_capacity(count);
        for _ in 0..count {{
            records.push(Self::create().await?);
        }}
        Ok(records)
    }}

    /// Create a {model_pascal} without saving
    pub fn make() -> {model_pascal} {{
        Self::definition()
    }}

    /// Create multiple {model_pascal}s without saving
    pub fn make_many(count: usize) -> Vec<{model_pascal}> {{
        (0..count).map(|_| Self::definition()).collect()
    }}

    /// Create a {model_pascal} with custom attributes
    pub fn with<F>(modifier: F) -> {model_pascal}
    where
        F: FnOnce(&mut {model_pascal}),
    {{
        let mut {model_snake} = Self::definition();
        modifier(&mut {model_snake});
        {model_snake}
    }}

    /// Create and save a {model_pascal} with custom attributes
    pub async fn create_with<F>(modifier: F) -> tideorm::Result<{model_pascal}>
    where
        F: FnOnce(&mut {model_pascal}),
    {{
        Self::with(modifier).save().await
    }}

}}

#[cfg(test)]
mod tests {{
    use super::*;

    #[test]
    fn test_make() {{
        let _{model_snake} = {factory_name}::make();
    }}

    #[test]
    fn test_make_many() {{
        let records = {factory_name}::make_many(5);
        assert_eq!(records.len(), 5);
    }}

    #[test]
    fn test_with_modifier() {{
        let _{model_snake} = {factory_name}::with(|_r| {{}});
    }}
}}
"#,
            factory_name,
            model_name,
            model_pascal = model_pascal,
            model_snake = model_snake,
            factory_name = factory_name,
        )
    }

    /// Update mod.rs with new factory
    fn update_mod_file(&self, factory_name: &str) -> Result<(), String> {
        let mod_path = format!("{}/mod.rs", self.config.paths.factories);
        let module_name = to_snake_case(factory_name);

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
