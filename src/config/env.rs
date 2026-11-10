//! Environment variables in `tideorm.toml`: the env file, `${VAR}` placeholders and
//! `TIDEORM_ENV`.

use regex::Regex;
use std::collections::HashMap;
use std::path::Path;
use std::sync::LazyLock;

/// A `${VAR}` or `$VAR` placeholder; the first or the second group names the variable.
static PLACEHOLDER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\$\{([^}]+)\}|\$([A-Z_][A-Z0-9_]*)").unwrap());

/// Resolve the project environment, letting `TIDEORM_ENV` win.
///
/// Production guards are only useful when they can be switched on without editing a
/// checked-in file, so an explicit `TIDEORM_ENV` (process environment or env file)
/// overrides whatever `tideorm.toml` declares. Scaffolds emit
/// `environment = "${TIDEORM_ENV}"`, which resolves to the development default when the
/// variable is unset instead of leaking the placeholder through.
pub(super) fn resolve_environment(raw: &str, env_values: &HashMap<String, String>) -> String {
    if let Some(environment) = lookup_env("TIDEORM_ENV", env_values) {
        let environment = environment.trim();
        if !environment.is_empty() {
            return environment.to_string();
        }
    }

    let expanded = expand_env_vars_with_sources(raw, env_values);
    let expanded = expanded.trim();
    if expanded.is_empty() || expanded.starts_with('$') {
        return "development".to_string();
    }

    expanded.to_string()
}

/// The variables the placeholders left in `value` name, in order.
pub(super) fn unresolved_placeholders(value: &str) -> Vec<&str> {
    PLACEHOLDER
        .captures_iter(value)
        .filter_map(|captures| captures.get(1).or_else(|| captures.get(2)))
        .map(|name| name.as_str())
        .collect()
}

/// Whether `value` is one placeholder and nothing else.
pub(super) fn is_lone_placeholder(value: &str) -> bool {
    let value = value.trim();
    PLACEHOLDER
        .find(value)
        .is_some_and(|found| found.range() == (0..value.len()))
}

pub(super) fn expand_env_vars_with_sources(
    s: &str,
    env_values: &HashMap<String, String>,
) -> String {
    let mut result = s.to_string();

    for cap in PLACEHOLDER.captures_iter(s) {
        let var_name = cap.get(1).or_else(|| cap.get(2)).unwrap().as_str();
        if let Some(value) = lookup_env(var_name, env_values) {
            result = result.replace(cap.get(0).unwrap().as_str(), &value);
        }
    }

    result
}

pub(super) fn lookup_env(name: &str, env_values: &HashMap<String, String>) -> Option<String> {
    std::env::var(name)
        .ok()
        .or_else(|| env_values.get(name).cloned())
}

pub(super) fn load_env_file(
    config_dir: &Path,
    env_file_name: &str,
) -> Result<HashMap<String, String>, String> {
    env_file_values(&config_dir.join(env_file_name))
}

/// The values the env file at `env_path` sets, read as dotenvy reads them;
/// none when there is no such file.
pub(crate) fn env_file_values(env_path: &Path) -> Result<HashMap<String, String>, String> {
    let content = match std::fs::read_to_string(env_path) {
        Ok(content) => content,
        Err(error) if crate::utils::is_missing_path(env_path, &error) => {
            return Ok(HashMap::new());
        }
        Err(error) => {
            return Err(format!(
                "Failed to read env file {}: {}",
                env_path.display(),
                error
            ));
        }
    };

    let mut values = HashMap::new();
    for (number, line) in content.lines().enumerate() {
        let Some((key, value)) = env_assignment(line) else {
            continue;
        };
        // As dotenvy does, a value may use `${VAR}` from the process environment or from
        // the lines above it, so a `DATABASE_URL` can be assembled from parts.
        let value = parse_env_value(value, &values)
            .map_err(|error| format!("{} line {}: {}", env_path.display(), number + 1, error))?;
        values.insert(key.to_string(), value);
    }

    Ok(values)
}

/// The key and the raw value of a `KEY=value` or `export KEY = value` line; `None` for a
/// blank line, a comment or a line that assigns nothing.
pub(crate) fn env_assignment(line: &str) -> Option<(&str, &str)> {
    let line = line.trim();
    if line.starts_with('#') {
        return None;
    }
    let line = line.strip_prefix("export ").unwrap_or(line);
    let (key, value) = line.split_once('=')?;
    let key = key.trim();
    (!key.is_empty()).then_some((key, value))
}

/// Parse the value of a `KEY=value` line as dotenvy does, expanding its `${VAR}`
/// references from the process environment or `values`, the lines above it.
///
/// A quoted value runs to its closing quote; an unquoted one ends at a ` #` comment, so
/// `TIDEORM_ENV=production  # live` is `production`, while `p#ss` keeps its `#`. Only a
/// single-quoted value is literal; a double-quoted one reads dotenvy's escapes (`\n`,
/// `\\`, `\"`, `\'`, `\$`, `\ `) and refuses any other.
pub(super) fn parse_env_value(
    value: &str,
    values: &HashMap<String, String>,
) -> Result<String, String> {
    let value = value.trim();

    if let Some(inner) = value.strip_prefix('\'')
        && let Some(end) = inner.find('\'')
        && is_line_end(&inner[end + 1..])
    {
        return Ok(inner[..end].to_string());
    }
    if let Some(inner) = value.strip_prefix('"')
        && let Some((text, rest)) = double_quoted(inner, values)?
        && is_line_end(rest)
    {
        return Ok(text);
    }

    let end = value
        .char_indices()
        .find(|&(index, character)| {
            character.is_whitespace() && value[index..].trim_start().starts_with('#')
        })
        .map_or(value.len(), |(index, _)| index);
    Ok(expand_env_vars_with_sources(
        value[..end].trim_end(),
        values,
    ))
}

/// Whether `rest`, what follows a closing quote, ends the line: nothing or a comment.
fn is_line_end(rest: &str) -> bool {
    let rest = rest.trim_start();
    rest.is_empty() || rest.starts_with('#')
}

/// The text of a double-quoted value up to its closing quote, escapes read and `${VAR}`
/// references expanded, with what follows the quote; `None` without a closing quote.
fn double_quoted<'a>(
    inner: &'a str,
    values: &HashMap<String, String>,
) -> Result<Option<(String, &'a str)>, String> {
    let mut text = String::new();
    // What is read since the last `\$`, which is expanded before the literal `$`.
    let mut pending = String::new();
    let mut characters = inner.char_indices();
    while let Some((index, character)) = characters.next() {
        match character {
            '"' => {
                text.push_str(&expand_env_vars_with_sources(&pending, values));
                return Ok(Some((text, &inner[index + 1..])));
            }
            '\\' => match characters.next().map(|(_, escaped)| escaped) {
                Some('$') => {
                    text.push_str(&expand_env_vars_with_sources(&pending, values));
                    pending.clear();
                    text.push('$');
                }
                Some('n') => pending.push('\n'),
                Some(escaped @ ('\\' | '\'' | '"' | ' ')) => pending.push(escaped),
                Some(other) => {
                    return Err(format!("invalid escape `\\{other}` in a quoted value"));
                }
                None => return Ok(None),
            },
            _ => pending.push(character),
        }
    }
    Ok(None)
}
