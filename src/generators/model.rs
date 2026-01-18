//! Model generator for TideORM CLI
//!
//! Generates model files with full TideORM attributes including:
//! - Fields with types and modifiers
//! - Relations (HasOne, HasMany, BelongsTo) as struct fields
//! - Translatable fields (struct-level attribute)
//! - Attachments (struct-level has_one_files, has_many_files)
//! - Indexes and unique constraints (struct-level attributes)
//! - Soft deletes, timestamps, tokenization

use crate::config::TideConfig;
use crate::utils::{
    ensure_directory, to_pascal_case, to_snake_case, pluralize, 
    FieldDefinition, RelationDefinition, RelationType,
};

/// Model generator
pub struct ModelGenerator<'a> {
    config: &'a TideConfig,
    name: String,
    table: Option<String>,
    fields: Vec<FieldDefinition>,
    relations: Vec<RelationDefinition>,
    translatable: Vec<String>,
    attachments_single: Vec<String>,
    attachments_multi: Vec<String>,
    indexed: Vec<String>,
    unique: Vec<String>,
    nullable: Vec<String>,
    soft_deletes: bool,
    timestamps: bool,
    tokenize: bool,
    output_dir: String,
}

impl<'a> ModelGenerator<'a> {
    /// Create a new model generator
    pub fn new(config: &'a TideConfig) -> Self {
        Self {
            config,
            name: String::new(),
            table: None,
            fields: Vec::new(),
            relations: Vec::new(),
            translatable: Vec::new(),
            attachments_single: Vec::new(),
            attachments_multi: Vec::new(),
            indexed: Vec::new(),
            unique: Vec::new(),
            nullable: Vec::new(),
            soft_deletes: config.model.soft_deletes,
            timestamps: config.model.timestamps,
            tokenize: config.model.tokenize,
            output_dir: config.paths.models.clone(),
        }
    }

    /// Set the model name
    pub fn name(mut self, name: &str) -> Self {
        self.name = to_pascal_case(name);
        self
    }

    /// Set the table name
    pub fn table(mut self, table: Option<String>) -> Self {
        self.table = table;
        self
    }

    /// Set fields from string
    pub fn fields(mut self, fields: Option<String>) -> Self {
        if let Some(fields_str) = fields {
            self.fields = fields_str
                .split(',')
                .filter_map(|f| FieldDefinition::parse(f.trim()).ok())
                .collect();
        }
        self
    }

    /// Set relations from string
    pub fn relations(mut self, relations: Option<String>) -> Self {
        if let Some(relations_str) = relations {
            self.relations = relations_str
                .split(',')
                .filter_map(|r| RelationDefinition::parse(r.trim()).ok())
                .collect();
        }
        self
    }

    /// Set translatable fields
    pub fn translatable(mut self, fields: Option<String>) -> Self {
        if let Some(fields_str) = fields {
            self.translatable = fields_str
                .split(',')
                .map(|f| f.trim().to_string())
                .collect();
        }
        self
    }

    /// Set single attachment fields
    pub fn attachments_single(mut self, fields: Option<String>) -> Self {
        if let Some(fields_str) = fields {
            self.attachments_single = fields_str
                .split(',')
                .map(|f| f.trim().to_string())
                .collect();
        }
        self
    }

    /// Set multi attachment fields
    pub fn attachments_multi(mut self, fields: Option<String>) -> Self {
        if let Some(fields_str) = fields {
            self.attachments_multi = fields_str
                .split(',')
                .map(|f| f.trim().to_string())
                .collect();
        }
        self
    }

    /// Set indexed fields
    pub fn indexed(mut self, fields: Option<String>) -> Self {
        if let Some(fields_str) = fields {
            self.indexed = fields_str
                .split(',')
                .map(|f| f.trim().to_string())
                .collect();
        }
        self
    }

    /// Set unique fields
    pub fn unique(mut self, fields: Option<String>) -> Self {
        if let Some(fields_str) = fields {
            self.unique = fields_str
                .split(',')
                .map(|f| f.trim().to_string())
                .collect();
        }
        self
    }

    /// Set nullable fields
    pub fn nullable(mut self, fields: Option<String>) -> Self {
        if let Some(fields_str) = fields {
            self.nullable = fields_str
                .split(',')
                .map(|f| f.trim().to_string())
                .collect();
        }
        self
    }

    /// Enable/disable soft deletes
    pub fn soft_deletes(mut self, enabled: bool) -> Self {
        self.soft_deletes = enabled;
        self
    }

    /// Enable/disable timestamps
    pub fn timestamps(mut self, enabled: bool) -> Self {
        self.timestamps = enabled;
        self
    }

    /// Enable/disable tokenization
    pub fn tokenize(mut self, enabled: bool) -> Self {
        self.tokenize = enabled;
        self
    }

    /// Set output directory
    pub fn output_dir(mut self, dir: &str) -> Self {
        self.output_dir = dir.to_string();
        self
    }

    /// Generate the model file
    pub fn generate(&self) -> Result<String, String> {
        if self.name.is_empty() {
            return Err("Model name is required".to_string());
        }

        // Ensure output directory exists
        ensure_directory(&self.output_dir)?;

        // Generate file content
        let content = self.generate_content();

        // Write file
        let file_name = format!("{}.rs", to_snake_case(&self.name));
        let file_path = format!("{}/{}", self.output_dir, file_name);

        std::fs::write(&file_path, content)
            .map_err(|e| format!("Failed to write model file: {}", e))?;

        // Update mod.rs
        self.update_mod_file()?;

        Ok(file_path)
    }

    /// Generate the model file content
    fn generate_content(&self) -> String {
        let mut content = String::new();

        // Module documentation
        content.push_str(&format!(
            "//! {} Model\n//!\n//! Auto-generated by TideORM CLI\n\n",
            self.name
        ));

        // Imports
        content.push_str("use tideorm::prelude::*;\n");
        
        // Add chrono if we have timestamps or soft deletes
        if self.timestamps || self.soft_deletes {
            content.push_str("use chrono::{DateTime, Utc};\n");
        }

        content.push('\n');

        // Import related models for relations
        for rel in &self.relations {
            content.push_str(&format!(
                "use super::{}::{};\n",
                to_snake_case(&rel.related_model),
                rel.related_model
            ));
        }

        if !self.relations.is_empty() {
            content.push('\n');
        }

        // Struct definition
        content.push_str(&self.generate_struct());

        // Impl block for custom methods
        content.push_str(&self.generate_impl());

        content
    }

    /// Generate the struct definition
    fn generate_struct(&self) -> String {
        let mut lines = Vec::new();

        // Table name
        let table_name = self.table.clone().unwrap_or_else(|| {
            pluralize(&to_snake_case(&self.name))
        });

        // Build tide attributes (struct-level)
        let mut tide_attrs = vec![format!("table = \"{}\"", table_name)];
        
        if self.soft_deletes {
            tide_attrs.push("soft_delete".to_string());
        }
        
        if self.tokenize {
            tide_attrs.push("tokenize".to_string());
        }
        
        // Translatable fields (struct-level attribute)
        if !self.translatable.is_empty() {
            tide_attrs.push(format!("translatable = \"{}\"", self.translatable.join(",")));
        }
        
        // File attachments (struct-level attributes)
        if !self.attachments_single.is_empty() {
            tide_attrs.push(format!("has_one_files = \"{}\"", self.attachments_single.join(",")));
        }
        
        if !self.attachments_multi.is_empty() {
            tide_attrs.push(format!("has_many_files = \"{}\"", self.attachments_multi.join(",")));
        }

        // Model derive and tide attributes
        lines.push("#[tideorm::model]".to_string());
        lines.push(format!("#[tide({})]", tide_attrs.join(", ")));
        
        // Index attributes (struct-level)
        for field_name in &self.indexed {
            lines.push(format!("#[index(\"{}\")]", field_name));
        }
        
        // Unique index attributes (struct-level)
        for field_name in &self.unique {
            lines.push(format!("#[unique_index(\"{}\")]", field_name));
        }

        // Struct definition
        lines.push(format!("pub struct {} {{", self.name));

        // Primary key
        lines.push(format!(
            "    #[tide(primary_key, auto_increment)]\n    pub {}: {},",
            self.config.model.primary_key,
            self.config.model.primary_key_type
        ));

        // Regular fields
        for field in &self.fields {
            let mut field_attrs = Vec::new();

            // Check if this field should be nullable
            let is_nullable = field.nullable || self.nullable.contains(&field.name);

            if is_nullable {
                field_attrs.push("nullable".to_string());
            }

            if let Some(default) = &field.default {
                field_attrs.push(format!("default = \"{}\"", default));
            }

            // Build field line
            let mut field_line = String::new();
            
            if !field_attrs.is_empty() {
                field_line.push_str(&format!("    #[tide({})]\n", field_attrs.join(", ")));
            }

            let rust_type = if is_nullable && !field.nullable {
                format!("Option<{}>", field.rust_type().replace("Option<", "").replace(">", ""))
            } else {
                field.rust_type()
            };

            field_line.push_str(&format!("    pub {}: {},", field.name, rust_type));
            lines.push(field_line);
        }
        
        // Relation fields (SeaORM-style: defined inside the struct)
        for rel in &self.relations {
            let fk = rel.foreign_key.clone().unwrap_or_else(|| {
                match rel.relation_type {
                    RelationType::BelongsTo => format!("{}_id", to_snake_case(&rel.related_model)),
                    RelationType::HasOne | RelationType::HasMany => {
                        format!("{}_id", to_snake_case(&self.name))
                    }
                }
            });
            
            let (rel_attr, rel_type) = match rel.relation_type {
                RelationType::BelongsTo => (
                    format!("belongs_to = \"{}\", foreign_key = \"{}\"", rel.related_model, fk),
                    format!("BelongsTo<{}>", rel.related_model)
                ),
                RelationType::HasOne => (
                    format!("has_one = \"{}\", foreign_key = \"{}\"", rel.related_model, fk),
                    format!("HasOne<{}>", rel.related_model)
                ),
                RelationType::HasMany => (
                    format!("has_many = \"{}\", foreign_key = \"{}\"", rel.related_model, fk),
                    format!("HasMany<{}>", rel.related_model)
                ),
            };
            
            lines.push(format!(
                "    #[tide({})]\n    pub {}: {},",
                rel_attr, rel.name, rel_type
            ));
        }
        
        // Single attachment fields (files JSONB column)
        if !self.attachments_single.is_empty() || !self.attachments_multi.is_empty() {
            lines.push("    /// JSONB column for file attachments\n    pub files: Option<serde_json::Value>,".to_string());
        }

        // Timestamps (plain DateTime fields, no auto_now attributes)
        if self.timestamps {
            lines.push("    pub created_at: DateTime<Utc>,".to_string());
            lines.push("    pub updated_at: DateTime<Utc>,".to_string());
        }

        // Soft delete field
        if self.soft_deletes {
            lines.push("    pub deleted_at: Option<DateTime<Utc>>,".to_string());
        }

        lines.push("}".to_string());
        lines.push(String::new());

        lines.join("\n")
    }

    /// Generate impl block
    fn generate_impl(&self) -> String {
        let mut impl_lines = Vec::new();

        impl_lines.push(format!("impl {} {{", self.name));

        // Custom finder methods for unique fields
        for field in &self.fields {
            if field.unique || self.unique.contains(&field.name) {
                impl_lines.push(format!(
                    r#"    /// Find by {}
    pub async fn find_by_{}({}: &{}) -> tideorm::Result<Option<Self>> {{
        Self::where_eq("{}", {}).first().await
    }}
"#,
                    field.name,
                    field.name,
                    field.name,
                    field.rust_type().replace("Option<", "").replace(">", ""),
                    field.name,
                    field.name
                ));
            }
        }

        impl_lines.push("}".to_string());

        impl_lines.join("\n")
    }

    /// Update the mod.rs file to include the new model
    fn update_mod_file(&self) -> Result<(), String> {
        let mod_path = format!("{}/mod.rs", self.output_dir);
        let module_name = to_snake_case(&self.name);

        // Read existing content
        let existing = std::fs::read_to_string(&mod_path).unwrap_or_default();

        // Check if already included
        let module_decl = format!("pub mod {};", module_name);
        if existing.contains(&module_decl) {
            return Ok(());
        }

        // Add the module
        let new_content = format!("{}{}\npub use {}::{};\n", existing, module_decl, module_name, self.name);

        std::fs::write(&mod_path, new_content)
            .map_err(|e| format!("Failed to update mod.rs: {}", e))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_generator_basic() {
        let config = TideConfig::default();
        let generator = ModelGenerator::new(&config)
            .name("User")
            .fields(Some("name:string,email:string:unique".to_string()));

        assert_eq!(generator.name, "User");
        assert_eq!(generator.fields.len(), 2);
    }

    #[test]
    fn test_field_parsing() {
        let field = FieldDefinition::parse("email:string:unique:indexed").unwrap();
        assert_eq!(field.name, "email");
        assert_eq!(field.field_type, "string");
        assert!(field.unique);
        assert!(field.indexed);
    }

    #[test]
    fn test_relation_parsing() {
        let rel = RelationDefinition::parse("posts:has_many:Post").unwrap();
        assert_eq!(rel.name, "posts");
        assert_eq!(rel.related_model, "Post");
        assert_eq!(rel.relation_type, RelationType::HasMany);
    }
    
    #[test]
    fn test_generated_model_uses_correct_syntax() {
        let config = TideConfig::default();
        let generator = ModelGenerator::new(&config)
            .name("User")
            .timestamps(true)
            .indexed(Some("email".to_string()))
            .unique(Some("email".to_string()));

        let content = generator.generate_content();
        
        // Should use #[tideorm::model] not #[derive(Model)]
        assert!(content.contains("#[tideorm::model]"));
        
        // Should use struct-level #[index()] and #[unique_index()]
        assert!(content.contains("#[index(\"email\")]"));
        assert!(content.contains("#[unique_index(\"email\")]"));
        
        // Should NOT use #[tide(auto_now_add)] or #[tide(auto_now)]
        assert!(!content.contains("auto_now_add"));
        assert!(!content.contains("auto_now"));
        
        // Timestamps should be plain DateTime fields
        assert!(content.contains("pub created_at: DateTime<Utc>,"));
        assert!(content.contains("pub updated_at: DateTime<Utc>,"));
    }
    
    #[test]
    fn test_relations_as_struct_fields() {
        let config = TideConfig::default();
        let generator = ModelGenerator::new(&config)
            .name("User")
            .relations(Some("posts:has_many:Post,profile:has_one:Profile".to_string()));

        let content = generator.generate_content();
        
        // Relations should be defined as struct fields with proper types
        assert!(content.contains("HasMany<Post>"));
        assert!(content.contains("HasOne<Profile>"));
        assert!(content.contains("#[tide(has_many = \"Post\""));
        assert!(content.contains("#[tide(has_one = \"Profile\""));
    }
}
