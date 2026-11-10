//! The source of a generated model: its struct, attributes and finder methods.

use super::ModelGenerator;
use crate::utils::{
    FieldDefinition, RelationType, auto_increments, escape_ident, pluralize, render_template,
    rust_type_for_field_type, to_snake_case, validate_rust_type_name, validate_sql_identifier,
    validate_sql_object_name,
};
use serde::Serialize;

impl ModelGenerator<'_> {
    /// Generate the model file content
    pub(super) fn generate_content(&self) -> Result<String, String> {
        validate_rust_type_name("model", &self.name)?;

        // The table name is written into a `table = ".."` string literal, so a name
        // carrying a quote would inject arbitrary Rust into the generated model.
        validate_sql_object_name("table", &self.table_name())?;
        self.validate_generated_fields()?;

        for (kind, names) in [
            ("translatable field", &self.translatable),
            ("single attachment field", &self.attachments_single),
            ("multiple attachment field", &self.attachments_multi),
            ("indexed field", &self.indexed),
            ("unique field", &self.unique),
            ("nullable field", &self.nullable),
        ] {
            for name in names {
                validate_sql_identifier(kind, name)?;
            }
        }

        let context = ModelTemplateContext {
            name: self.name.clone(),
            related_imports: self.build_related_imports(),
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

    /// Collect the `use super::..` imports for related models.
    ///
    /// The same model may back several relations, and a self-relation refers to the struct
    /// being defined here, so both cases must be dropped or the file will not compile.
    fn build_related_imports(&self) -> Vec<ModelImportContext> {
        let mut imports: Vec<ModelImportContext> = Vec::new();

        for relation in &self.relations {
            if relation.related_model == self.name {
                continue;
            }

            if imports
                .iter()
                .any(|import| import.name == relation.related_model)
            {
                continue;
            }

            imports.push(ModelImportContext {
                module: escape_ident(&to_snake_case(&relation.related_model)),
                name: relation.related_model.clone(),
            });
        }

        imports
    }

    /// The table this model binds to, honouring an explicit `--table` override.
    fn table_name(&self) -> String {
        self.table
            .clone()
            .unwrap_or_else(|| pluralize(&to_snake_case(&self.name)))
    }

    fn build_struct_attributes(&self) -> Vec<String> {
        let table_name = self.table_name();

        let mut attributes = Vec::new();
        // A schema-qualified table is the model's `schema`: `table` is one name.
        let mut tide_attrs = match crate::utils::split_table(&table_name) {
            (Some(schema), table) => {
                vec![
                    format!("table = \"{table}\""),
                    format!("schema = \"{schema}\""),
                ]
            }
            (None, table) => vec![format!("table = \"{table}\"")],
        };

        if self.soft_deletes {
            tide_attrs.push("soft_delete".to_string());
        }

        if self.tokenize {
            tide_attrs.push("tokenize".to_string());
        }

        // Translatable fields (struct-level attribute)
        if !self.translatable.is_empty() {
            tide_attrs.push(format!(
                "translatable = \"{}\"",
                self.translatable.join(",")
            ));
        }

        // File attachments (struct-level attributes)
        if !self.attachments_single.is_empty() {
            tide_attrs.push(format!(
                "has_one_files = \"{}\"",
                self.attachments_single.join(",")
            ));
        }

        if !self.attachments_multi.is_empty() {
            tide_attrs.push(format!(
                "has_many_files = \"{}\"",
                self.attachments_multi.join(",")
            ));
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
            // TideORM numbers only integer keys; a `Uuid` key gets a random value instead.
            let attribute = if auto_increments(&self.config.model.primary_key_type) {
                "#[tideorm(primary_key, auto_increment)]"
            } else {
                "#[tideorm(primary_key)]"
            };
            fields.push(ModelFieldTemplateContext {
                doc_comment: None,
                attribute: Some(attribute.to_string()),
                // The configured type is a field type alias like every other one, so it has
                // to go through the same mapper: `bigint` is `i64`, not a Rust type.
                declaration: format!(
                    "pub {}: {},",
                    escape_ident(&self.config.model.primary_key),
                    rust_type_for_field_type(&self.config.model.primary_key_type)
                ),
            });
        }

        // Regular fields
        for field in self.generated_fields() {
            let mut field_attrs = Vec::new();
            let is_primary_key = field.primary_key || field.name == self.config.model.primary_key;
            // Auto increment is never inferred from the field name: the migration side only
            // emits it when the modifier was given, and the two must describe the same table.
            let is_auto_increment = field.auto_increment;

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

            // The migration names the column exactly as the field was given, while the
            // model derives one by snake-casing the field name, which splits digits off
            // (`sha256` becomes `sha_256`) and lowercases. Pin any name that could change.
            if !derives_itself_as_column(&field.name) {
                field_attrs.push(format!("column = \"{}\"", field.name));
            }

            let rust_type = field.rust_type_for(is_nullable);

            fields.push(ModelFieldTemplateContext {
                doc_comment: None,
                attribute: (!field_attrs.is_empty())
                    .then(|| format!("#[tideorm({})]", field_attrs.join(", "))),
                declaration: format!("pub {}: {},", escape_ident(&field.name), rust_type),
            });
        }

        // Relation fields (declared as struct fields)
        for rel in &self.relations {
            let fk = rel
                .foreign_key
                .clone()
                .unwrap_or_else(|| match rel.relation_type {
                    RelationType::BelongsTo => format!("{}_id", to_snake_case(&rel.related_model)),
                    RelationType::HasOne | RelationType::HasMany => {
                        format!("{}_id", to_snake_case(&self.name))
                    }
                });

            let (rel_attr, rel_type) = match rel.relation_type {
                RelationType::BelongsTo => (
                    format!(
                        "belongs_to = \"{}\", foreign_key = \"{}\"",
                        rel.related_model, fk
                    ),
                    format!("BelongsTo<{}>", rel.related_model),
                ),
                RelationType::HasOne => (
                    format!(
                        "has_one = \"{}\", foreign_key = \"{}\"",
                        rel.related_model, fk
                    ),
                    format!("HasOne<{}>", rel.related_model),
                ),
                RelationType::HasMany => (
                    format!(
                        "has_many = \"{}\", foreign_key = \"{}\"",
                        rel.related_model, fk
                    ),
                    format!("HasMany<{}>", rel.related_model),
                ),
            };

            fields.push(ModelFieldTemplateContext {
                doc_comment: None,
                attribute: Some(format!("#[tideorm({})]", rel_attr)),
                declaration: format!("pub {}: {},", escape_ident(&rel.name), rel_type),
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
                let param = escape_ident(&field.name);
                impl_lines.push(format!(
                    r#"    /// Find by {}
    pub async fn find_by_{}({}: {}) -> tideorm::Result<Option<Self>> {{
        Self::query().where_eq("{}", {}).first().await
    }}
"#,
                    field.name, field.name, param, rust_type, field.name, param
                ));
            }
        }

        impl_lines
    }

    fn finder_param_type(&self, field: &FieldDefinition) -> String {
        match field.field_type.to_lowercase().as_str() {
            "string" | "varchar" | "text" => "&str".to_string(),
            _ => field.base_rust_type(),
        }
    }
}

// The prelude stays for the code added to a model later, scopes, callbacks or
// queries, though a model of plain columns uses none of it yet.
const DEFAULT_MODEL_TEMPLATE: &str = r#"//! {{ name }} Model
//!
//! Auto-generated by TideORM CLI

#[allow(unused_imports)]
use tideorm::prelude::*;
{%- for import in related_imports %}
use super::{{ import.module }}::{{ import.name }};
{%- endfor %}
{% for attribute in struct_attributes %}
{{ attribute }}
{%- endfor %}
pub struct {{ name }} {
{%- for field in struct_fields %}
{%- if field.doc_comment %}
    {{ field.doc_comment }}
{%- endif %}
{%- if field.attribute %}
    {{ field.attribute }}
{%- endif %}
    {{ field.declaration }}
{%- endfor %}
}
{%- if methods %}

impl {{ name }} {
{%- for method in methods %}
{%- if not loop.first %}
{% endif %}
{{ method }}
{%- endfor %}
}
{%- endif %}

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

/// Whether TideORM derives exactly `name` as the column of a field called `name`:
/// lowercase ASCII words joined by single underscores, with no digits.
fn derives_itself_as_column(name: &str) -> bool {
    name.split('_')
        .all(|word| !word.is_empty() && word.bytes().all(|byte| byte.is_ascii_lowercase()))
}
