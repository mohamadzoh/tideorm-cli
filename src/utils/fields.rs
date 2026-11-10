//! The `--fields` and `--relations` definitions of the generators, and the Rust and SQL
//! types a field type stands for.

use super::{
    is_identifier_start, to_pascal_case, validate_rust_type_name, validate_sql_identifier,
};
use crate::config::canonical_driver;
use std::sync::Arc;
use tideorm::migration::{ColumnType, StringLen};

/// Strip a case-insensitive prefix, leaving the remainder untouched.
fn strip_prefix_ignore_ascii_case<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    match value.get(..prefix.len()) {
        Some(head) if head.eq_ignore_ascii_case(prefix) => Some(&value[prefix.len()..]),
        _ => None,
    }
}

/// Split a `--fields` list on `separator`, except inside a quoted SQL string or between
/// parentheses or brackets: `price:DECIMAL(10, 2)` stays one field and
/// `opens:time:default='09:00:00'` keeps its default whole.
pub fn split_definitions(definitions: &str, separator: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let (mut depth, mut quoted, mut start) = (0usize, false, 0);
    for (index, character) in definitions.char_indices() {
        match character {
            '\'' => quoted = !quoted,
            '(' | '[' if !quoted => depth += 1,
            ')' | ']' if !quoted => depth = depth.saturating_sub(1),
            _ if character == separator && !quoted && depth == 0 => {
                parts.push(&definitions[start..index]);
                start = index + character.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(&definitions[start..]);
    parts
}

/// Characters allowed in a field type beyond letters, digits and underscores.
///
/// They are what a custom type needs to stay expressible - `Vec<u8>`, `DECIMAL(10, 2)` -
/// while still excluding quotes, backslashes and `#`, which are the only characters that
/// could break out of the string literals the type is written into. (`:` is absent because
/// the field syntax already uses it as the separator.)
const EXTRA_FIELD_TYPE_CHARS: &[char] = &['<', '>', ',', '(', ')', '[', ']', ' '];

/// Validate a field type before it is emitted as a Rust type and as a SQL type.
///
/// Unrecognised types are deliberately passed through (see [`FieldDefinition`]), so this
/// only rejects spellings that could not be a type at all.
fn validate_field_type(field_type: &str) -> Result<(), String> {
    if field_type.is_empty() {
        return Err("Invalid field definition: the type must not be empty".to_string());
    }

    if !field_type.starts_with(is_identifier_start) {
        return Err(format!(
            "Invalid field type '{}': it must start with a letter or an underscore",
            field_type
        ));
    }

    if let Some(character) = field_type.chars().find(|character| {
        !character.is_ascii_alphanumeric()
            && *character != '_'
            && !EXTRA_FIELD_TYPE_CHARS.contains(character)
    }) {
        return Err(format!(
            "Invalid field type '{}': '{}' is not allowed",
            field_type, character
        ));
    }

    Ok(())
}

/// Validate the SQL fragment given to a `default=` modifier.
///
/// The value is copied verbatim into generated DDL and into a `#[tideorm(default = "..")]`
/// attribute, so it must not be able to terminate either string literal.
fn validate_default_value(value: &str) -> Result<(), String> {
    if let Some(character) = value
        .chars()
        .find(|character| matches!(character, '"' | '\\' | '\n' | '\r'))
    {
        return Err(format!(
            "Invalid default value '{}': '{}' cannot be emitted into generated code",
            value.escape_debug(),
            character.escape_debug()
        ));
    }

    Ok(())
}

/// Whether the database numbers a primary key of `field_type`: an `i32` or `i64` key is
/// auto-incremented, a key of any other type (a `uuid`, say) gets its value from the
/// application. The model and its migration both decide by this, so they agree.
pub fn auto_increments(field_type: &str) -> bool {
    matches!(rust_type_for_field_type(field_type).as_str(), "i32" | "i64")
}

/// Map a field type name to its Rust type, ignoring nullability.
///
/// Names that are not one of the aliases below are returned unchanged - see
/// [`FieldDefinition`] for why that passthrough exists.
pub fn rust_type_for_field_type(field_type: &str) -> String {
    let base_type = match field_type.to_lowercase().as_str() {
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
        "uuid" => "Uuid",
        "json" => "Json",
        "jsonb" => "Jsonb",
        "decimal" => "Decimal",
        "bytes" | "blob" | "binary" => "Vec<u8>",
        "int_array" | "integer_array" => "IntArray",
        "bigint_array" => "BigIntArray",
        "text_array" | "string_array" => "TextArray",
        "bool_array" | "boolean_array" => "BoolArray",
        "float_array" => "FloatArray",
        "json_array" => "JsonArray",
        _ => field_type,
    };

    base_type.to_string()
}

/// Parse field definition string
/// Format: name:type[:modifier1:modifier2...]
///
/// # Unknown types are passed through on purpose
///
/// Modifiers are a closed set and an unknown one is an error, but the type is not: a type
/// that matches none of the aliases in [`rust_type_for_field_type`] is emitted verbatim as
/// the Rust type and uppercased as the SQL type. That is the escape hatch for types the
/// CLI does not model - a `citext` or `vector` column or a `DECIMAL(10, 2)` with a custom
/// precision in a migration, a `u64` or `NaiveDateTime` in a model - and removing it would
/// be a breaking change for the projects relying on it. A model only compiles with a type
/// TideORM can store, so a domain enum is not one of them. The cost is that a typo such as
/// `count:integer8` is not caught here; it surfaces as `pub count: integer8` failing to
/// compile in the generated model, with TideORM's list of supported types.
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
        let parts = split_definitions(s, ':');

        if parts.len() < 2 {
            return Err(format!(
                "Invalid field definition '{}'. Expected format: name:type[:modifiers]",
                s
            ));
        }

        let name = parts[0].trim().to_string();
        let field_type = parts[1].trim().to_string();

        validate_sql_identifier("column", &name)?;
        validate_field_type(&field_type)?;

        let mut nullable = false;
        let mut unique = false;
        let mut indexed = false;
        let mut primary_key = false;
        let mut auto_increment = false;
        let mut default = None;

        // Parse modifiers
        for part in parts.iter().skip(2) {
            let part = part.trim();

            // The value of `default=` is SQL, so its casing must survive verbatim.
            if let Some(value) = strip_prefix_ignore_ascii_case(part, "default=") {
                validate_default_value(value)?;
                default = Some(value.to_string());
                continue;
            }

            match part.to_lowercase().as_str() {
                "nullable" | "null" => nullable = true,
                "unique" | "uniq" => unique = true,
                "indexed" | "index" | "idx" => indexed = true,
                "primary_key" | "primary" | "pk" => primary_key = true,
                "auto_increment" | "autoincrement" | "increment" => auto_increment = true,
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

    /// Convert field type string to its Rust type, ignoring nullability
    ///
    /// The names emitted here must all be reachable through `tideorm::prelude`, which
    /// generated models glob-import; scaffolded projects declare no other dependencies.
    pub fn base_rust_type(&self) -> String {
        rust_type_for_field_type(&self.field_type)
    }

    /// Convert field type string to a Rust type, wrapped in `Option<..>` when nullable.
    ///
    /// Nullability is passed in rather than read from `self.nullable` because the
    /// model generator also takes it from the `--nullable` list, which the parsed
    /// field definition does not know about.
    pub fn rust_type_for(&self, nullable: bool) -> String {
        if nullable {
            format!("Option<{}>", self.base_rust_type())
        } else {
            self.base_rust_type()
        }
    }

    /// The column type the field's type names, as TideORM's migrations and schema
    /// sync render it; `None` for a type the CLI passes through.
    ///
    /// Only the choice of type lives here. The SQL per backend is TideORM's, so a
    /// generated migration creates the column sync would: on MySQL a `Uuid` is
    /// `BINARY(16)`, timestamps keep their microseconds, and text and binary columns
    /// are the `LONG` kinds; on SQLite a decimal is `REAL`.
    fn column_type(&self, driver: &str) -> Option<ColumnType> {
        Some(match self.field_type.to_lowercase().as_str() {
            "string" | "varchar" => ColumnType::String(StringLen::None),
            // MySQL cannot index a TEXT column, unique or not, without a key length.
            "text"
                if canonical_driver(driver) == Some("mysql") && (self.unique || self.indexed) =>
            {
                ColumnType::String(StringLen::None)
            }
            "text" => ColumnType::Text,
            "i8" | "tinyint" | "i16" | "smallint" => ColumnType::SmallInteger,
            "i32" | "int" | "integer" => ColumnType::Integer,
            "i64" | "bigint" => ColumnType::BigInteger,
            "f32" | "float" => ColumnType::Float,
            "f64" | "double" => ColumnType::Double,
            "bool" | "boolean" => ColumnType::Boolean,
            "datetime" | "timestamp" => ColumnType::TimestampWithTimeZone,
            "date" => ColumnType::Date,
            "time" => ColumnType::Time,
            "uuid" => ColumnType::Uuid,
            "json" => ColumnType::Json,
            "jsonb" => ColumnType::JsonBinary,
            "decimal" => ColumnType::Decimal(Some((19, 4))),
            "bytes" | "blob" | "binary" => ColumnType::Blob,
            "int_array" | "integer_array" => ColumnType::Array(Arc::new(ColumnType::Integer)),
            "bigint_array" => ColumnType::Array(Arc::new(ColumnType::BigInteger)),
            "text_array" | "string_array" => ColumnType::Array(Arc::new(ColumnType::Text)),
            "bool_array" | "boolean_array" => ColumnType::Array(Arc::new(ColumnType::Boolean)),
            "float_array" => ColumnType::Array(Arc::new(ColumnType::Double)),
            "json_array" => ColumnType::Array(Arc::new(ColumnType::JsonBinary)),
            _ => return None,
        })
    }

    /// The column type a migration declares the field with.
    ///
    /// A type that matches none of the aliases is uppercased and used as-is, which is the
    /// SQL half of the passthrough documented on [`FieldDefinition`].
    pub fn migration_column_type(&self, driver: &str) -> ColumnType {
        self.column_type(driver)
            .unwrap_or_else(|| ColumnType::custom(self.field_type.to_uppercase()))
    }

    /// Reject a type the configured driver cannot store.
    ///
    /// Array fields become `Vec<T>` model fields, and only PostgreSQL has array
    /// columns: on any other driver the generated model would not compile.
    pub fn check_driver_support(&self, driver: &str) -> Result<(), String> {
        let is_array = matches!(
            self.field_type.to_lowercase().as_str(),
            "int_array"
                | "integer_array"
                | "bigint_array"
                | "text_array"
                | "string_array"
                | "bool_array"
                | "boolean_array"
                | "float_array"
                | "json_array"
        );

        let postgres = canonical_driver(driver) == Some("postgres");
        if is_array && !postgres {
            return Err(format!(
                "Field '{}' is a {}, which only PostgreSQL supports (this project uses {}). Store the list in a `json` field instead.",
                self.name, self.field_type, driver
            ));
        }

        if postgres && matches!(self.field_type.to_lowercase().as_str(), "i8" | "tinyint") {
            return Err(format!(
                "Field '{}' is an i8, which the PostgreSQL driver cannot bind. Use i16 (smallint) instead.",
                self.name
            ));
        }

        Ok(())
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
        validate_sql_identifier("relation field", &name)?;
        let relation_type = match parts[1].trim().to_lowercase().as_str() {
            "belongs_to" | "belongsto" => RelationType::BelongsTo,
            "has_one" | "hasone" => RelationType::HasOne,
            "has_many" | "hasmany" => RelationType::HasMany,
            other => return Err(format!("Unknown relation type: {}", other)),
        };
        let related_model = to_pascal_case(parts[2].trim());
        validate_rust_type_name("related model", &related_model)?;
        let foreign_key = parts
            .get(3)
            .map(|value| value.trim().to_string())
            .map(|foreign_key| {
                validate_sql_identifier("foreign key", &foreign_key)?;
                Ok::<_, String>(foreign_key)
            })
            .transpose()?;

        Ok(Self {
            name,
            relation_type,
            related_model,
            foreign_key,
        })
    }
}
