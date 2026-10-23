//! Factory generator for TideORM CLI

use crate::config::TideConfig;
use crate::utils::{
    append_source_block, crate_module_path, ensure_directory, ensure_writable, escape_ident,
    has_module_declaration, read_source_or_empty, to_snake_case, validate_rust_type_name,
};

/// Factory generator
pub struct FactoryGenerator<'a> {
    config: &'a TideConfig,
    output_dir: Option<String>,
}

impl<'a> FactoryGenerator<'a> {
    /// Create a new factory generator
    pub fn new(config: &'a TideConfig) -> Self {
        Self {
            config,
            output_dir: None,
        }
    }

    /// Override the directory factories are written to (the `--output` flag).
    ///
    /// `None` keeps the configured `[paths] factories` directory.
    ///
    /// This only moves the generated file: the model import is still derived from
    /// `[paths] models`, so relocating a factory does not break it.
    pub fn output_dir(mut self, dir: Option<&str>) -> Self {
        self.output_dir = dir
            .map(str::trim)
            .filter(|dir| !dir.is_empty())
            .map(ToOwned::to_owned);
        self
    }

    /// Directory the generated factory and its `mod.rs` belong to.
    fn factories_dir(&self) -> &str {
        self.output_dir
            .as_deref()
            .unwrap_or(&self.config.paths.factories)
    }

    /// Generate a factory file
    pub fn generate(&self, name: &str, model: Option<String>) -> Result<String, String> {
        if name.trim().is_empty() {
            return Err("Factory name is required".to_string());
        }

        let factory_name = if name.ends_with("Factory") {
            to_pascal_case(name)
        } else {
            format!("{}Factory", to_pascal_case(name))
        };
        validate_rust_type_name("factory", &factory_name)?;

        let file_name = format!("{}.rs", to_snake_case(&factory_name));
        let file_path = format!("{}/{}", self.factories_dir(), file_name);

        let model_name = model.unwrap_or_else(|| {
            factory_name
                .strip_suffix("Factory")
                .unwrap_or(&factory_name)
                .to_string()
        });
        validate_rust_type_name("model", &to_pascal_case(&model_name))?;
        ensure_directory(self.factories_dir())?;

        let content = self.generate_factory(&factory_name, &model_name);

        ensure_writable(&file_path)?;

        std::fs::write(&file_path, content)
            .map_err(|e| format!("Failed to write factory file: {}", e))?;

        // Update mod.rs
        self.update_mod_file(&factory_name)?;

        Ok(file_path)
    }

    /// Generate factory content
    ///
    /// The model is imported through the configured `[paths] models` directory rather than
    /// a hardcoded `crate::models`, so a project that keeps its models elsewhere still gets
    /// a factory that compiles.
    fn generate_factory(&self, factory_name: &str, model_name: &str) -> String {
        let model_pascal = to_pascal_case(model_name);
        let model_snake = to_snake_case(model_name);
        let model_module = escape_ident(&model_snake);
        let models_path = crate_module_path(&self.config.paths.models);

        format!(
            r#"//! {} Factory
//!
//! Factory for creating {} instances for testing and seeding.

use tideorm::prelude::*;
use {models_path}::{model_module}::{model_pascal};

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
        let mut record = Self::definition();
        modifier(&mut record);
        record
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
            model_module = model_module,
            models_path = models_path,
            factory_name = factory_name,
        )
    }

    /// Update mod.rs with new factory
    fn update_mod_file(&self, factory_name: &str) -> Result<(), String> {
        let mod_path = format!("{}/mod.rs", self.factories_dir());
        let module_name = to_snake_case(factory_name);

        let existing = read_source_or_empty(&mod_path)?;

        if has_module_declaration(&existing, &module_name) {
            return Ok(());
        }

        let module_decl = format!("pub mod {};", module_name);
        let new_content = append_source_block(&existing, &module_decl);

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
    use super::FactoryGenerator;
    use crate::config::TideConfig;

    #[test]
    fn factory_imports_from_the_configured_models_path() {
        let mut config = TideConfig::default();
        let generator = FactoryGenerator::new(&config);
        let content = generator.generate_factory("UserFactory", "User");
        assert!(content.contains("use crate::models::user::User;"));

        config.paths.models = "src/domain/models".to_string();
        let generator = FactoryGenerator::new(&config);
        let content = generator.generate_factory("UserFactory", "User");

        assert!(content.contains("use crate::domain::models::user::User;"));
        assert!(!content.contains("use crate::models::"));
    }

    /// The record was bound to the model's snake-cased name, which for `Match` is
    /// `let mut match`, and the factory did not compile.
    #[test]
    fn factory_for_a_keyword_named_model_compiles() {
        let config = TideConfig::default();
        let content = FactoryGenerator::new(&config).generate_factory("MatchFactory", "Match");

        assert!(content.contains("use crate::models::r#match::Match;"));
        assert!(content.contains("let mut record = Self::definition();"));
        assert!(!content.contains("let mut match"), "{content}");
    }

    #[test]
    fn factory_output_dir_overrides_the_configured_path() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("custom").to_string_lossy().into_owned();

        let mut config = TideConfig::default();
        config.paths.factories = dir.path().join("configured").to_string_lossy().into_owned();

        let path = FactoryGenerator::new(&config)
            .output_dir(Some(&output))
            .generate("Demo", None)
            .unwrap();

        assert!(path.starts_with(&output), "{}", path);
    }
}
