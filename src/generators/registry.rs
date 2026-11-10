//! The `all()` registries a project's runners call, in a `mod.rs`: `tideorm
//! make` declares each module it generates there and adds it to the registry.

use crate::utils::{has_module_declaration, read_source_or_empty, write_atomically};
use std::path::Path;

/// A registry function: its first line, then the constructor its `.add(..)`
/// entries follow, one per line.
pub(crate) struct Registry {
    pub(crate) signature: &'static str,
    constructor: &'static str,
}

/// `seeders::all()`, which `tideorm db seed` runs.
pub(crate) const SEEDERS: Registry = Registry {
    signature: "pub fn all() -> Seeder {",
    constructor: "Seeder::new()",
};

/// `migrations::all()`, which `tideorm migrate` runs.
pub(crate) const MIGRATIONS: Registry = Registry {
    signature: "pub fn all() -> Migrator {",
    constructor: "Migrator::new()",
};

/// Declare `module` in `<dir>/mod.rs` with `declaration`, its lines after the
/// last `pub mod`, and add `entry` to `registry` there.
///
/// Returns `false`, leaving the registry alone, when the file has none in the
/// shape `tideorm init` scaffolds: editing a hand-written one could break it.
pub(crate) fn register(
    dir: &str,
    module: &str,
    declaration: &str,
    registry: &Registry,
    entry: &str,
) -> Result<bool, String> {
    let mod_path = format!("{}/mod.rs", dir);
    let source = read_source_or_empty(&mod_path)?;
    let mut lines: Vec<String> = source.lines().map(str::to_string).collect();

    if !has_module_declaration(&source, module) {
        let declaration: Vec<String> = declaration.lines().map(str::to_string).collect();
        match lines
            .iter()
            .rposition(|line| line.trim_start().starts_with("pub mod "))
        {
            Some(last) => {
                lines.splice(last + 1..last + 1, declaration);
            }
            None => {
                // After the leading comments and `#![..]` inner attributes, which must
                // come before any item, and the blank line after them, set off by another.
                let leading = lines
                    .iter()
                    .take_while(|line| {
                        let line = line.trim_start();
                        line.is_empty()
                            || (line.starts_with("//") && !line.starts_with("///"))
                            || line.starts_with("#![")
                    })
                    .count();
                let header = lines[..leading]
                    .iter()
                    .rposition(|line| !line.trim().is_empty())
                    .map_or(0, |last| last + 1);
                let at = header + usize::from(lines.get(header).is_some_and(|l| l.is_empty()));
                let end = at + declaration.len();
                lines.splice(at..at, declaration);
                if lines.get(end).is_some_and(|line| !line.is_empty()) {
                    lines.insert(end, String::new());
                }
            }
        }
    }

    let start = lines
        .iter()
        .position(|line| line.trim() == registry.signature)
        .filter(|&start| {
            lines
                .get(start + 1)
                .is_some_and(|l| l.trim() == registry.constructor)
        });
    if let Some(start) = start
        && !lines.iter().any(|line| line.trim() == entry)
    {
        let mut at = start + 2;
        while lines
            .get(at)
            .is_some_and(|l| l.trim_start().starts_with(".add("))
        {
            at += 1;
        }
        lines.insert(at, format!("        {}", entry));
    }

    let newline = if source.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    write_atomically(Path::new(&mod_path), &(lines.join(newline) + newline))?;

    Ok(start.is_some())
}

#[cfg(test)]
#[path = "../../tests/unit/generators_registry_tests.rs"]
mod tests;
