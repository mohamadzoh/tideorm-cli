//! Make commands for TideORM CLI (generators)

use crate::MakeCommands;
use crate::config::TideConfig;
use crate::generators::{
    factory::FactoryGenerator, migration::MigrationGenerator, model::ModelGenerator,
    seeder::SeederGenerator,
};
use crate::utils::{RelationDefinition, RelationType, split_definitions};
use crate::utils::{print_info, print_success};

/// Handle make subcommands
pub async fn handle(config_path: &str, cmd: MakeCommands, verbose: bool) -> Result<(), String> {
    match cmd {
        MakeCommands::Model {
            name,
            table,
            fields,
            relations,
            translatable,
            attachments_single,
            attachments_multi,
            indexed,
            unique,
            nullable,
            soft_deletes,
            timestamps,
            tokenize,
            output,
            migration,
            seeder,
            factory,
            all,
        } => {
            make_model(
                config_path,
                &name,
                table,
                fields,
                relations,
                translatable,
                attachments_single,
                attachments_multi,
                indexed,
                unique,
                nullable,
                soft_deletes,
                timestamps,
                tokenize,
                output.as_deref(),
                migration || all,
                seeder || all,
                factory || all,
                verbose,
            )
            .await
        }

        MakeCommands::Migration {
            name,
            create,
            table,
            fields,
            output,
        } => {
            make_migration(
                config_path,
                &name,
                create,
                table,
                fields,
                output.as_deref(),
                verbose,
            )
            .await
        }

        MakeCommands::Seeder {
            name,
            model,
            count,
            output,
        } => make_seeder(config_path, &name, model, count, output.as_deref(), verbose).await,

        MakeCommands::Factory {
            name,
            model,
            output,
        } => make_factory(config_path, &name, model, output.as_deref(), verbose).await,
    }
}

/// Generate a new model
#[allow(clippy::too_many_arguments)]
async fn make_model(
    config_path: &str,
    name: &str,
    table: Option<String>,
    fields: Option<String>,
    relations: Option<String>,
    translatable: Option<String>,
    attachments_single: Option<String>,
    attachments_multi: Option<String>,
    indexed: Option<String>,
    unique: Option<String>,
    nullable: Option<String>,
    soft_deletes: Option<bool>,
    timestamps: Option<bool>,
    tokenize: Option<bool>,
    output: Option<&str>,
    create_migration: bool,
    create_seeder: bool,
    create_factory: bool,
    verbose: bool,
) -> Result<(), String> {
    let mut config = TideConfig::load_or_default(config_path)?;

    if create_migration {
        crate::generators::migration::ensure_ddl_driver(&config.database.driver)?;
    }

    if verbose {
        print_info(&format!("Generating model: {}", name));
    }

    // Clone fields for migration generation
    let fields_for_migration = prepare_model_migration_fields(
        fields.clone(),
        relations.as_deref(),
        translatable.as_deref(),
        attachments_single.as_deref(),
        attachments_multi.as_deref(),
        &config.model.primary_key_type,
    )?
    .map(|fields| {
        with_listed_modifiers(
            &fields,
            &[
                ("nullable", nullable.as_deref()),
                ("unique", unique.as_deref()),
                ("indexed", indexed.as_deref()),
            ],
        )
    });

    // The migration has to describe the same table the model is bound to.
    let table_name = resolve_table_name(name, table.as_deref());

    // The clap flags are only overrides: an absent flag arrives as `None` and leaves the
    // `[model]`/`[paths]` config in place, so an explicit `--timestamps=true` is still
    // distinguishable from "not supplied" and wins over `timestamps = false`.
    let soft_deletes = soft_deletes.unwrap_or(config.model.soft_deletes);
    let timestamps = timestamps.unwrap_or(config.model.timestamps);
    let tokenize = tokenize.unwrap_or(config.model.tokenize);
    let output_dir = match output_override(output)? {
        Some(dir) => dir.to_string(),
        None => config.paths.models.clone(),
    };

    // A relocated model still has to be importable: the companion seeder and factory
    // derive their `use crate::..` path from `[paths] models`, so point it at wherever the
    // model actually landed.
    config.paths.models = output_dir.clone();

    // Create model generator
    let generator = ModelGenerator::new(&config)
        .name(name)
        .table(table)
        .fields(fields)
        .relations(relations)
        .translatable(translatable)
        .attachments_single(attachments_single)
        .attachments_multi(attachments_multi)
        .indexed(indexed)
        .unique(unique)
        .nullable(nullable)
        .soft_deletes(soft_deletes)
        .timestamps(timestamps)
        .tokenize(tokenize)
        .output_dir(&output_dir);

    let migration_gen = MigrationGenerator::new(&config);
    let seeder_gen = SeederGenerator::new(&config);
    let factory_gen = FactoryGenerator::new(&config);
    let seed_fields = generator.seed_fields();

    // Check and render every requested file before writing any, so a failure leaves
    // nothing half generated.
    let mut files = vec![("model", generator.render()?)];

    if create_migration {
        if verbose {
            print_info("Generating migration for model...");
        }
        let migration_name = format!("create_{}_table", table_name);
        files.push((
            "migration",
            migration_gen.render(
                &migration_name,
                Some(table_name),
                None,
                fields_for_migration,
                timestamps,
                soft_deletes,
            )?,
        ));
    }

    if create_seeder {
        if verbose {
            print_info("Generating seeder for model...");
        }
        let seeder_name = format!("{}Seeder", name);
        files.push((
            "seeder",
            seeder_gen.render(&seeder_name, Some(name.to_string()), 10, &seed_fields)?,
        ));
    }

    if create_factory {
        if verbose {
            print_info("Generating factory for model...");
        }
        let factory_name = format!("{}Factory", name);
        files.push((
            "factory",
            factory_gen.render(&factory_name, Some(name.to_string()))?,
        ));
    }

    for (kind, file) in files {
        let path = file.write()?;
        print_success(&format!("Created {}: {}", kind, path));
    }

    Ok(())
}

/// Generate a new migration
async fn make_migration(
    config_path: &str,
    name: &str,
    create: Option<String>,
    table: Option<String>,
    fields: Option<String>,
    output: Option<&str>,
    verbose: bool,
) -> Result<(), String> {
    let config = TideConfig::load_or_default(config_path)?;

    if verbose {
        print_info(&format!("Generating migration: {}", name));
    }

    let output_dir = output_override(output)?;
    let generator = MigrationGenerator::new(&config).output_dir(output_dir);
    let path = generator.generate(name, create, table, fields, false, false)?;

    print_success(&format!("Created migration: {}", path));

    Ok(())
}

/// Generate a new seeder
async fn make_seeder(
    config_path: &str,
    name: &str,
    model: Option<String>,
    count: u32,
    output: Option<&str>,
    verbose: bool,
) -> Result<(), String> {
    let config = TideConfig::load_or_default(config_path)?;

    if verbose {
        print_info(&format!("Generating seeder: {}", name));
    }

    let output_dir = output_override(output)?;
    let generator = SeederGenerator::new(&config).output_dir(output_dir);
    let path = generator.generate(name, model, count)?;

    print_success(&format!("Created seeder: {}", path));

    Ok(())
}

/// Generate a new factory
async fn make_factory(
    config_path: &str,
    name: &str,
    model: Option<String>,
    output: Option<&str>,
    verbose: bool,
) -> Result<(), String> {
    let config = TideConfig::load_or_default(config_path)?;

    if verbose {
        print_info(&format!("Generating factory: {}", name));
    }

    let output_dir = output_override(output)?;
    let generator = FactoryGenerator::new(&config).output_dir(output_dir);
    let path = generator.generate(name, model)?;

    print_success(&format!("Created factory: {}", path));

    Ok(())
}

/// The directory an explicit `--output` asks for, or `None` when the configured path wins.
fn output_override(output: Option<&str>) -> Result<Option<&str>, String> {
    match output {
        Some("") => Err("--output requires a non-empty directory".to_string()),
        Some(directory) => Ok(Some(directory)),
        None => Ok(None),
    }
}

/// Resolve the table a generated model binds to, honouring an explicit `--table` override.
fn resolve_table_name(model_name: &str, table: Option<&str>) -> String {
    match table {
        Some(table) => table.to_string(),
        None => crate::utils::pluralize(&crate::utils::to_snake_case(model_name)),
    }
}

/// `fields` with each field a `--nullable`, `--unique` or `--indexed` list
/// names carrying that modifier, so the migration creates the column the
/// model is generated for: a nullable field left `NOT NULL` fails the first
/// insert that leaves it empty.
fn with_listed_modifiers(fields: &str, lists: &[(&str, Option<&str>)]) -> String {
    split_definitions(fields, ',')
        .into_iter()
        .map(|field| {
            let name = field.split(':').next().unwrap_or_default().trim();
            let mut field = field.to_string();
            for (modifier, list) in lists {
                let listed = list.is_some_and(|names| names.split(',').any(|n| n.trim() == name));
                let present = split_definitions(&field, ':')
                    .iter()
                    .skip(2)
                    .any(|m| m.trim() == *modifier);
                if listed && !present {
                    field.push(':');
                    field.push_str(modifier);
                }
            }
            field
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn prepare_model_migration_fields(
    fields: Option<String>,
    relations: Option<&str>,
    translatable: Option<&str>,
    attachments_single: Option<&str>,
    attachments_multi: Option<&str>,
    primary_key_type: &str,
) -> Result<Option<String>, String> {
    let mut field_defs: Vec<String> = fields
        .as_deref()
        .map(|value| {
            split_definitions(value, ',')
                .into_iter()
                .map(str::trim)
                .filter(|field| !field.is_empty())
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default();

    if let Some(relations_str) = relations {
        for relation in relations_str
            .split(',')
            .map(str::trim)
            .filter(|relation| !relation.is_empty())
        {
            let relation = RelationDefinition::parse(relation)?;
            if relation.relation_type != RelationType::BelongsTo {
                continue;
            }

            let foreign_key = relation.foreign_key.unwrap_or_else(|| {
                format!(
                    "{}_id",
                    crate::utils::to_snake_case(&relation.related_model)
                )
            });

            let already_present = field_defs.iter().any(|field| {
                field
                    .split(':')
                    .next()
                    .is_some_and(|name| name.trim() == foreign_key)
            });

            if !already_present {
                field_defs.push(format!("{}:{}:indexed", foreign_key, primary_key_type));
            }
        }
    }

    if translatable.is_some() {
        let has_translations_column = field_defs.iter().any(|field| {
            field
                .split(':')
                .next()
                .is_some_and(|name| name.trim() == "translations")
        });

        if !has_translations_column {
            field_defs.push("translations:jsonb:nullable".to_string());
        }
    }

    if attachments_single.is_some() || attachments_multi.is_some() {
        let has_files_column = field_defs.iter().any(|field| {
            field
                .split(':')
                .next()
                .is_some_and(|name| name.trim() == "files")
        });

        if !has_files_column {
            field_defs.push("files:jsonb:nullable".to_string());
        }
    }

    if field_defs.is_empty() {
        Ok(None)
    } else {
        Ok(Some(field_defs.join(",")))
    }
}

#[cfg(test)]
#[path = "../../tests/unit/commands_make_tests.rs"]
mod tests;
