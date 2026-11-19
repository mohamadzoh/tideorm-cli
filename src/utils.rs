//! Utility functions for TideORM CLI

use crate::config::canonical_driver;
use colored::Colorize;
use minijinja::{AutoEscape, Environment, value::Serde};
use serde::Serialize;
use std::path::Path;
use tideorm::config::DatabaseType;

mod fields;

pub use fields::{
    FieldDefinition, RelationDefinition, RelationType, auto_increments, rust_type_for_field_type,
    split_definitions,
};

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

/// `pub mod module;` for the module kept in `<file_stem>.rs`, with a
/// `#[path]` when the module's name does not lead to that file (`r#name`
/// still does; the underscore fallback of [`escape_ident`] does not).
pub fn module_declaration(module: &str, file_stem: &str) -> String {
    if module.trim_start_matches("r#") == file_stem {
        format!("pub mod {};", module)
    } else {
        format!("#[path = \"{}.rs\"]\npub mod {};", file_stem, module)
    }
}

/// Declare the module kept in `<dir>/<file_stem>.rs`, named after the file,
/// in `<dir>/mod.rs`, unless it is declared there already.
pub fn declare_module(dir: &str, file_stem: &str) -> Result<(), String> {
    let mod_path = format!("{}/mod.rs", dir);
    let module = escape_ident(file_stem);
    let existing = read_source_or_empty(&mod_path)?;
    if has_module_declaration(&existing, &module) {
        return Ok(());
    }
    let new_content = append_source_block(&existing, &module_declaration(&module, file_stem));
    write_atomically(Path::new(&mod_path), &new_content)
}

/// Replace the file at `path` with `content` through a temporary sibling renamed over it,
/// so a failed or interrupted write never leaves the file truncated.
pub fn write_atomically(path: &Path, content: &str) -> Result<(), String> {
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tideorm-tmp");
    std::fs::write(&temporary, content)
        .and_then(|()| std::fs::rename(&temporary, path))
        .map_err(|error| {
            let _ = std::fs::remove_file(&temporary);
            format!("Failed to write {}: {}", path.display(), error)
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
        .render(Serde(context))
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

/// The database type a configured driver name stands for.
pub fn database_type(driver: &str) -> DatabaseType {
    match canonical_driver(driver) {
        Some("postgres") => DatabaseType::Postgres,
        Some("mysql") => DatabaseType::MySQL,
        _ => DatabaseType::SQLite,
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
#[path = "../tests/unit/utils_tests.rs"]
mod tests;
