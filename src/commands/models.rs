//! Models command for TideORM CLI

use crate::config::TideConfig;
use crate::utils::{print_info, print_warning};
use colored::Colorize;
use std::fs;
use std::path::Path;

/// List all models in the project
pub async fn list(config_path: &str, verbose: bool) -> Result<(), String> {
    // Listing works without a project config, but a `tideorm.toml` that exists and cannot
    // be parsed is reported rather than being replaced by the default `src/models`.
    let config = TideConfig::load_or_default(config_path)?;

    if verbose {
        print_info(&format!("Looking for models in: {}", config.paths.models));
    }

    let models_path = Path::new(&config.paths.models);

    if !models_path.exists() {
        return Err(format!(
            "Models directory not found: {}",
            config.paths.models
        ));
    }

    let models = scan_models(&config.paths.models)?;

    println!("\n{}", "TideORM Models:".cyan().bold());
    println!("{}", "─".repeat(80));

    if models.is_empty() {
        println!("  No models found in {}", config.paths.models);
        println!("\n  Create a model with:");
        println!(
            "    {}",
            "tideorm make model User --fields=\"name:string,email:string:unique\"".yellow()
        );
    } else {
        println!(
            "  {:<20} {:<30} {:<15} Features",
            "Model", "Table", "Fields"
        );
        println!("{}", "─".repeat(80));

        for model in &models {
            let features: Vec<&str> = [
                if model.has_timestamps {
                    Some("timestamps")
                } else {
                    None
                },
                if model.has_soft_deletes {
                    Some("soft_delete")
                } else {
                    None
                },
                if model.has_tokenize {
                    Some("tokenize")
                } else {
                    None
                },
                if !model.relations.is_empty() {
                    Some("relations")
                } else {
                    None
                },
                if model.has_translatable {
                    Some("translatable")
                } else {
                    None
                },
            ]
            .into_iter()
            .flatten()
            .collect();

            println!(
                "  {:<20} {:<30} {:<15} {}",
                model.name.green(),
                model.table,
                model.fields.len().to_string(),
                features.join(", ")
            );
        }

        println!("{}", "─".repeat(80));
        println!("  Total: {} model(s)", models.len());
    }

    Ok(())
}

/// Model information
#[derive(Debug)]
struct ModelInfo {
    name: String,
    table: String,
    fields: Vec<String>,
    relations: Vec<String>,
    has_translatable: bool,
    has_timestamps: bool,
    has_soft_deletes: bool,
    has_tokenize: bool,
}

/// Scan the models directory and its subdirectories for model definitions
fn scan_models(models_path: &str) -> Result<Vec<ModelInfo>, String> {
    let mut models = Vec::new();
    scan_directory(Path::new(models_path), &mut models)?;
    models.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(models)
}

/// Collect the models of every `.rs` file under `path`; a file that is not valid Rust is
/// reported and skipped.
fn scan_directory(path: &Path, models: &mut Vec<ModelInfo>) -> Result<(), String> {
    let entries = fs::read_dir(path)
        .map_err(|e| format!("Failed to read models directory {}: {}", path.display(), e))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("Failed to read entry: {}", e))?;
        let file_path = entry.path();
        // `file_type` does not follow symlinks, so a link cycle cannot recurse forever.
        let file_type = entry
            .file_type()
            .map_err(|e| format!("Failed to read {}: {}", file_path.display(), e))?;

        if file_type.is_dir() {
            scan_directory(&file_path, models)?;
        } else if file_path.extension().is_some_and(|ext| ext == "rs") {
            let content = fs::read_to_string(&file_path)
                .map_err(|e| format!("Failed to read model file {}: {}", file_path.display(), e))?;
            match parse_model_file(&content) {
                Ok(found) => models.extend(found),
                Err(error) => print_warning(&format!(
                    "Skipping {}: it is not valid Rust ({})",
                    file_path.display(),
                    error
                )),
            }
        }
    }
    Ok(())
}

/// Parse a source file and return every model it defines, inline modules included
fn parse_model_file(content: &str) -> Result<Vec<ModelInfo>, syn::Error> {
    let syntax = syn::parse_file(content)?;
    let mut models = Vec::new();
    collect_models(&syntax.items, &mut models);
    Ok(models)
}

fn collect_models(items: &[syn::Item], models: &mut Vec<ModelInfo>) {
    for item in items {
        match item {
            syn::Item::Struct(item)
                if matches!(&item.vis, syn::Visibility::Public(_)) && is_model(&item.attrs) =>
            {
                models.extend(model_info(item));
            }
            syn::Item::Mod(syn::ItemMod {
                content: Some((_, items)),
                ..
            }) => collect_models(items, models),
            _ => {}
        }
    }
}

fn model_info(model: &syn::ItemStruct) -> Option<ModelInfo> {
    let struct_name = model.ident.to_string();
    let options = TideormOptions::of(&model.attrs);
    let table = options
        .table
        .clone()
        .unwrap_or_else(|| crate::utils::pluralize(&crate::utils::to_snake_case(&struct_name)));
    let named_fields = match &model.fields {
        syn::Fields::Named(fields) => &fields.named,
        _ => return None,
    };
    let fields: Vec<String> = named_fields
        .iter()
        .filter(|field| matches!(&field.vis, syn::Visibility::Public(_)))
        .filter_map(|field| field.ident.as_ref().map(ToString::to_string))
        .collect();

    let has_timestamps = fields.iter().any(|field| field == "created_at")
        && fields.iter().any(|field| field == "updated_at");
    let has_soft_deletes =
        options.has("soft_delete") || fields.iter().any(|field| field == "deleted_at");

    let relations: Vec<String> = named_fields
        .iter()
        .filter(|field| is_relation(&field.ty))
        .filter_map(|field| field.ident.as_ref().map(ToString::to_string))
        .collect();

    Some(ModelInfo {
        name: struct_name.to_string(),
        table,
        fields,
        relations,
        has_translatable: options.has("translatable"),
        has_timestamps,
        has_soft_deletes,
        has_tokenize: options.has("tokenize"),
    })
}

/// Whether a field's type is one of TideORM's relation wrappers, seen through `Option`,
/// `Box`, `Rc` and `Arc`: the wrapper, not an attribute, decides that a field is a relation.
fn is_relation(ty: &syn::Type) -> bool {
    const WRAPPERS: [&str; 9] = [
        "HasOne",
        "HasMany",
        "BelongsTo",
        "HasManyThrough",
        "MorphOne",
        "MorphMany",
        "MorphTo",
        "SelfRef",
        "SelfRefMany",
    ];
    let syn::Type::Path(path) = ty else {
        return false;
    };
    let Some(segment) = path.path.segments.last() else {
        return false;
    };
    if ["Option", "Box", "Rc", "Arc"]
        .iter()
        .any(|outer| segment.ident == outer)
    {
        let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments else {
            return false;
        };
        return arguments.args.iter().any(
            |argument| matches!(argument, syn::GenericArgument::Type(inner) if is_relation(inner)),
        );
    }
    WRAPPERS.iter().any(|wrapper| segment.ident == wrapper)
}

/// Whether `path` is `tideorm::model`, or `model` as the prelude imports it.
fn is_model_attribute(path: &syn::Path) -> bool {
    path.is_ident("model")
        || (path.segments.len() == 2
            && path.segments[0].ident == "tideorm"
            && path.segments[1].ident == "model")
}

fn is_model(attributes: &[syn::Attribute]) -> bool {
    attributes.iter().any(|attribute| {
        let path = attribute.path();
        is_model_attribute(path)
            || (path.is_ident("derive")
                && attribute
                    .parse_args_with(
                        syn::punctuated::Punctuated::<syn::Path, syn::Token![,]>::parse_terminated,
                    )
                    .is_ok_and(|derives| {
                        derives.iter().any(|path| {
                            path.segments
                                .last()
                                .is_some_and(|segment| segment.ident == "Model")
                        })
                    }))
    })
}

/// The options of an item's `#[tideorm(..)]` and `#[tideorm::model(..)]`
/// attributes.
#[derive(Default)]
struct TideormOptions {
    /// The name of every option, with a value or without.
    names: Vec<String>,
    table: Option<String>,
}

impl TideormOptions {
    fn of(attributes: &[syn::Attribute]) -> Self {
        let mut options = Self::default();
        for attribute in attributes {
            let path = attribute.path();
            if !path.is_ident("tideorm") && !is_model_attribute(path) {
                continue;
            }
            let _ = attribute.parse_nested_meta(|meta| {
                let name = meta
                    .path
                    .get_ident()
                    .map(ToString::to_string)
                    .unwrap_or_default();
                if name == "table" {
                    options.table = Some(meta.value()?.parse::<syn::LitStr>()?.value());
                } else if meta.input.peek(syn::Token![=]) {
                    let _: syn::Expr = meta.value()?.parse()?;
                } else if !meta.input.is_empty() && !meta.input.peek(syn::Token![,]) {
                    let _: proc_macro2::TokenTree = meta.input.parse()?;
                }
                options.names.push(name);
                Ok(())
            });
        }
        options
    }

    fn has(&self, name: &str) -> bool {
        self.names.iter().any(|option| option == name)
    }
}

#[cfg(test)]
#[path = "../../tests/unit/commands_models_tests.rs"]
mod tests;
