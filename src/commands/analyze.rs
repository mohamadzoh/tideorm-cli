//! `tideorm analyze`: report missing indexes, slow query patterns and common
//! mistakes in the project's code.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use colored::Colorize;

use crate::analyze::{self, Catalog, Finding, Model, Severity, SourceFile, Table};
use crate::config::TideConfig;
use crate::runtime_db;
use crate::utils::{print_info, print_warning};

/// Analyze the project of `config_path`: its `src/` and, unless `offline`,
/// its database. Fails when an error is found, so CI can run it.
pub async fn run(config_path: &str, offline: bool, verbose: bool) -> Result<(), String> {
    let root = Path::new(config_path)
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let source = root.join("src");
    if !source.is_dir() {
        return Err(format!("No source directory at {}", source.display()));
    }
    let files = read_sources(&source)?;
    let models = analyze::read_models(&files);
    if verbose {
        print_info(&format!(
            "Read {} files and {} models",
            files.len(),
            models.len()
        ));
    }

    let catalog = if offline || models.is_empty() {
        None
    } else if !Path::new(config_path).is_file() {
        print_warning(
            "No tideorm.toml, so the database is not read: indexes are the ones the migrations and models make",
        );
        None
    } else {
        let catalog = match TideConfig::load(config_path) {
            Ok(config) => read_catalog(&config, &models).await,
            Err(error) => Err(error),
        };
        match catalog {
            Ok(catalog) => Some(catalog),
            Err(error) => {
                print_warning(&format!(
                    "Could not read the database ({error}); indexes are the ones the migrations and models make"
                ));
                None
            }
        }
    };
    let indexes_from = if catalog.is_some() {
        "the database"
    } else {
        "the migrations and models"
    };

    let findings = analyze::analyze(&files, models, catalog.as_ref());
    for finding in &findings {
        print_finding(root, finding);
    }
    let count = |severity| {
        findings
            .iter()
            .filter(|finding| finding.severity == severity)
            .count()
    };
    let (errors, warnings, hints) = (
        count(Severity::Error),
        count(Severity::Warning),
        count(Severity::Hint),
    );
    if findings.is_empty() {
        println!(
            "{} No issues found in {} files (indexes from {indexes_from})",
            "✓".green(),
            files.len()
        );
    } else {
        println!(
            "\n{errors} error(s), {warnings} warning(s), {hints} hint(s) in {} files (indexes from {indexes_from})",
            files.len()
        );
    }
    if errors > 0 {
        return Err(format!("{errors} error(s) found"));
    }
    Ok(())
}

fn print_finding(root: &Path, finding: &Finding) {
    let label = match finding.severity {
        Severity::Error => "error".red().bold(),
        Severity::Warning => "warning".yellow().bold(),
        Severity::Hint => "hint".cyan().bold(),
    };
    let file = finding.file.strip_prefix(root).unwrap_or(&finding.file);
    println!("{label}[{}]: {}", finding.rule, finding.message.bold());
    println!("  {} {}:{}", "-->".blue(), file.display(), finding.line);
    println!("   {} {}\n", "= help:".blue(), finding.help);
}

/// Parse every `.rs` file under `dir`; one that is not valid Rust is
/// reported and skipped.
pub(crate) fn read_sources(dir: &Path) -> Result<Vec<SourceFile>, String> {
    let mut files = Vec::new();
    read_sources_into(dir, &mut files)?;
    Ok(files)
}

fn read_sources_into(dir: &Path, files: &mut Vec<SourceFile>) -> Result<(), String> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|error| format!("Failed to read {}: {error}", dir.display()))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<_, _>>()
        .map_err(|error| format!("Failed to read {}: {error}", dir.display()))?;
    entries.sort();
    for path in entries {
        // `symlink_metadata` does not follow links, so a link cycle cannot recurse forever.
        let is_dir = std::fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.is_dir());
        if is_dir {
            read_sources_into(&path, files)?;
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            let text = std::fs::read_to_string(&path)
                .map_err(|error| format!("Failed to read {}: {error}", path.display()))?;
            match syn::parse_file(&text) {
                Ok(syntax) => files.push(SourceFile { path, syntax }),
                Err(error) => print_warning(&format!(
                    "Skipping {}: it is not valid Rust ({error})",
                    path.display()
                )),
            }
        }
    }
    Ok(())
}

/// The columns and indexes of the tables the models and their pivots read,
/// those that exist. A table is looked up by the name the model gives it,
/// so a view or a `schema.table` is found as the model's queries find it.
async fn read_catalog(config: &TideConfig, models: &[Model]) -> Result<Catalog, String> {
    let db = runtime_db::connect(config).await?;
    let pivots = models
        .iter()
        .flat_map(|model| &model.fields)
        .filter_map(|field| field.relation.as_ref()?.pivot.clone());
    let mut tables: Vec<String> = models.iter().map(|model| model.table.clone()).collect();
    tables.extend(pivots);
    tables.sort();
    tables.dedup();
    let views: HashSet<String> = db
        .catalog()
        .views()
        .await
        .map_err(|error| error.to_string())?
        .into_iter()
        .collect();
    let mut catalog = Catalog::new();
    for table in tables {
        let columns = db
            .catalog()
            .columns(&table)
            .await
            .map_err(|error| error.to_string())?;
        if !columns.is_empty() {
            let indexes = runtime_db::table_indexes(&db, &table).await?;
            let view = views.contains(&table);
            catalog.insert(
                table,
                Table {
                    columns,
                    indexes,
                    view,
                },
            );
        }
    }
    Ok(catalog)
}
