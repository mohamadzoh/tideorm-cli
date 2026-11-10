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
use crate::generators::Generated;
use crate::utils::{
    FieldDefinition, RelationDefinition, RelationType, declare_module, ensure_writable,
    split_definitions, to_pascal_case, to_snake_case,
};

mod source;

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
        let driver = &self.config.database.driver;
        if let Some(fields_str) = fields {
            for field in split_definitions(&fields_str, ',') {
                let field = field.trim();
                if field.is_empty() {
                    continue;
                }

                let parsed = FieldDefinition::parse(field)
                    .and_then(|parsed| parsed.check_driver_support(driver).map(|()| parsed));
                match parsed {
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

    /// Check and render the model file without writing anything.
    pub fn render(&self) -> Result<Generated<'_>, String> {
        if self.name.is_empty() {
            return Err("Model name is required".to_string());
        }

        if !self.parse_errors.is_empty() {
            return Err(self.parse_errors.join("\n"));
        }

        let content = self.generate_content()?;
        let file_stem = to_snake_case(&self.name);
        let file_path = format!("{}/{}.rs", self.output_dir, file_stem);

        ensure_writable(&file_path)?;

        Ok(Generated::new(file_path, content, move || {
            declare_module(&self.output_dir, &file_stem)
        }))
    }

    fn validate_generated_fields(&self) -> Result<(), String> {
        let fields = self.generated_fields();
        let primary_keys = fields
            .iter()
            .filter(|field| field.primary_key || field.name == self.config.model.primary_key)
            .count();
        if primary_keys > 1 {
            return Err("A model can define only one primary key field".to_string());
        }

        for (kind, names) in [
            ("translatable", &self.translatable),
            ("indexed", &self.indexed),
            ("unique", &self.unique),
            ("nullable", &self.nullable),
        ] {
            for name in names {
                if !fields.iter().any(|field| field.name == name.as_str()) {
                    return Err(format!(
                        "Unknown {kind} field '{}'; define it with --fields or a belongs_to relation",
                        name
                    ));
                }
            }
        }

        let mut names = Vec::new();
        let mut add = |name: &str| {
            if names
                .iter()
                .any(|existing: &String| existing.as_str() == name)
            {
                Err(format!(
                    "Duplicate generated model field '{}'; rename the field or disable the generated field that uses this name",
                    name
                ))
            } else {
                names.push(name.to_string());
                Ok(())
            }
        };

        for field in &fields {
            add(&field.name)?;
        }
        for relation in &self.relations {
            add(&relation.name)?;
        }
        if !self.has_explicit_primary_key() {
            add(&self.config.model.primary_key)?;
        }
        if !self.translatable.is_empty() {
            add("translations")?;
        }
        if !self.attachments_single.is_empty() || !self.attachments_multi.is_empty() {
            add("files")?;
        }
        if self.timestamps {
            add("created_at")?;
            add("updated_at")?;
        }
        if self.soft_deletes {
            add("deleted_at")?;
        }

        Ok(())
    }

    /// The declared fields as a generated seeder fills them: a field in the
    /// `--nullable` list is optional there too.
    pub fn seed_fields(&self) -> Vec<FieldDefinition> {
        self.fields
            .iter()
            .cloned()
            .map(|mut field| {
                field.nullable |= self.nullable.contains(&field.name);
                field.unique |= self.unique.contains(&field.name);
                field
            })
            .collect()
    }

    fn generated_fields(&self) -> Vec<FieldDefinition> {
        let mut fields = self.fields.clone();

        for relation in &self.relations {
            if relation.relation_type != RelationType::BelongsTo {
                continue;
            }

            let foreign_key = relation
                .foreign_key
                .clone()
                .unwrap_or_else(|| format!("{}_id", to_snake_case(&relation.related_model)));

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
        self.generated_fields()
            .into_iter()
            .any(|field| field.primary_key || field.name == self.config.model.primary_key)
    }
}

#[cfg(test)]
#[path = "../../tests/unit/generators_model_tests.rs"]
mod tests;
