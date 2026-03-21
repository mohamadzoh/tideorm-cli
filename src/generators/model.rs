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
    ensure_directory, pluralize, render_template, to_pascal_case, to_snake_case,
    FieldDefinition, RelationDefinition, RelationType,
};
use serde::Serialize;

/// Model generator
pub struct ModelGenerator<'a> {
    config: &'a TideConfig,
    name: String,
    table: Option<String>,
    fields: Vec<FieldDefinition>,
    relations: Vec<RelationDefinition>,
    parse_errors: Vec<String>,
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
            parse_errors: Vec::new(),
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
            for field in fields_str.split(',') {
                let field = field.trim();
                if field.is_empty() {
                    continue;
                }

                match FieldDefinition::parse(field) {
                    Ok(parsed) => self.fields.push(parsed),
                    Err(err) => self.parse_errors.push(err),
                }
            }
        }
        self
    }

    /// Set relations from string
    pub fn relations(mut self, relations: Option<String>) -> Self {
        if let Some(relations_str) = relations {
            for relation in relations_str.split(',') {
                let relation = relation.trim();
                if relation.is_empty() {
                    continue;
                }

                match RelationDefinition::parse(relation) {
                    Ok(parsed) => self.relations.push(parsed),
                    Err(err) => self.parse_errors.push(err),
                }
            }
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

        if !self.parse_errors.is_empty() {
            return Err(self.parse_errors.join("\n"));
        }

        // Ensure output directory exists
        ensure_directory(&self.output_dir)?;

        // Generate file content
        let content = self.generate_content()?;

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
    fn generate_content(&self) -> Result<String, String> {
        let context = ModelTemplateContext {
            name: self.name.clone(),
            related_imports: self
                .relations
                .iter()
                .map(|relation| ModelImportContext {
                    module: to_snake_case(&relation.related_model),
                    name: relation.related_model.clone(),
                })
                .collect(),
            struct_attributes: self.build_struct_attributes(),
            struct_fields: self.build_struct_fields(),
            methods: self.build_impl_methods(),
        };

        render_template(
            "model",
            DEFAULT_MODEL_TEMPLATE,
            self.config.model.template.as_deref(),
            &context,
        )
    }

    fn build_struct_attributes(&self) -> Vec<String> {
        // Table name
        let table_name = self.table.clone().unwrap_or_else(|| {
            pluralize(&to_snake_case(&self.name))
        });

        let mut attributes = Vec::new();
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

        attributes.push(format!("#[tideorm::model({})]", tide_attrs.join(", ")));
        
        // Index attributes (struct-level)
        for field_name in &self.indexed {
            attributes.push(format!("#[index(\"{}\")]", field_name));
        }

        for field_name in self.generated_indexed_fields() {
            if !self.indexed.iter().any(|indexed| indexed == &field_name) {
                attributes.push(format!("#[index(\"{}\")]", field_name));
            }
        }
        
        // Unique index attributes (struct-level)
        for field_name in &self.unique {
            attributes.push(format!("#[unique_index(\"{}\")]", field_name));
        }

        for field_name in self.generated_unique_fields() {
            if !self.unique.iter().any(|unique| unique == &field_name) {
                attributes.push(format!("#[unique_index(\"{}\")]", field_name));
            }
        }

        attributes
    }

    fn build_struct_fields(&self) -> Vec<ModelFieldTemplateContext> {
        let mut fields = Vec::new();

        if !self.has_explicit_primary_key() {
            fields.push(ModelFieldTemplateContext {
                doc_comment: None,
                attribute: Some("#[tideorm(primary_key, auto_increment)]".to_string()),
                declaration: format!(
                    "pub {}: {},",
                    self.config.model.primary_key,
                    self.config.model.primary_key_type
                ),
            });
        }

        // Regular fields
        for field in self.generated_fields() {
            let mut field_attrs = Vec::new();
            let is_primary_key = field.primary_key || field.name == self.config.model.primary_key;
            let is_auto_increment = field.auto_increment
                || (is_primary_key && field.name == self.config.model.primary_key);

            // Check if this field should be nullable
            let is_nullable = field.nullable || self.nullable.contains(&field.name);

            if is_primary_key {
                field_attrs.push("primary_key".to_string());
            }

            if is_auto_increment {
                field_attrs.push("auto_increment".to_string());
            }

            if is_nullable {
                field_attrs.push("nullable".to_string());
            }

            if let Some(default) = &field.default {
                field_attrs.push(format!("default = \"{}\"", default));
            }

            let rust_type = if is_nullable && !field.nullable {
                format!("Option<{}>", field.rust_type().replace("Option<", "").replace(">", ""))
            } else {
                field.rust_type()
            };

            fields.push(ModelFieldTemplateContext {
                doc_comment: None,
                attribute: (!field_attrs.is_empty())
                    .then(|| format!("#[tideorm({})]", field_attrs.join(", "))),
                declaration: format!("pub {}: {},", field.name, rust_type),
            });
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
            
            fields.push(ModelFieldTemplateContext {
                doc_comment: None,
                attribute: Some(format!("#[tideorm({})]", rel_attr)),
                declaration: format!("pub {}: {},", rel.name, rel_type),
            });
        }
        
        if !self.translatable.is_empty() {
            fields.push(ModelFieldTemplateContext {
                doc_comment: Some("/// JSONB column for field translations".to_string()),
                attribute: None,
                declaration: "pub translations: Option<JsonValue>,".to_string(),
            });
        }

        // Single attachment fields (files JSONB column)
        if !self.attachments_single.is_empty() || !self.attachments_multi.is_empty() {
            fields.push(ModelFieldTemplateContext {
                doc_comment: Some("/// JSONB column for file attachments".to_string()),
                attribute: None,
                declaration: "pub files: Option<JsonValue>,".to_string(),
            });
        }

        // Timestamps (plain DateTime fields, no auto_now attributes)
        if self.timestamps {
            fields.push(ModelFieldTemplateContext {
                doc_comment: None,
                attribute: None,
                declaration: "pub created_at: chrono::DateTime<chrono::Utc>,".to_string(),
            });
            fields.push(ModelFieldTemplateContext {
                doc_comment: None,
                attribute: None,
                declaration: "pub updated_at: chrono::DateTime<chrono::Utc>,".to_string(),
            });
        }

        // Soft delete field
        if self.soft_deletes {
            fields.push(ModelFieldTemplateContext {
                doc_comment: None,
                attribute: None,
                declaration: "pub deleted_at: Option<chrono::DateTime<chrono::Utc>>,".to_string(),
            });
        }

        fields
    }

    fn build_impl_methods(&self) -> Vec<String> {
        let mut impl_lines = Vec::new();

        // Custom finder methods for unique fields
        for field in self.generated_fields() {
            if field.unique || self.unique.contains(&field.name) {
                let rust_type = self.finder_param_type(&field);
                impl_lines.push(format!(
                    r#"    /// Find by {}
    pub async fn find_by_{}({}: {}) -> tideorm::Result<Option<Self>> {{
        Self::query().where_eq("{}", {}).first().await
    }}
"#,
                    field.name,
                    field.name,
                    field.name,
                    rust_type,
                    field.name,
                    field.name
                ));
            }
        }

        impl_lines
    }

    fn finder_param_type(&self, field: &FieldDefinition) -> String {
        match field.field_type.to_lowercase().as_str() {
            "string" | "varchar" | "text" => "&str".to_string(),
            _ => field.rust_type().replace("Option<", "").replace(">", ""),
        }
    }

    fn generated_fields(&self) -> Vec<FieldDefinition> {
        let mut fields = self.fields.clone();

        for relation in &self.relations {
            if relation.relation_type != RelationType::BelongsTo {
                continue;
            }

            let foreign_key = relation.foreign_key.clone().unwrap_or_else(|| {
                format!("{}_id", to_snake_case(&relation.related_model))
            });

            if fields.iter().any(|field| field.name == foreign_key) {
                continue;
            }

            fields.push(FieldDefinition {
                name: foreign_key,
                field_type: self.config.model.primary_key_type.clone(),
                nullable: false,
                unique: false,
                indexed: true,
                primary_key: false,
                auto_increment: false,
                default: None,
            });
        }

        fields
    }

    fn generated_indexed_fields(&self) -> Vec<String> {
        self.generated_fields()
            .into_iter()
            .filter(|field| field.indexed)
            .map(|field| field.name)
            .collect()
    }

    fn generated_unique_fields(&self) -> Vec<String> {
        self.generated_fields()
            .into_iter()
            .filter(|field| field.unique)
            .map(|field| field.name)
            .collect()
    }

    fn has_explicit_primary_key(&self) -> bool {
        self.generated_fields().into_iter().any(|field| {
            field.primary_key || field.name == self.config.model.primary_key
        })
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

        let new_content = format!("{}{}\n", existing, module_decl);

        std::fs::write(&mod_path, new_content)
            .map_err(|e| format!("Failed to update mod.rs: {}", e))?;

        Ok(())
    }
}

const DEFAULT_MODEL_TEMPLATE: &str = r#"//! {{ name }} Model
//!
//! Auto-generated by TideORM CLI

use tideorm::prelude::*;
{% if related_imports %}

{% for import in related_imports %}
use super::{{ import.module }}::{{ import.name }};
{% endfor %}{% endif %}

{% for attribute in struct_attributes %}
{{ attribute }}
{% endfor %}
pub struct {{ name }} {
{% for field in struct_fields %}
{% if field.doc_comment %}    {{ field.doc_comment }}
{% endif %}{% if field.attribute %}    {{ field.attribute }}
{% endif %}    {{ field.declaration }}
{% endfor %}}

impl {{ name }} {
{% for method in methods %}
{{ method }}
{% endfor %}}
"#;

#[derive(Serialize)]
struct ModelTemplateContext {
    name: String,
    related_imports: Vec<ModelImportContext>,
    struct_attributes: Vec<String>,
    struct_fields: Vec<ModelFieldTemplateContext>,
    methods: Vec<String>,
}

#[derive(Serialize)]
struct ModelImportContext {
    module: String,
    name: String,
}

#[derive(Serialize)]
struct ModelFieldTemplateContext {
    doc_comment: Option<String>,
    attribute: Option<String>,
    declaration: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

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

        let field = FieldDefinition::parse("id:i64:primary_key:auto_increment").unwrap();
        assert!(field.primary_key);
        assert!(field.auto_increment);
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

        let content = generator.generate_content().unwrap();
        
        // Generated models should use the canonical TideORM model attribute
        assert!(content.contains("#[tideorm::model(table = \"users\")]") );
        assert!(!content.contains("#[derive(tideorm::Model)]"));
        
        // Should use struct-level #[index()] and #[unique_index()]
        assert!(content.contains("#[index(\"email\")]"));
        assert!(content.contains("#[unique_index(\"email\")]"));
        
        // Should NOT use legacy auto timestamp attributes
        assert!(!content.contains("auto_now_add"));
        assert!(!content.contains("auto_now"));
        
        // Timestamps should be plain DateTime fields
        assert!(content.contains("pub created_at: chrono::DateTime<chrono::Utc>,"));
        assert!(content.contains("pub updated_at: chrono::DateTime<chrono::Utc>,"));
    }
    
    #[test]
    fn test_relations_as_struct_fields() {
        let config = TideConfig::default();
        let generator = ModelGenerator::new(&config)
            .name("User")
            .relations(Some("posts:has_many:Post,profile:has_one:Profile".to_string()));

        let content = generator.generate_content().unwrap();
        
        // Relations should be defined as struct fields with proper types
        assert!(content.contains("HasMany<Post>"));
        assert!(content.contains("HasOne<Profile>"));
        assert!(content.contains("#[tideorm(has_many = \"Post\""));
        assert!(content.contains("#[tideorm(has_one = \"Profile\""));
    }

    #[test]
    fn test_translatable_models_include_translations_column() {
        let config = TideConfig::default();
        let generator = ModelGenerator::new(&config)
            .name("Product")
            .fields(Some("name:string,description:text".to_string()))
            .translatable(Some("name,description".to_string()));

        let content = generator.generate_content().unwrap();

        assert!(content.contains("#[tideorm::model(table = \"products\", translatable = \"name,description\")]"));
        assert!(content.contains("pub translations: Option<JsonValue>,"));
    }

    #[test]
    fn test_string_unique_finders_use_str_slices() {
        let config = TideConfig::default();
        let generator = ModelGenerator::new(&config)
            .name("User")
            .fields(Some("email:string:unique".to_string()));

        let content = generator.generate_content().unwrap();

        assert!(content.contains("pub async fn find_by_email(email: &str) -> tideorm::Result<Option<Self>>"));
        assert!(!content.contains("pub async fn find_by_email(email: String)"));
        assert!(!content.contains("pub async fn find_by_email(email: &String)"));
    }

    #[test]
    fn test_belongs_to_generates_foreign_key_field() {
        let config = TideConfig::default();
        let generator = ModelGenerator::new(&config)
            .name("Post")
            .relations(Some("author:belongs_to:User".to_string()));

        let content = generator.generate_content().unwrap();

        assert!(content.contains("pub user_id: i64,"));
        assert!(content.contains("#[index(\"user_id\")]"));
        assert!(content.contains("pub author: BelongsTo<User>,"));
    }

    #[test]
    fn test_model_template_override_is_used() {
        let dir = tempdir().unwrap();
        let template_path = dir.path().join("model.rs.j2");
        std::fs::write(&template_path, "// custom model for {{ name }}\n").unwrap();

        let mut config = TideConfig::default();
        config.model.template = Some(template_path.to_string_lossy().into_owned());

        let generator = ModelGenerator::new(&config).name("User");
        let content = generator.generate_content().unwrap();

        assert_eq!(content, "// custom model for User");
    }
}
