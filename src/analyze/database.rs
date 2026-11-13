//! The models checked against the database: tables and columns the code
//! expects, NULLs it cannot read, and indexes another one makes redundant.

use std::collections::{HashMap, HashSet};

use tideorm::schema::{ColumnInfo, IndexInfo};

use super::{Finding, Project, Severity};

/// A table as the database's catalog describes it.
pub(crate) struct Table {
    pub(crate) columns: Vec<ColumnInfo>,
    pub(crate) indexes: Vec<IndexInfo>,
    /// A view, whose columns the catalogs call nullable whatever they hold.
    pub(crate) view: bool,
}

/// The tables the models read that exist, by name.
pub(crate) type Catalog = HashMap<String, Table>;

/// The key columns of each index of each table, a primary key of one
/// column included even where the catalog lists no index for it (SQLite's
/// rowid alias).
pub(super) fn indexes(catalog: &Catalog) -> HashMap<String, Vec<Vec<String>>> {
    catalog
        .iter()
        .map(|(name, table)| {
            let mut indexes: Vec<Vec<String>> = table
                .indexes
                .iter()
                .map(|index| index.columns.clone())
                .collect();
            if let [key] = table
                .columns
                .iter()
                .filter(|column| column.primary_key)
                .collect::<Vec<_>>()
                .as_slice()
            {
                indexes.push(vec![key.name.clone()]);
            }
            (name.clone(), indexes)
        })
        .collect()
}

pub(super) fn check(project: &Project, catalog: &Catalog) -> Vec<Finding> {
    let mut findings = Vec::new();
    // A table several models read has its indexes reported once.
    let mut indexes_checked = HashSet::new();
    for model in &project.models {
        let Some(table) = catalog.get(&model.table) else {
            findings.push(Finding {
                severity: Severity::Warning,
                rule: "missing-table",
                file: model.file.clone(),
                line: model.line,
                message: format!(
                    "{} reads `{}`, which the database does not have",
                    model.name, model.table
                ),
                help: "run the migrations (`tideorm migrate`) against this database".into(),
            });
            continue;
        };
        for field in model.fields.iter().filter(|field| field.relation.is_none()) {
            let finding = |severity, rule, message, help| Finding {
                severity,
                rule,
                file: model.file.clone(),
                line: field.line,
                message,
                help,
            };
            match table
                .columns
                .iter()
                .find(|column| column.name == field.column)
            {
                None => findings.push(finding(
                    Severity::Error,
                    "missing-column",
                    format!(
                        "`{}.{}` reads the column `{}`, which `{}` does not have",
                        model.name, field.name, field.column, model.table
                    ),
                    "add it in a migration, or name the column with #[tideorm(column = \"..\")]"
                        .into(),
                )),
                Some(column)
                    if column.nullable && !table.view && !field.optional && !field.primary_key =>
                {
                    findings.push(finding(
                        Severity::Warning,
                        "nullable-mismatch",
                        format!(
                            "`{}.{}` may be NULL, but `{}.{}` is `{}`: loading a row \
                             that holds NULL fails",
                            model.table, field.column, model.name, field.name, field.ty
                        ),
                        "wrap the field's type in `Option<..>`, or make the column NOT NULL".into(),
                    ))
                }
                Some(_) => {}
            }
        }
        if indexes_checked.insert(&model.table) {
            findings.extend(redundant_indexes(model, table));
        }
    }
    findings
}

/// Plain indexes whose columns lead the primary key, which serves every
/// lookup they do; each write updates both. Only the primary key is sure to
/// be a whole B-tree: the catalog does not say which other index is partial
/// (`WHERE ..`), full-text or GIN, and such a one serves only some lookups.
fn redundant_indexes(model: &super::Model, table: &Table) -> Vec<Finding> {
    let mut keys: Vec<Vec<String>> = table
        .indexes
        .iter()
        .filter(|index| index.primary && index.whole_columns)
        .map(|index| index.columns.clone())
        .collect();
    // SQLite lists no index for a rowid key.
    if let [key] = table
        .columns
        .iter()
        .filter(|column| column.primary_key)
        .collect::<Vec<_>>()
        .as_slice()
    {
        keys.push(vec![key.name.clone()]);
    }
    let mut findings = Vec::new();
    for index in &table.indexes {
        if index.unique || index.primary || !index.whole_columns || index.columns.is_empty() {
            continue;
        }
        if let Some(key) = keys.iter().find(|key| key.starts_with(&index.columns)) {
            findings.push(Finding {
                severity: Severity::Hint,
                rule: "redundant-index",
                file: model.file.clone(),
                line: model.line,
                message: format!(
                    "index `{}` on `{}` ({}) is covered by the primary key ({}): \
                     every write updates both",
                    index.name,
                    model.table,
                    index.columns.join(", "),
                    key.join(", ")
                ),
                help: format!("drop `{}` in a migration", index.name),
            });
        }
    }
    findings
}
