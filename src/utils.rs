//! Utility functions for TideORM CLI

use crate::config::canonical_driver;
use colored::Colorize;
use minijinja::{AutoEscape, Environment};
use serde::Serialize;
use std::path::Path;
use tideorm::config::DatabaseType;
use tideorm::migration::ColumnType;

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

/// The `[package] name` of the Cargo manifest at `manifest`.
pub fn package_name(manifest: &Path) -> Result<String, String> {
    let content = std::fs::read_to_string(manifest)
        .map_err(|error| format!("cannot read {}: {}", manifest.display(), error))?;
    let parsed: toml::Value = toml::from_str(&content)
        .map_err(|error| format!("cannot parse {}: {}", manifest.display(), error))?;

    parsed
        .get("package")
        .and_then(|package| package.get("name"))
        .and_then(toml::Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("{} has no [package] name", manifest.display()))
}

/// Report whether the process is attached to a terminal it can prompt on.
fn can_prompt() -> bool {
    use std::io::IsTerminal;

    std::env::var_os("TIDEORM_NONINTERACTIVE").is_none()
        && std::env::var_os("CI").is_none()
        && std::io::stdin().is_terminal()
        && std::io::stdout().is_terminal()
}

/// Make sure a generated file can be written without silently clobbering existing work.
///
/// Non-interactive runs never prompt: they fail with a clear message instead of hanging.
pub fn ensure_writable(path: &str) -> Result<(), String> {
    if !file_exists(path) {
        return Ok(());
    }

    if !can_prompt() {
        return Err(format!(
            "'{}' already exists. Remove it or pick a different name (refusing to overwrite).",
            path
        ));
    }

    if confirm(&format!("'{}' already exists. Overwrite it?", path)) {
        Ok(())
    } else {
        Err(format!("Aborted: '{}' already exists.", path))
    }
}

/// Whether `error`, from reading `path`, means there is nothing there yet:
/// not found, below a directory or none, rather than below a file.
pub(crate) fn is_missing_path(path: &Path, error: &std::io::Error) -> bool {
    if error.kind() != std::io::ErrorKind::NotFound {
        return false;
    }

    let mut ancestor = path.parent();
    while let Some(parent) = ancestor {
        match std::fs::metadata(parent) {
            Ok(metadata) => return metadata.is_dir(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                ancestor = parent.parent();
            }
            Err(_) => return false,
        }
    }

    true
}

pub fn read_source_or_empty(path: &str) -> Result<String, String> {
    match std::fs::read_to_string(path) {
        Ok(source) => Ok(source),
        Err(error) if is_missing_path(Path::new(path), &error) => Ok(String::new()),
        Err(error) => Err(format!("Failed to read {}: {}", path, error)),
    }
}

pub fn append_source_block(existing: &str, block: &str) -> String {
    let newline = if existing.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let mut updated = existing.to_string();
    if !updated.is_empty() && !updated.ends_with('\n') {
        updated.push_str(newline);
    }

    let block = block.replace("\r\n", "\n").replace('\n', newline);
    updated.push_str(&block);
    if !updated.ends_with('\n') {
        updated.push_str(newline);
    }
    updated
}

pub fn has_module_declaration(source: &str, module: &str) -> bool {
    let declaration = format!("pub mod {};", module);
    source.lines().any(|line| {
        line.split_once("//").map_or(line, |(code, _)| code).trim() == declaration.as_str()
    })
}

/// Rust keywords that cannot be emitted as bare identifiers in generated code.
const RUST_KEYWORDS: &[&str] = &[
    "as", "async", "await", "become", "box", "break", "const", "continue", "crate", "do", "dyn",
    "else", "enum", "extern", "false", "final", "fn", "for", "gen", "if", "impl", "in", "let",
    "loop", "macro", "match", "mod", "move", "mut", "override", "priv", "pub", "ref", "return",
    "self", "Self", "static", "struct", "super", "trait", "true", "try", "type", "typeof",
    "unsafe", "unsized", "use", "virtual", "where", "while", "yield",
];

/// Keywords Rust refuses to accept even in raw identifier form.
const NON_RAW_KEYWORDS: &[&str] = &["crate", "self", "Self", "super"];

/// Escape an identifier so it is valid when emitted into generated Rust source.
///
/// Keywords become raw identifiers (`type` -> `r#type`); the handful of keywords that
/// have no raw form get a trailing underscore instead.
pub fn escape_ident(name: &str) -> String {
    if NON_RAW_KEYWORDS.contains(&name) {
        format!("{}_", name)
    } else if RUST_KEYWORDS.contains(&name) {
        format!("r#{}", name)
    } else {
        name.to_string()
    }
}

/// Render generator output from a MiniJinja template.
pub fn render_template<T: Serialize>(
    template_name: &str,
    default_template: &str,
    template_path: Option<&str>,
    context: &T,
) -> Result<String, String> {
    let source = match template_path.map(str::trim).filter(|path| !path.is_empty()) {
        Some(path) => std::fs::read_to_string(path).map_err(|error| {
            format!(
                "Failed to read {} template '{}': {}",
                template_name, path, error
            )
        })?,
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
///
/// The resolution is milliseconds, not seconds: two migrations generated in the same
/// second would otherwise share a file name and collide on the `version` UNIQUE
/// constraint of the migrations table.
pub fn migration_timestamp() -> String {
    chrono::Utc::now().format("%Y%m%d%H%M%S%3f").to_string()
}

/// `message`, an error from connecting to `url`, with `url` masked wherever it shows.
pub fn masked_connection_error(message: impl std::fmt::Display, url: &str) -> String {
    message
        .to_string()
        .replace(url, &tideorm::internal::mask_url_credentials(url))
}

/// Derive the Rust module path that reaches a configured source directory.
///
/// `src/models` becomes `crate::models` and `src/domain/models` becomes
/// `crate::domain::models`. A leading `src` component is dropped because it is the crate
/// root itself, and every remaining component is escaped so a keyword directory name
/// still yields valid Rust.
pub fn crate_module_path(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    let mut segments: Vec<String> = normalized
        .split('/')
        .map(str::trim)
        .filter(|segment| !segment.is_empty() && *segment != ".")
        .map(escape_ident)
        .collect();

    if segments.first().is_some_and(|segment| segment == "src") {
        segments.remove(0);
    }

    if segments.is_empty() {
        "crate".to_string()
    } else {
        format!("crate::{}", segments.join("::"))
    }
}

/// Whether a character may start an identifier or a type name.
fn is_identifier_start(character: char) -> bool {
    character.is_ascii_alphabetic() || character == '_'
}

/// Validate a single SQL identifier before it is written into generated code.
///
/// Generated DDL is embedded in a Rust raw string literal, so an identifier carrying a
/// quote could close that literal early and inject arbitrary code into the user's own
/// source. Escaping cannot make that safe (`"#` ends the literal whichever way the quote
/// is doubled), so anything that is not a plain identifier is rejected outright.
pub fn validate_sql_identifier(kind: &str, name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err(format!("Invalid {} name: it must not be empty", kind));
    }

    if !name.starts_with(is_identifier_start) {
        return Err(format!(
            "Invalid {} name '{}': it must start with a letter or an underscore",
            kind, name
        ));
    }

    if let Some(character) = name
        .chars()
        .find(|character| !character.is_ascii_alphanumeric() && *character != '_')
    {
        return Err(format!(
            "Invalid {} name '{}': '{}' is not allowed (letters, digits and underscores only)",
            kind, name, character
        ));
    }

    Ok(())
}

/// Validate a generated Rust type name, including type-only reserved names.
pub fn validate_rust_type_name(kind: &str, name: &str) -> Result<(), String> {
    validate_sql_identifier(kind, name)?;
    if matches!(name, "Self" | "_") {
        return Err(format!(
            "Invalid {} name '{}': this is a reserved Rust type name",
            kind, name
        ));
    }
    Ok(())
}

/// The schema of a table named `schema.table`, if it has one, and its bare
/// name.
pub(crate) fn split_table(table: &str) -> (Option<&str>, &str) {
    match table.rsplit_once('.') {
        Some((schema, bare)) => (Some(schema), bare),
        None => (None, table),
    }
}

/// Validate a possibly schema qualified object name such as `public.users`.
pub fn validate_sql_object_name(kind: &str, name: &str) -> Result<(), String> {
    for segment in name.split('.') {
        validate_sql_identifier(kind, segment)?;
    }

    Ok(())
}

/// Quote a table or column name for the configured driver.
///
/// Schema qualified names are quoted segment by segment, and every segment is validated
/// with [`validate_sql_identifier`] first so no quote can ever reach the generated file.
pub fn quote_sql_identifier(driver: &str, kind: &str, name: &str) -> Result<String, String> {
    validate_sql_object_name(kind, name)?;

    let quoted: Vec<String> = name
        .split('.')
        .map(|segment| match canonical_driver(driver) {
            Some("mysql") => format!("`{}`", segment),
            _ => format!("\"{}\"", segment),
        })
        .collect();

    Ok(quoted.join("."))
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

/// Strip a case-insensitive prefix, leaving the remainder untouched.
fn strip_prefix_ignore_ascii_case<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    match value.get(..prefix.len()) {
        Some(head) if head.eq_ignore_ascii_case(prefix) => Some(&value[prefix.len()..]),
        _ => None,
    }
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

/// The database type a configured driver name stands for.
pub fn database_type(driver: &str) -> DatabaseType {
    match canonical_driver(driver) {
        Some("postgres") => DatabaseType::Postgres,
        Some("mysql") => DatabaseType::MySQL,
        _ => DatabaseType::SQLite,
    }
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
        let parts: Vec<&str> = s.split(':').collect();

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
            "string" | "varchar" => ColumnType::String,
            // MySQL cannot index a TEXT column, unique or not, without a key length.
            "text"
                if canonical_driver(driver) == Some("mysql") && (self.unique || self.indexed) =>
            {
                ColumnType::String
            }
            "text" => ColumnType::Text,
            "i8" | "tinyint" | "i16" | "smallint" => ColumnType::SmallInteger,
            "i32" | "int" | "integer" => ColumnType::Integer,
            "i64" | "bigint" => ColumnType::BigInteger,
            "f32" | "float" => ColumnType::Float,
            "f64" | "double" => ColumnType::Double,
            "bool" | "boolean" => ColumnType::Boolean,
            "datetime" | "timestamp" => ColumnType::TimestampTz,
            "date" => ColumnType::Date,
            "time" => ColumnType::Time,
            "uuid" => ColumnType::Uuid,
            "json" => ColumnType::Json,
            "jsonb" => ColumnType::Jsonb,
            "decimal" => ColumnType::Decimal {
                precision: 19,
                scale: 4,
            },
            "bytes" | "blob" | "binary" => ColumnType::Binary,
            "int_array" | "integer_array" => ColumnType::IntegerArray,
            "bigint_array" => ColumnType::BigIntegerArray,
            "text_array" | "string_array" => ColumnType::TextArray,
            "bool_array" | "boolean_array" => ColumnType::BooleanArray,
            "float_array" => ColumnType::DoubleArray,
            "json_array" => ColumnType::JsonArray,
            _ => return None,
        })
    }

    /// The column type a migration declares the field with.
    ///
    /// A type that matches none of the aliases is uppercased and used as-is, which is the
    /// SQL half of the passthrough documented on [`FieldDefinition`].
    pub fn migration_column_type(&self, driver: &str) -> ColumnType {
        self.column_type(driver)
            .unwrap_or_else(|| ColumnType::Custom(self.field_type.to_uppercase()))
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

/// Confirm an action with the user
///
/// A prompt that cannot be shown - or that fails half way through - answers "no",
/// which is only safe for callers that treat "no" as an abort. Destructive
/// commands must use [`confirm_destructive`] instead.
pub fn confirm(message: &str) -> bool {
    use dialoguer::Confirm;

    Confirm::new()
        .with_prompt(message)
        .default(false)
        .interact()
        .unwrap_or(false)
}

/// Confirm a destructive action with the user, failing closed when it cannot ask.
///
/// [`confirm`] cannot tell a real refusal apart from a prompt that was never
/// shown, and callers report a refusal as "Operation cancelled" plus a success
/// exit code. A non-interactive run would therefore look like it succeeded while
/// having done nothing, so a missing terminal is surfaced as an error and the
/// caller is pointed at `--force`.
pub fn confirm_destructive(message: &str) -> Result<bool, String> {
    confirm_destructive_with(message, can_prompt(), || confirm(message))
}

/// Core of [`confirm_destructive`], split out so the fail-closed behaviour is
/// testable without controlling the terminal.
fn confirm_destructive_with<F>(message: &str, can_prompt: bool, ask: F) -> Result<bool, String>
where
    F: FnOnce() -> bool,
{
    if !can_prompt {
        return Err(format!(
            "Cannot confirm \"{}\" without an interactive terminal; re-run with --force to proceed non-interactively.",
            message
        ));
    }

    Ok(ask())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appending_generated_source_separates_declarations_and_preserves_line_endings() {
        assert_eq!(
            append_source_block("pub mod old;", "pub mod new;"),
            "pub mod old;\npub mod new;\n"
        );
        assert_eq!(
            append_source_block("pub mod old;\r\n", "#[path = \"new.rs\"]\npub mod new;"),
            "pub mod old;\r\n#[path = \"new.rs\"]\r\npub mod new;\r\n"
        );
    }

    #[test]
    fn module_declaration_detection_ignores_comments_but_accepts_inline_comments() {
        assert!(!has_module_declaration("// pub mod user;", "user"));
        assert!(has_module_declaration("pub mod user; // models", "user"));
    }

    #[test]
    fn rust_type_names_reject_invalid_and_reserved_identifiers() {
        assert!(validate_rust_type_name("model", "User").is_ok());
        assert!(validate_rust_type_name("model", "123User").is_err());
        assert!(validate_rust_type_name("model", "Self").is_err());
        assert!(validate_rust_type_name("model", "_").is_err());
    }

    #[test]
    fn reading_unusable_source_does_not_treat_it_as_a_new_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mod.rs");
        std::fs::write(&path, [0xff]).unwrap();

        assert!(read_source_or_empty(path.to_str().unwrap()).is_err());
    }

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
    fn test_default_modifier_preserves_value_case() {
        let field = FieldDefinition::parse("status:string:default='Active'").unwrap();
        assert_eq!(field.default.as_deref(), Some("'Active'"));

        let field = FieldDefinition::parse("created_at:timestamp:DEFAULT=NOW()").unwrap();
        assert_eq!(field.default.as_deref(), Some("NOW()"));
    }

    #[test]
    fn test_nullable_generic_types_stay_balanced() {
        let field = FieldDefinition::parse("published_at:datetime:nullable").unwrap();
        assert_eq!(
            field.rust_type_for(true),
            "Option<chrono::DateTime<chrono::Utc>>"
        );
        assert_eq!(field.base_rust_type(), "chrono::DateTime<chrono::Utc>");
    }

    #[test]
    fn test_prelude_types_are_emitted_unqualified() {
        assert_eq!(
            FieldDefinition::parse("token:uuid")
                .unwrap()
                .rust_type_for(false),
            "Uuid"
        );
        assert_eq!(
            FieldDefinition::parse("price:decimal")
                .unwrap()
                .rust_type_for(false),
            "Decimal"
        );
    }

    #[test]
    fn test_unknown_field_types_pass_through_but_stay_emittable() {
        // Custom types are a documented escape hatch and must survive untouched...
        let field = FieldDefinition::parse("embedding:vector(1536)").unwrap();
        assert_eq!(field.rust_type_for(false), "vector(1536)");
        assert_eq!(
            field
                .migration_column_type("postgres")
                .to_sql(DatabaseType::Postgres),
            "VECTOR(1536)"
        );

        // ...but a "type" that could terminate a generated string literal is refused.
        let error = FieldDefinition::parse("evil:foo\"# BAD").unwrap_err();
        assert!(error.contains("Invalid field type"));

        let error = FieldDefinition::parse("evil:9lives").unwrap_err();
        assert!(error.contains("must start with a letter"));
    }

    #[test]
    fn test_array_fields_are_postgres_only() {
        let field = FieldDefinition::parse("tags:text_array:nullable").unwrap();
        assert!(field.check_driver_support("postgres").is_ok());
        assert_eq!(
            field
                .migration_column_type("postgres")
                .to_sql(DatabaseType::Postgres),
            "TEXT[]"
        );

        for driver in ["sqlite", "mysql"] {
            let error = field.check_driver_support(driver).unwrap_err();
            assert!(error.contains("only PostgreSQL"), "{}", error);
            assert!(error.contains("`json`"), "{}", error);
        }

        assert!(
            FieldDefinition::parse("meta:json")
                .unwrap()
                .check_driver_support("sqlite")
                .is_ok()
        );
    }

    #[test]
    fn test_column_names_and_defaults_cannot_break_generated_code() {
        let error = FieldDefinition::parse("na\"me:string").unwrap_err();
        assert!(error.contains("Invalid column name"));

        let error = FieldDefinition::parse("title:string:default=\"x\"").unwrap_err();
        assert!(error.contains("Invalid default value"));

        // Ordinary SQL defaults keep working.
        assert_eq!(
            FieldDefinition::parse("status:string:default='Active'")
                .unwrap()
                .default
                .as_deref(),
            Some("'Active'")
        );
    }

    #[test]
    fn test_migration_timestamp_has_sub_second_resolution() {
        let timestamp = migration_timestamp();
        assert_eq!(timestamp.len(), 17, "expected milliseconds: {}", timestamp);
        assert!(
            timestamp
                .chars()
                .all(|character| character.is_ascii_digit())
        );
    }

    #[test]
    fn test_crate_module_path_follows_configured_directories() {
        assert_eq!(crate_module_path("src/models"), "crate::models");
        assert_eq!(
            crate_module_path("./src/domain/models"),
            "crate::domain::models"
        );
        assert_eq!(crate_module_path("src\\models"), "crate::models");
        assert_eq!(crate_module_path("app/models"), "crate::app::models");
        assert_eq!(crate_module_path("src/type"), "crate::r#type");
        assert_eq!(crate_module_path("src"), "crate");
    }

    #[test]
    fn test_sql_identifiers_are_quoted_per_driver_and_validated() {
        assert_eq!(
            quote_sql_identifier("postgres", "table", "order").unwrap(),
            "\"order\""
        );
        assert_eq!(
            quote_sql_identifier("mysql", "table", "order").unwrap(),
            "`order`"
        );
        assert_eq!(
            quote_sql_identifier("postgres", "table", "public.users").unwrap(),
            "\"public\".\"users\""
        );

        let error = quote_sql_identifier("postgres", "table", "users\"#").unwrap_err();
        assert!(error.contains("Invalid table name"));
    }

    /// An unknown driver used to fall through to SQLite, so a `mariadb://` project
    /// generated SQLite DDL.
    #[test]
    fn test_mariadb_and_postgresql_map_to_their_database_types() {
        assert_eq!(database_type("mariadb"), DatabaseType::MySQL);
        assert_eq!(database_type("MySQL"), DatabaseType::MySQL);
        assert_eq!(database_type("postgresql"), DatabaseType::Postgres);
        assert_eq!(database_type("sqlite"), DatabaseType::SQLite);
    }

    #[test]
    fn test_only_integer_keys_auto_increment() {
        for field_type in ["i64", "bigint", "i32", "integer"] {
            assert!(auto_increments(field_type), "{field_type}");
        }
        for field_type in ["uuid", "string", "i16"] {
            assert!(!auto_increments(field_type), "{field_type}");
        }
    }

    #[test]
    fn test_escape_ident_handles_keywords() {
        assert_eq!(escape_ident("email"), "email");
        assert_eq!(escape_ident("type"), "r#type");
        assert_eq!(escape_ident("match"), "r#match");
        assert_eq!(escape_ident("self"), "self_");
        assert_eq!(escape_ident("crate"), "crate_");
    }

    #[test]
    fn test_destructive_confirmation_fails_closed_without_a_terminal() {
        // A prompt that cannot be shown must not be reported as a refusal: the
        // caller would print "Operation cancelled" and exit 0 having done nothing.
        let error = confirm_destructive_with("wipe all tables?", false, || true)
            .expect_err("a missing terminal must be an error");
        assert!(error.contains("wipe all tables?"));
        assert!(error.contains("--force"));

        assert_eq!(
            confirm_destructive_with("wipe all tables?", true, || false),
            Ok(false)
        );
        assert_eq!(
            confirm_destructive_with("wipe all tables?", true, || true),
            Ok(true)
        );
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

        let lowercase = RelationDefinition::parse("posts:has_many:post").unwrap();
        assert_eq!(lowercase.related_model, "Post");
        assert!(RelationDefinition::parse("posts\"#]:has_many:Post").is_err());
        assert!(RelationDefinition::parse("posts:belongs_to:Post:bad-name").is_err());
    }

    /// A password holding `?`, `#` or `/` is masked whole; TideORM 0.15.0 cut the URL
    /// at the `?` and printed the rest of the password.
    #[test]
    fn urls_mask_every_spelling_of_a_password() {
        use tideorm::internal::mask_url_credentials;

        for (url, shown) in [
            (
                "postgres://app:p?ss#w/rd@db.internal:5432/app?sslmode=require",
                "postgres://***@db.internal:5432/app?sslmode=require",
            ),
            ("postgres://app:secret@db/app", "postgres://***@db/app"),
            (
                "postgres://db/app?user=app&password=hunter2",
                "postgres://db/app?user=app&password=***",
            ),
            ("sqlite://app.db", "sqlite://app.db"),
        ] {
            assert_eq!(mask_url_credentials(url), shown);
        }

        let url = "mysql://root:p?ss@db.internal/app";
        let error = masked_connection_error(format!("could not reach {url}"), url);
        assert!(!error.contains("ss@"), "{error}");
        assert!(error.contains("mysql://***@db.internal/app"), "{error}");
    }
}
