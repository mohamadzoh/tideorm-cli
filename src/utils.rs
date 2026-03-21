//! Utility functions for TideORM CLI

use colored::Colorize;
use minijinja::{AutoEscape, Environment};
use serde::Serialize;
use std::path::Path;

/// Print a success message
pub fn print_success(message: &str) {
    println!("{} {}", "✓".green(), message);
}

/// Print an info message
pub fn print_info(message: &str) {
    println!("{} {}", "ℹ".blue(), message);
}

/// Print a warning message
pub fn print_warning(message: &str) {
    println!("{} {}", "⚠".yellow(), message);
}

/// Create a directory if it doesn't exist
pub fn ensure_directory(path: &str) -> Result<(), String> {
    let path = Path::new(path);
    if !path.exists() {
        std::fs::create_dir_all(path)
            .map_err(|e| format!("Failed to create directory '{}': {}", path.display(), e))?;
    }
    Ok(())
}

/// Check if a file exists
pub fn file_exists(path: &str) -> bool {
    Path::new(path).exists()
}

/// Render generator output from a MiniJinja template.
pub fn render_template<T: Serialize>(
    template_name: &str,
    default_template: &str,
    template_path: Option<&str>,
    context: &T,
) -> Result<String, String> {
    let source = match template_path.map(str::trim).filter(|path| !path.is_empty()) {
        Some(path) => std::fs::read_to_string(path)
            .map_err(|error| format!("Failed to read {} template '{}': {}", template_name, path, error))?,
        None => default_template.to_string(),
    };

    let mut env = Environment::new();
    env.set_auto_escape_callback(|_| AutoEscape::None);
    env.add_template(template_name, &source)
        .map_err(|error| format!("Failed to parse {} template: {}", template_name, error))?;

    env.get_template(template_name)
        .map_err(|error| format!("Failed to load {} template: {}", template_name, error))?
        .render(context)
        .map_err(|error| format!("Failed to render {} template: {}", template_name, error))
}

/// Generate a timestamp for migration names
pub fn migration_timestamp() -> String {
    chrono::Utc::now().format("%Y%m%d%H%M%S").to_string()
}

/// Convert a string to snake_case
pub fn to_snake_case(s: &str) -> String {
    heck::AsSnakeCase(s).to_string()
}

/// Convert a string to PascalCase
pub fn to_pascal_case(s: &str) -> String {
    heck::AsPascalCase(s).to_string()
}

/// Pluralize a word
pub fn pluralize(word: &str) -> String {
    pluralizer::pluralize(word, 2, false)
}

/// Singularize a word
#[cfg(test)]
pub fn singularize(word: &str) -> String {
    pluralizer::pluralize(word, 1, false)
}

/// Parse field definition string
/// Format: name:type[:modifier1:modifier2...]
#[derive(Debug, Clone)]
pub struct FieldDefinition {
    pub name: String,
    pub field_type: String,
    pub nullable: bool,
    pub unique: bool,
    pub indexed: bool,
    pub primary_key: bool,
    pub auto_increment: bool,
    pub default: Option<String>,
}

impl FieldDefinition {
    pub fn parse(s: &str) -> Result<Self, String> {
        let parts: Vec<&str> = s.split(':').collect();
        
        if parts.len() < 2 {
            return Err(format!(
                "Invalid field definition '{}'. Expected format: name:type[:modifiers]",
                s
            ));
        }

        let name = parts[0].trim().to_string();
        let field_type = parts[1].trim().to_string();
        let mut nullable = false;
        let mut unique = false;
        let mut indexed = false;
        let mut primary_key = false;
        let mut auto_increment = false;
        let mut default = None;

        // Parse modifiers
        for part in parts.iter().skip(2) {
            let part = part.trim().to_lowercase();
            match part.as_str() {
                "nullable" | "null" => nullable = true,
                "unique" | "uniq" => unique = true,
                "indexed" | "index" | "idx" => indexed = true,
                "primary_key" | "primary" | "pk" => primary_key = true,
                "auto_increment" | "autoincrement" | "increment" => auto_increment = true,
                _ if part.starts_with("default=") => {
                    default = Some(part.strip_prefix("default=").unwrap().to_string());
                }
                _ => {
                    return Err(format!("Unknown modifier: {}", part));
                }
            }
        }

        Ok(Self {
            name,
            field_type,
            nullable,
            unique,
            indexed,
            primary_key,
            auto_increment,
            default,
        })
    }

    /// Convert field type string to Rust type
    pub fn rust_type(&self) -> String {
        let base_type = match self.field_type.to_lowercase().as_str() {
            "string" | "varchar" | "text" => "String",
            "i8" | "tinyint" => "i8",
            "i16" | "smallint" => "i16",
            "i32" | "int" | "integer" => "i32",
            "i64" | "bigint" => "i64",
            "f32" | "float" => "f32",
            "f64" | "double" => "f64",
            "bool" | "boolean" => "bool",
            "datetime" | "timestamp" => "chrono::DateTime<chrono::Utc>",
            "date" => "chrono::NaiveDate",
            "time" => "chrono::NaiveTime",
            "uuid" => "uuid::Uuid",
            "json" => "Json",
            "jsonb" => "Jsonb",
            "decimal" => "rust_decimal::Decimal",
            "bytes" | "blob" | "binary" => "Vec<u8>",
            "int_array" | "integer_array" => "IntArray",
            "bigint_array" => "BigIntArray",
            "text_array" | "string_array" => "TextArray",
            "bool_array" | "boolean_array" => "BoolArray",
            "float_array" => "FloatArray",
            "json_array" => "JsonArray",
            _ => &self.field_type,
        };

        if self.nullable {
            format!("Option<{}>", base_type)
        } else {
            base_type.to_string()
        }
    }

    /// Convert to SQL type
    pub fn sql_type(&self, driver: &str) -> String {
        match (self.field_type.to_lowercase().as_str(), driver) {
            ("string" | "varchar", _) => "VARCHAR(255)".to_string(),
            ("text", _) => "TEXT".to_string(),
            ("i8" | "tinyint", "mysql") => "TINYINT".to_string(),
            ("i8" | "tinyint", _) => "SMALLINT".to_string(),
            ("i16" | "smallint", _) => "SMALLINT".to_string(),
            ("i32" | "int" | "integer", _) => "INTEGER".to_string(),
            ("i64" | "bigint", _) => "BIGINT".to_string(),
            ("f32" | "float", _) => "REAL".to_string(),
            ("f64" | "double", _) => "DOUBLE PRECISION".to_string(),
            ("bool" | "boolean", "mysql") => "TINYINT(1)".to_string(),
            ("bool" | "boolean", _) => "BOOLEAN".to_string(),
            ("datetime" | "timestamp", "postgres") => "TIMESTAMPTZ".to_string(),
            ("datetime" | "timestamp", _) => "DATETIME".to_string(),
            ("date", _) => "DATE".to_string(),
            ("time", _) => "TIME".to_string(),
            ("uuid", "postgres") => "UUID".to_string(),
            ("uuid", _) => "VARCHAR(36)".to_string(),
            ("json", "postgres") => "JSON".to_string(),
            ("jsonb", "postgres") => "JSONB".to_string(),
            ("json" | "jsonb", _) => "TEXT".to_string(),
            ("decimal", _) => "DECIMAL(19, 4)".to_string(),
            ("bytes" | "blob" | "binary", "postgres") => "BYTEA".to_string(),
            ("bytes" | "blob" | "binary", _) => "BLOB".to_string(),
            ("int_array" | "integer_array", "postgres") => "INTEGER[]".to_string(),
            ("bigint_array", "postgres") => "BIGINT[]".to_string(),
            ("text_array" | "string_array", "postgres") => "TEXT[]".to_string(),
            ("bool_array" | "boolean_array", "postgres") => "BOOLEAN[]".to_string(),
            ("float_array", "postgres") => "DOUBLE PRECISION[]".to_string(),
            ("json_array", "postgres") => "JSONB[]".to_string(),
            (
                "int_array"
                | "integer_array"
                | "bigint_array"
                | "text_array"
                | "string_array"
                | "bool_array"
                | "boolean_array"
                | "float_array"
                | "json_array",
                _,
            ) => "TEXT".to_string(),
            _ => self.field_type.to_uppercase(),
        }
    }
}

/// Parse relation definition string
/// Format: name:type:Model[:foreign_key]
#[derive(Debug, Clone)]
pub struct RelationDefinition {
    pub name: String,
    pub relation_type: RelationType,
    pub related_model: String,
    pub foreign_key: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RelationType {
    BelongsTo,
    HasOne,
    HasMany,
}

impl RelationDefinition {
    pub fn parse(s: &str) -> Result<Self, String> {
        let parts: Vec<&str> = s.split(':').collect();
        
        if parts.len() < 3 {
            return Err(format!(
                "Invalid relation definition '{}'. Expected format: name:type:Model[:foreign_key]",
                s
            ));
        }

        let name = parts[0].trim().to_string();
        let relation_type = match parts[1].trim().to_lowercase().as_str() {
            "belongs_to" | "belongsto" => RelationType::BelongsTo,
            "has_one" | "hasone" => RelationType::HasOne,
            "has_many" | "hasmany" => RelationType::HasMany,
            other => return Err(format!("Unknown relation type: {}", other)),
        };
        let related_model = parts[2].trim().to_string();
        let foreign_key = parts.get(3).map(|s| s.trim().to_string());

        Ok(Self {
            name,
            relation_type,
            related_model,
            foreign_key,
        })
    }
}

/// Confirm an action with the user
pub fn confirm(message: &str) -> bool {
    use dialoguer::Confirm;
    
    Confirm::new()
        .with_prompt(message)
        .default(false)
        .interact()
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pluralize() {
        assert_eq!(pluralize("user"), "users");
        assert_eq!(pluralize("company"), "companies");
        assert_eq!(pluralize("person"), "people");
        assert_eq!(pluralize("box"), "boxes");
        assert_eq!(pluralize("leaf"), "leaves");
    }

    #[test]
    fn test_singularize() {
        assert_eq!(singularize("users"), "user");
        assert_eq!(singularize("companies"), "company");
        assert_eq!(singularize("people"), "person");
        assert_eq!(singularize("boxes"), "box");
    }

    #[test]
    fn test_field_definition_parse() {
        let field = FieldDefinition::parse("name:string").unwrap();
        assert_eq!(field.name, "name");
        assert_eq!(field.field_type, "string");
        assert!(!field.nullable);

        let field = FieldDefinition::parse("age:i32:nullable").unwrap();
        assert_eq!(field.name, "age");
        assert_eq!(field.field_type, "i32");
        assert!(field.nullable);

        let field = FieldDefinition::parse("email:string:unique:indexed").unwrap();
        assert!(field.unique);
        assert!(field.indexed);

        let field = FieldDefinition::parse("id:i64:primary_key:auto_increment").unwrap();
        assert!(field.primary_key);
        assert!(field.auto_increment);
    }

    #[test]
    fn test_relation_definition_parse() {
        let rel = RelationDefinition::parse("posts:has_many:Post").unwrap();
        assert_eq!(rel.name, "posts");
        assert_eq!(rel.relation_type, RelationType::HasMany);
        assert_eq!(rel.related_model, "Post");

        let rel = RelationDefinition::parse("user:belongs_to:User:user_id").unwrap();
        assert_eq!(rel.name, "user");
        assert_eq!(rel.relation_type, RelationType::BelongsTo);
        assert_eq!(rel.foreign_key, Some("user_id".to_string()));
    }
}
