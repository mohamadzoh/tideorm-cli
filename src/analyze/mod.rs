//! `tideorm analyze`: what a project's models and queries say about missing
//! indexes, slow patterns and common mistakes. Everything is read from the
//! project's source; when the database is reachable, its indexes and columns
//! are read too, and the models are checked against them.

mod database;
mod migrations;
mod models;
mod queries;

use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};

pub(crate) use database::{Catalog, Table};
pub(crate) use models::{Model, read_models};

/// How much a finding matters.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Severity {
    /// The code fails when it runs.
    Error,
    /// Slow or unsafe as written.
    Warning,
    /// Worth a look.
    Hint,
}

#[derive(Debug)]
pub(crate) struct Finding {
    pub(crate) severity: Severity,
    pub(crate) rule: &'static str,
    pub(crate) file: PathBuf,
    pub(crate) line: usize,
    pub(crate) message: String,
    pub(crate) help: String,
}

/// A parsed source file of the project.
pub(crate) struct SourceFile {
    pub(crate) path: PathBuf,
    pub(crate) syntax: syn::File,
}

/// What the checks know about the project.
pub(crate) struct Project {
    pub(crate) models: Vec<Model>,
    /// The key columns of each index of each table whose indexes are known,
    /// leading column first.
    indexes: HashMap<String, Vec<Vec<String>>>,
}

impl Project {
    /// The model `name` stands for in `file`: the one declared there, else
    /// the only one of that name its program can see. `None` when there are
    /// several, rather than a guess.
    fn model(&self, name: &str, file: &Path) -> Option<&Model> {
        let named: Vec<&Model> = self
            .visible(file)
            .filter(|model| model.name == name)
            .collect();
        let local: Vec<&Model> = named
            .iter()
            .copied()
            .filter(|model| model.file == file)
            .collect();
        match (local.as_slice(), named.as_slice()) {
            ([model], _) | ([], [model]) => Some(model),
            _ => None,
        }
    }

    /// A model `file` can see that reads `table`, one declared there first.
    fn model_of_table(&self, table: &str, file: &Path) -> Option<&Model> {
        let mut on_table = self.visible(file).filter(|model| model.table == table);
        let first = on_table.next()?;
        Some(
            std::iter::once(first)
                .chain(on_table)
                .find(|model| model.file == file)
                .unwrap_or(first),
        )
    }

    /// The models code in `file` can name: those of its own program and,
    /// from a `src/bin` program, the library's. Each `src/bin` program is a
    /// crate of its own, often declaring models of the same names.
    fn visible<'a>(&'a self, file: &Path) -> impl Iterator<Item = &'a Model> + use<'a> {
        let own = program(file).map(ToOwned::to_owned);
        self.models.iter().filter(move |model| {
            let other = program(&model.file);
            other.is_none() || other == own.as_deref()
        })
    }

    fn knows_indexes(&self, table: &str) -> bool {
        self.indexes.contains_key(table)
    }

    /// Whether some index of `table` leads with `column`; `None` when the
    /// table's indexes are not known.
    fn leads_index(&self, table: &str, column: &str) -> Option<bool> {
        let indexes = self.indexes.get(table)?;
        Some(
            indexes
                .iter()
                .any(|index| index.first().is_some_and(|first| first == column)),
        )
    }
}

/// Every finding for `files`, whose models are `models`. With `catalog`,
/// indexes are the database's and the models are checked against its
/// tables; without, indexes are the ones the migrations and models make.
pub(crate) fn analyze(
    files: &[SourceFile],
    models: Vec<Model>,
    catalog: Option<&Catalog>,
) -> Vec<Finding> {
    let indexes = match catalog {
        Some(catalog) => database::indexes(catalog),
        // The ones the migrations create and the ones the models declare;
        // models of one table (in separate programs, say) may each declare
        // some.
        None => {
            let mut indexes = migrations::indexes(files);
            for model in &models {
                indexes
                    .entry(model.table.clone())
                    .or_default()
                    .extend(model.indexes.iter().cloned());
            }
            indexes
        }
    };
    let project = Project { models, indexes };

    let mut findings = models::check(&project);
    if let Some(catalog) = catalog {
        findings.extend(database::check(&project, catalog));
    }
    for file in files {
        findings.extend(queries::check(&project, file));
    }
    findings.sort_by(|a, b| {
        (a.severity, &a.file, a.line, a.rule).cmp(&(b.severity, &b.file, b.line, b.rule))
    });
    findings
}

/// The `src/bin` program `path` is part of (`src/bin/<name>.rs` or
/// `src/bin/<name>/..`); `None` for the library and `src/main.rs`.
fn program(path: &Path) -> Option<&OsStr> {
    let components: Vec<Component> = path.components().collect();
    components.windows(3).rev().find_map(|window| match window {
        [
            Component::Normal(src),
            Component::Normal(bin),
            Component::Normal(name),
        ] if *src == "src" && *bin == "bin" => Path::new(*name).file_stem(),
        _ => None,
    })
}

/// The source line `span` starts on.
fn line(span: proc_macro2::Span) -> usize {
    span.start().line
}

/// `items` as prose: `a`, `a and b`, `a, b and c`, each in backticks.
fn listed(items: &[String]) -> String {
    let quoted: Vec<_> = items.iter().map(|item| format!("`{item}`")).collect();
    match quoted.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} and {last}", rest.join(", ")),
        _ => quoted.concat(),
    }
}

#[cfg(test)]
#[path = "../../tests/unit/analyze_tests.rs"]
mod tests;
