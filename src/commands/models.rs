//! Models command for TideORM CLI

use crate::config::TideConfig;
use crate::utils::print_info;
use colored::Colorize;
use std::fs;
use std::path::Path;

/// List all models in the project
pub async fn list(config_path: &str, verbose: bool) -> Result<(), String> {
    let config = TideConfig::load_or_default(config_path);

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
            "tideorm make:model User --fields=\"name:string,email:string:unique\"".yellow()
        );
    } else {
        println!(
            "  {:<20} {:<30} {:<15} {}",
            "Model", "Table", "Fields", "Features"
        );
        println!("{}", "─".repeat(80));

        for model in &models {
            let features: Vec<&str> = [
                if model.has_timestamps { Some("timestamps") } else { None },
                if model.has_soft_deletes { Some("soft_delete") } else { None },
                if model.has_tokenize { Some("tokenize") } else { None },
                if !model.relations.is_empty() { Some("relations") } else { None },
                if !model.translatable.is_empty() { Some("translatable") } else { None },
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
    translatable: Vec<String>,
    has_timestamps: bool,
    has_soft_deletes: bool,
    has_tokenize: bool,
}

/// Scan models directory and extract model information
fn scan_models(models_path: &str) -> Result<Vec<ModelInfo>, String> {
    let path = Path::new(models_path);
    let mut models = Vec::new();

    for entry in fs::read_dir(path).map_err(|e| format!("Failed to read models directory: {}", e))? {
        let entry = entry.map_err(|e| format!("Failed to read entry: {}", e))?;
        let file_path = entry.path();

        if file_path.extension().map_or(false, |ext| ext == "rs") {
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
    // Find struct name with #[derive(Model)]
    let derive_model_pattern = regex::Regex::new(
        r#"#\[derive\([^)]*Model[^)]*\)\]\s*(?:#\[tide\([^\]]*\)\]\s*)*pub\s+struct\s+(\w+)"#
    ).ok()?;

    let struct_name = derive_model_pattern.captures(content)?.get(1)?.as_str();

    // Find table name
    let table_pattern = regex::Regex::new(r#"#\[tide\([^)]*table\s*=\s*"([^"]+)"[^)]*\)\]"#).ok()?;
    let table = table_pattern
        .captures(content)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
        .unwrap_or_else(|| crate::utils::pluralize(&crate::utils::to_snake_case(struct_name)));

    // Find fields
    let field_pattern = regex::Regex::new(r"pub\s+(\w+)\s*:\s*([^,\n}]+)").ok()?;
    let fields: Vec<String> = field_pattern
        .captures_iter(content)
        .map(|c| c.get(1).unwrap().as_str().to_string())
        .collect();

    // Check for features
    let has_timestamps = content.contains("created_at") && content.contains("updated_at");
    let has_soft_deletes = content.contains("soft_delete") || content.contains("deleted_at");
    let has_tokenize = content.contains("tokenize");

    // Find relations
    let relation_pattern = regex::Regex::new(r#"#\[tide\([^)]*(?:belongs_to|has_one|has_many)[^)]*\)\]"#).ok()?;
    let relations: Vec<String> = relation_pattern
        .find_iter(content)
        .map(|m| m.as_str().to_string())
        .collect();

    // Find translatable fields
    let translatable_pattern = regex::Regex::new(r#"#\[tide\([^)]*translatable[^)]*\)\]"#).ok()?;
    let translatable: Vec<String> = translatable_pattern
        .find_iter(content)
        .map(|m| m.as_str().to_string())
        .collect();

    Some(ModelInfo {
        name: struct_name.to_string(),
        table,
        fields,
        relations,
        translatable,
        has_timestamps,
        has_soft_deletes,
        has_tokenize,
    })
}
