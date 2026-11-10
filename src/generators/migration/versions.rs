//! The versions and names of the migrations already in a migrations directory.

/// Refuse to generate a migration that could never be applied.
///
/// `version` is UNIQUE in the migrations ledger, so two files sharing one can be
/// written but only one of them can ever run, and a name picks a migration out for
/// `migrate up --migration`, which a second file of that name would make ambiguous.
/// Timestamps carry milliseconds, which makes a version clash unlikely, but reusing a
/// name is easy to do by accident and is worth reporting at generation time.
pub(super) fn ensure_no_conflicting_migration(
    directory: &str,
    timestamp: &str,
    migration_name: &str,
) -> Result<(), String> {
    for entry in migration_entries(directory)? {
        let path = entry.path();
        if path.extension().is_none_or(|extension| extension != "rs") {
            continue;
        }

        let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };

        if stem == "mod" {
            continue;
        }

        let existing_name = match stem.split_once('_') {
            Some((version, name))
                if !version.is_empty()
                    && version.chars().all(|character| character.is_ascii_digit()) =>
            {
                name
            }
            _ => stem,
        };

        if existing_name == migration_name {
            return Err(format!(
                "Migration '{}' already exists as '{}.rs'. Migration names are unique - pick a different name.",
                migration_name, stem
            ));
        }

        if stem.starts_with(&format!("{}_", timestamp)) {
            return Err(format!(
                "Migration version '{}' is already used by '{}.rs'. Re-run to get a fresh one.",
                timestamp, stem
            ));
        }
    }

    Ok(())
}

/// The version of a migration made without a timestamp: one past the highest
/// numbered migration in `directory`, as wide as the widest number there and
/// four digits at least, so that versions compare as text in the order they
/// were made. A number that needs another digit would sort before the rest,
/// so it is an error.
pub(super) fn next_migration_number(directory: &str) -> Result<String, String> {
    let mut highest = 0u128;
    let mut width = 4;
    for entry in migration_entries(directory)? {
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        let Some((number, _)) = name
            .strip_suffix(".rs")
            .and_then(|stem| stem.split_once('_'))
        else {
            continue;
        };
        if number.is_empty() || !number.bytes().all(|byte| byte.is_ascii_digit()) {
            continue;
        }
        width = width.max(number.len());
        let number = number
            .parse::<u128>()
            .map_err(|_| format!("Migration number of '{name}' is too large to follow"))?;
        highest = highest.max(number);
    }
    let next = highest
        .checked_add(1)
        .ok_or_else(|| format!("No migration number follows {highest}"))?;
    let version = format!("{next:0width$}");
    if version.len() > width {
        return Err(format!(
            "Migration number {version} has more digits than the {width} of the migrations before it, \
             so it would run before them. Set `timestamps = true` under [migration] in tideorm.toml, \
             or renumber the existing migrations with more digits."
        ));
    }
    Ok(version)
}

/// The entries of the migrations directory; none while it does not exist yet.
fn migration_entries(directory: &str) -> Result<Vec<std::fs::DirEntry>, String> {
    let read_error = |e: std::io::Error| format!("Failed to read {directory}: {e}");
    match std::fs::read_dir(directory) {
        Ok(entries) => entries.map(|entry| entry.map_err(read_error)).collect(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(read_error(e)),
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/generators_migration_versions_tests.rs"]
mod tests;
