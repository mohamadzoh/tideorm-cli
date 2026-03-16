//! Make commands for TideORM CLI (generators)

use crate::config::TideConfig;
use crate::generators::{
    factory::FactoryGenerator, migration::MigrationGenerator,
    model::ModelGenerator, seeder::SeederGenerator,
};
use crate::utils::{RelationDefinition, RelationType};
use crate::utils::{print_info, print_success};
use crate::MakeCommands;

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
                &output,
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
        } => make_migration(config_path, &name, create, table, fields, &output, verbose).await,

        MakeCommands::Seeder {
            name,
            model,
            count,
            output,
        } => make_seeder(config_path, &name, model, count, &output, verbose).await,

        MakeCommands::Factory {
            name,
            model,
            output,
        } => make_factory(config_path, &name, model, &output, verbose).await,
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
    soft_deletes: bool,
    timestamps: bool,
    tokenize: bool,
    output: &str,
    create_migration: bool,
    create_seeder: bool,
    create_factory: bool,
    verbose: bool,
) -> Result<(), String> {
    let config = TideConfig::load_or_default(config_path);

    if verbose {
        print_info(&format!("Generating model: {}", name));
    }

    // Clone fields for migration generation
    let fields_for_migration = prepare_model_migration_fields(
        fields.clone(),
        relations.as_deref(),
        attachments_single.as_deref(),
        attachments_multi.as_deref(),
        &config.model.primary_key_type,
    )?;

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
        .output_dir(output);

    // Generate model file
    let model_path = generator.generate()?;
    print_success(&format!("Created model: {}", model_path));

    // Generate migration if requested
    if create_migration {
        if verbose {
            print_info("Generating migration for model...");
        }

        let migration_gen = MigrationGenerator::new(&config);
        let migration_name = format!("create_{}_table", crate::utils::pluralize(&crate::utils::to_snake_case(name)));
        let migration_path = migration_gen.generate(
            &migration_name,
            Some(crate::utils::pluralize(&crate::utils::to_snake_case(name))),
            None,
            fields_for_migration,
            timestamps,
            soft_deletes,
        )?;
        print_success(&format!("Created migration: {}", migration_path));
    }

    // Generate seeder if requested
    if create_seeder {
        if verbose {
            print_info("Generating seeder for model...");
        }

        let seeder_gen = SeederGenerator::new(&config);
        let seeder_name = format!("{}Seeder", name);
        let seeder_path = seeder_gen.generate(&seeder_name, Some(name.to_string()), 10)?;
        print_success(&format!("Created seeder: {}", seeder_path));
    }

    // Generate factory if requested
    if create_factory {
        if verbose {
            print_info("Generating factory for model...");
        }

        let factory_gen = FactoryGenerator::new(&config);
        let factory_name = format!("{}Factory", name);
        let factory_path = factory_gen.generate(&factory_name, Some(name.to_string()))?;
        print_success(&format!("Created factory: {}", factory_path));
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
    _output: &str,
    verbose: bool,
) -> Result<(), String> {
    let config = TideConfig::load_or_default(config_path);

    if verbose {
        print_info(&format!("Generating migration: {}", name));
    }

    let generator = MigrationGenerator::new(&config);
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
    _output: &str,
    verbose: bool,
) -> Result<(), String> {
    let config = TideConfig::load_or_default(config_path);

    if verbose {
        print_info(&format!("Generating seeder: {}", name));
    }

    let generator = SeederGenerator::new(&config);
    let path = generator.generate(name, model, count)?;

    print_success(&format!("Created seeder: {}", path));

    Ok(())
}

/// Generate a new factory
async fn make_factory(
    config_path: &str,
    name: &str,
    model: Option<String>,
    _output: &str,
    verbose: bool,
) -> Result<(), String> {
    let config = TideConfig::load_or_default(config_path);

    if verbose {
        print_info(&format!("Generating factory: {}", name));
    }

    let generator = FactoryGenerator::new(&config);
    let path = generator.generate(name, model)?;

    print_success(&format!("Created factory: {}", path));

    Ok(())
}

fn prepare_model_migration_fields(
    fields: Option<String>,
    relations: Option<&str>,
    attachments_single: Option<&str>,
    attachments_multi: Option<&str>,
    primary_key_type: &str,
) -> Result<Option<String>, String> {
    let mut field_defs: Vec<String> = fields
        .as_deref()
        .map(|value| {
            value
                .split(',')
                .map(str::trim)
                .filter(|field| !field.is_empty())
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default();

    if let Some(relations_str) = relations {
        for relation in relations_str.split(',').map(str::trim).filter(|relation| !relation.is_empty()) {
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
mod tests {
    use super::prepare_model_migration_fields;

    #[test]
    fn test_prepare_model_migration_fields_uses_configured_primary_key_type() {
        let fields = prepare_model_migration_fields(
            Some("title:string".to_string()),
            Some("author:belongs_to:User"),
            None,
            None,
            "uuid",
        )
        .unwrap()
        .unwrap();

        assert!(fields.contains("title:string"));
        assert!(fields.contains("user_id:uuid:indexed"));
    }
}
