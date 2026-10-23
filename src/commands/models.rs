//! Models command for TideORM CLI

use crate::config::TideConfig;
use crate::utils::print_info;
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

/// Scan models directory and extract model information
fn scan_models(models_path: &str) -> Result<Vec<ModelInfo>, String> {
    let path = Path::new(models_path);
    let mut models = Vec::new();

    for entry in
        fs::read_dir(path).map_err(|e| format!("Failed to read models directory: {}", e))?
    {
        let entry = entry.map_err(|e| format!("Failed to read entry: {}", e))?;
        let file_path = entry.path();

        if file_path.extension().is_some_and(|ext| ext == "rs") {
            let name = file_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();

            if name == "mod" {
                continue;
            }

            let content = fs::read_to_string(&file_path)
                .map_err(|e| format!("Failed to read model file: {}", e))?;

            if let Some(model_info) = parse_model_file(&content) {
                models.push(model_info);
            }
        }
    }

    models.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(models)
}

/// Parse a model file to extract information
fn parse_model_file(content: &str) -> Option<ModelInfo> {
    let syntax = syn::parse_file(content).ok()?;
    let model = syntax.items.iter().find_map(|item| {
        let syn::Item::Struct(item) = item else {
            return None;
        };
        if !matches!(&item.vis, syn::Visibility::Public(_)) || !is_model(&item.attrs) {
            return None;
        }
        Some(item)
    })?;

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
        .filter(|field| {
            let options = TideormOptions::of(&field.attrs);
            ["belongs_to", "has_one", "has_many"]
                .iter()
                .any(|relation| options.has(relation))
        })
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

/// Whether `path` is `tideorm::model`.
fn is_model_attribute(path: &syn::Path) -> bool {
    path.segments.len() == 2
        && path.segments[0].ident == "tideorm"
        && path.segments[1].ident == "model"
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
mod tests {
    use super::parse_model_file;

    #[test]
    fn test_parse_tideorm_model_attribute() {
        let content = r#"
#[tideorm::model(table = "posts")]
#[index("user_id")]
pub struct Post {
    pub id: i64,
    pub user_id: i64,
    pub title: String,
}
"#;

        let model = parse_model_file(content).expect("model should parse");
        assert_eq!(model.name, "Post");
        assert_eq!(model.table, "posts");
        assert_eq!(model.fields.len(), 3);
    }

    #[test]
    fn test_parse_only_reads_the_annotated_model_struct() {
        let content = r#"
#[tideorm::model(table = "posts")]
pub struct Post {
    pub id: i64,
    pub title: String,
    // pub commented_out: String,
}

#[tideorm::model(table = "helpers", soft_delete, translatable = "name", tokenize)]
pub struct Helper {
    pub created_at: String,
    pub updated_at: String,
    pub deleted_at: String,
    pub other: String,
}

impl Post {
    pub fn tokenize(&self) {}
}
"#;

        let model = parse_model_file(content).expect("model should parse");
        assert_eq!(model.name, "Post");
        assert_eq!(model.table, "posts");
        assert_eq!(model.fields, ["id", "title"]);
        assert!(!model.has_timestamps);
        assert!(!model.has_soft_deletes);
        assert!(!model.has_tokenize);
    }

    #[test]
    fn test_attachment_attribute_is_not_reported_as_a_relation() {
        let content = r#"
#[tideorm::model(table = "posts", has_many_files = "photos", translatable = "title", tokenize)]
pub struct Post {
    pub id: i64,
    #[tideorm(belongs_to = "User", foreign_key = "user_id")]
    pub user: BelongsTo<User>,
}
"#;

        let model = parse_model_file(content).expect("model should parse");
        assert_eq!(model.relations, ["user"]);
        assert!(model.has_translatable);
        assert!(model.has_tokenize);
    }

    #[test]
    fn test_parse_derived_model() {
        let content = r#"
#[derive(Clone, Model)]
pub struct User {
    pub id: i64,
}
"#;

        let model = parse_model_file(content).expect("derived model should parse");
        assert_eq!(model.name, "User");
        assert_eq!(model.table, "users");
        assert_eq!(model.fields, ["id"]);
    }
}
