//! Controller/Handler generator for TideORM CLI

use crate::config::TideConfig;
use crate::utils::{ensure_directory, to_pascal_case, to_snake_case, pluralize};

/// Controller generator
pub struct ControllerGenerator;

impl ControllerGenerator {
    /// Create a new controller generator
    pub fn new(_config: &TideConfig) -> Self {
        Self
    }

    /// Generate a controller file
    pub fn generate(
        &self,
        name: &str,
        model: Option<String>,
        resource: bool,
    ) -> Result<String, String> {
        let handlers_path = "src/handlers";
        ensure_directory(handlers_path)?;

        let controller_name = if name.ends_with("Controller") || name.ends_with("Handler") {
            to_pascal_case(name)
        } else {
            format!("{}Handler", to_pascal_case(name))
        };

        let file_name = format!("{}.rs", to_snake_case(&controller_name));
        let file_path = format!("{}/{}", handlers_path, file_name);

        let content = if resource {
            if let Some(model_name) = model {
                self.generate_resource_controller(&controller_name, &model_name)
            } else {
                return Err("Resource controller requires a model".to_string());
            }
        } else if let Some(model_name) = model {
            self.generate_model_controller(&controller_name, &model_name)
        } else {
            self.generate_basic_controller(&controller_name)
        };

        std::fs::write(&file_path, content)
            .map_err(|e| format!("Failed to write controller file: {}", e))?;

        // Update mod.rs
        self.update_mod_file(handlers_path, &controller_name)?;

        Ok(file_path)
    }

    /// Generate a full CRUD resource controller
    fn generate_resource_controller(&self, handler_name: &str, model_name: &str) -> String {
        let model_pascal = to_pascal_case(model_name);
        let model_snake = to_snake_case(model_name);
        let model_plural = pluralize(&model_snake);

        format!(
            r#"//! {} - CRUD Handler for {}
//!
//! Provides full CRUD operations for the {} model.

use tideorm::prelude::*;
use crate::models::{model_pascal};
use serde::{{Deserialize, Serialize}};

/// Request payload for creating a {model_pascal}
#[derive(Debug, Deserialize)]
pub struct Create{model_pascal}Request {{
    // TODO: Add fields
    // pub name: String,
    // pub email: String,
}}

/// Request payload for updating a {model_pascal}
#[derive(Debug, Deserialize)]
pub struct Update{model_pascal}Request {{
    // TODO: Add fields
    // pub name: Option<String>,
    // pub email: Option<String>,
}}

/// Response payload for a {model_pascal}
#[derive(Debug, Serialize)]
pub struct {model_pascal}Response {{
    pub id: i64,
    // TODO: Add fields
    // pub name: String,
    // pub email: String,
    // pub created_at: String,
}}

impl From<{model_pascal}> for {model_pascal}Response {{
    fn from({model_snake}: {model_pascal}) -> Self {{
        Self {{
            id: {model_snake}.id,
            // TODO: Map fields
        }}
    }}
}}

/// {handler_name} - Handles {model_pascal} CRUD operations
pub struct {handler_name};

impl {handler_name} {{
    // =========================================================================
    // INDEX - List all {model_plural}
    // =========================================================================

    /// List all {model_plural}
    /// 
    /// GET /{model_plural}
    pub async fn index() -> tideorm::Result<Vec<{model_pascal}Response>> {{
        let {model_plural} = {model_pascal}::all().await?;
        Ok({model_plural}.into_iter().map(|r| r.into()).collect())
    }}

    /// List {model_plural} with pagination
    /// 
    /// GET /{model_plural}?page=1&per_page=10
    pub async fn index_paginated(page: u64, per_page: u64) -> tideorm::Result<(u64, Vec<{model_pascal}Response>)> {{
        let (total, {model_plural}) = {model_pascal}::paginate(page, per_page).await?;
        let responses = {model_plural}.into_iter().map(|r| r.into()).collect();
        Ok((total, responses))
    }}

    // =========================================================================
    // SHOW - Get a single {model_snake}
    // =========================================================================

    /// Get a single {model_snake} by ID
    /// 
    /// GET /{model_plural}/{{id}}
    pub async fn show(id: i64) -> tideorm::Result<Option<{model_pascal}Response>> {{
        let {model_snake} = {model_pascal}::find(id).await?;
        Ok({model_snake}.map(|r| r.into()))
    }}

    /// Get a {model_snake} by token
    /// 
    /// GET /{model_plural}/token/{{token}}
    #[cfg(feature = "tokenize")]
    pub async fn show_by_token(token: &str) -> tideorm::Result<{model_pascal}Response> {{
        let {model_snake} = {model_pascal}::from_token(token).await?;
        Ok({model_snake}.into())
    }}

    // =========================================================================
    // CREATE - Create a new {model_snake}
    // =========================================================================

    /// Create a new {model_snake}
    /// 
    /// POST /{model_plural}
    pub async fn create(request: Create{model_pascal}Request) -> tideorm::Result<{model_pascal}Response> {{
        let mut {model_snake} = {model_pascal} {{
            id: 0,
            // TODO: Map from request
            // name: request.name,
            // email: request.email,
            ..Default::default()
        }};

        {model_snake}.save().await?;
        Ok({model_snake}.into())
    }}

    // =========================================================================
    // UPDATE - Update an existing {model_snake}
    // =========================================================================

    /// Update an existing {model_snake}
    /// 
    /// PUT /{model_plural}/{{id}}
    pub async fn update(id: i64, request: Update{model_pascal}Request) -> tideorm::Result<Option<{model_pascal}Response>> {{
        let Some(mut {model_snake}) = {model_pascal}::find(id).await? else {{
            return Ok(None);
        }};

        // TODO: Update fields from request
        // if let Some(name) = request.name {{
        //     {model_snake}.name = name;
        // }}

        {model_snake}.save().await?;
        Ok(Some({model_snake}.into()))
    }}

    // =========================================================================
    // DELETE - Delete a {model_snake}
    // =========================================================================

    /// Delete a {model_snake}
    /// 
    /// DELETE /{model_plural}/{{id}}
    pub async fn destroy(id: i64) -> tideorm::Result<bool> {{
        let Some({model_snake}) = {model_pascal}::find(id).await? else {{
            return Ok(false);
        }};

        {model_snake}.delete().await?;
        Ok(true)
    }}

    // =========================================================================
    // BULK OPERATIONS
    // =========================================================================

    /// Delete multiple {model_plural}
    /// 
    /// DELETE /{model_plural}
    pub async fn destroy_many(ids: Vec<i64>) -> tideorm::Result<u64> {{
        let count = {model_pascal}::where_in("id", ids)
            .delete()
            .await?;
        Ok(count)
    }}
}}

#[cfg(test)]
mod tests {{
    use super::*;

    #[tokio::test]
    async fn test_index() {{
        // TODO: Set up test database
        // let result = {handler_name}::index().await;
        // assert!(result.is_ok());
    }}

    #[tokio::test]
    async fn test_create() {{
        // TODO: Set up test database
        // let request = Create{model_pascal}Request {{ /* fields */ }};
        // let result = {handler_name}::create(request).await;
        // assert!(result.is_ok());
    }}
}}
"#,
            handler_name,
            model_name,
            model_name,
            handler_name = handler_name,
            model_pascal = model_pascal,
            model_snake = model_snake,
            model_plural = model_plural,
        )
    }

    /// Generate a simple model controller
    fn generate_model_controller(&self, handler_name: &str, model_name: &str) -> String {
        let model_pascal = to_pascal_case(model_name);
        let model_snake = to_snake_case(model_name);

        format!(
            r#"//! {} - Handler for {}

use tideorm::prelude::*;
use crate::models::{model_pascal};

/// {} handler
pub struct {handler_name};

impl {handler_name} {{
    /// Get all {model_snake}s
    pub async fn all() -> tideorm::Result<Vec<{model_pascal}>> {{
        {model_pascal}::all().await
    }}

    /// Find a {model_snake} by ID
    pub async fn find(id: i64) -> tideorm::Result<Option<{model_pascal}>> {{
        {model_pascal}::find(id).await
    }}

    /// Create a new {model_snake}
    pub async fn create({model_snake}: {model_pascal}) -> tideorm::Result<{model_pascal}> {{
        let mut record = {model_snake};
        record.save().await?;
        Ok(record)
    }}

    /// Update a {model_snake}
    pub async fn update(id: i64, updates: {model_pascal}) -> tideorm::Result<Option<{model_pascal}>> {{
        let Some(mut {model_snake}) = {model_pascal}::find(id).await? else {{
            return Ok(None);
        }};
        
        // TODO: Apply updates
        
        {model_snake}.save().await?;
        Ok(Some({model_snake}))
    }}

    /// Delete a {model_snake}
    pub async fn delete(id: i64) -> tideorm::Result<bool> {{
        let Some({model_snake}) = {model_pascal}::find(id).await? else {{
            return Ok(false);
        }};
        
        {model_snake}.delete().await?;
        Ok(true)
    }}
}}
"#,
            handler_name,
            model_name,
            model_name,
            handler_name = handler_name,
            model_pascal = model_pascal,
            model_snake = model_snake,
        )
    }

    /// Generate a basic controller
    fn generate_basic_controller(&self, handler_name: &str) -> String {
        format!(
            r#"//! {}
//!
//! Custom handler/controller.

use tideorm::prelude::*;

/// {}
pub struct {};

impl {} {{
    /// Example handler method
    pub async fn handle() -> tideorm::Result<String> {{
        // TODO: Implement handler logic
        Ok("Hello from {}!".to_string())
    }}
}}

#[cfg(test)]
mod tests {{
    use super::*;

    #[tokio::test]
    async fn test_handle() {{
        let result = {}::handle().await;
        assert!(result.is_ok());
    }}
}}
"#,
            handler_name, handler_name, handler_name, handler_name, handler_name, handler_name
        )
    }

    /// Update mod.rs with new handler
    fn update_mod_file(&self, handlers_path: &str, handler_name: &str) -> Result<(), String> {
        let mod_path = format!("{}/mod.rs", handlers_path);
        let module_name = to_snake_case(handler_name);

        // Create mod.rs if it doesn't exist
        if !std::path::Path::new(&mod_path).exists() {
            std::fs::write(&mod_path, "//! Request handlers\n")
                .map_err(|e| format!("Failed to create mod.rs: {}", e))?;
        }

        let existing = std::fs::read_to_string(&mod_path).unwrap_or_default();

        let module_decl = format!("pub mod {};", module_name);
        if existing.contains(&module_decl) {
            return Ok(());
        }

        let new_content = format!(
            "{}{}\npub use {}::{};\n",
            existing, module_decl, module_name, handler_name
        );

        std::fs::write(&mod_path, new_content)
            .map_err(|e| format!("Failed to update mod.rs: {}", e))?;

        Ok(())
    }
}
