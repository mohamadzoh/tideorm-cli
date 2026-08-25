//! Make commands for TideORM CLI (generators)

use crate::MakeCommands;
use crate::config::TideConfig;
use crate::generators::{
    factory::FactoryGenerator, migration::MigrationGenerator, model::ModelGenerator,
    seeder::SeederGenerator,
};
use crate::utils::{RelationDefinition, RelationType};
use crate::utils::{print_info, print_success};

/// Clap's default for `make model --output`; anything else is an explicit override.
const DEFAULT_MODEL_OUTPUT: &str = "src/models";

/// Clap's default for `make migration --output`.
const DEFAULT_MIGRATION_OUTPUT: &str = "src/migrations";

/// Clap's default for `make seeder --output`.
const DEFAULT_SEEDER_OUTPUT: &str = "src/seeders";

/// Clap's default for `make factory --output`.
const DEFAULT_FACTORY_OUTPUT: &str = "src/factories";

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
    soft_deletes: Option<bool>,
    timestamps: Option<bool>,
    tokenize: Option<bool>,
    output: &str,
    create_migration: bool,
    create_seeder: bool,
    create_factory: bool,
    verbose: bool,
) -> Result<(), String> {
    let mut config = TideConfig::load_or_default(config_path)?;

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
    )?;

    // The migration has to describe the same table the model is bound to.
    let table_name = resolve_table_name(name, table.as_deref());

    // The clap flags are only overrides: an absent flag arrives as `None` and leaves the
    // `[model]`/`[paths]` config in place, so an explicit `--timestamps=true` is still
    // distinguishable from "not supplied" and wins over `timestamps = false`.
    let soft_deletes = soft_deletes.unwrap_or(config.model.soft_deletes);
    let timestamps = timestamps.unwrap_or(config.model.timestamps);
    let tokenize = tokenize.unwrap_or(config.model.tokenize);
    let output_dir = match output_override(output, DEFAULT_MODEL_OUTPUT) {
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

    // Generate model file
    let model_path = generator.generate()?;
    print_success(&format!("Created model: {}", model_path));

    // Generate migration if requested
    if create_migration {
        if verbose {
            print_info("Generating migration for model...");
        }

        let migration_gen = MigrationGenerator::new(&config);
        let migration_name = format!("create_{}_table", table_name);
        let migration_path = migration_gen.generate(
            &migration_name,
            Some(table_name),
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
    output: &str,
    verbose: bool,
) -> Result<(), String> {
    let config = TideConfig::load_or_default(config_path)?;

    if verbose {
        print_info(&format!("Generating migration: {}", name));
    }

    let output_dir = output_override(output, DEFAULT_MIGRATION_OUTPUT);
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
    output: &str,
    verbose: bool,
) -> Result<(), String> {
    let config = TideConfig::load_or_default(config_path)?;

    if verbose {
        print_info(&format!("Generating seeder: {}", name));
    }

    let output_dir = output_override(output, DEFAULT_SEEDER_OUTPUT);
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
    output: &str,
    verbose: bool,
) -> Result<(), String> {
    let config = TideConfig::load_or_default(config_path)?;

    if verbose {
        print_info(&format!("Generating factory: {}", name));
    }

    let output_dir = output_override(output, DEFAULT_FACTORY_OUTPUT);
    let generator = FactoryGenerator::new(&config).output_dir(output_dir);
    let path = generator.generate(name, model)?;

    print_success(&format!("Created factory: {}", path));

    Ok(())
}

/// The directory an explicit `--output` asks for, or `None` when the flag was left at its
/// clap default and the configured `[paths]` entry should keep winning.
///
/// `--output` carries a default value rather than being optional, so "not supplied" and
/// "supplied with the same value" are indistinguishable; treating both as "use the config"
/// is what keeps a project whose `[paths]` point elsewhere working without the flag.
fn output_override<'a>(output: &'a str, clap_default: &str) -> Option<&'a str> {
    let output = output.trim();
    if output.is_empty() || output == clap_default {
        None
    } else {
        Some(output)
    }
}

/// Resolve the table a generated model binds to, honouring an explicit `--table` override.
fn resolve_table_name(model_name: &str, table: Option<&str>) -> String {
    match table {
        Some(table) => table.to_string(),
        None => crate::utils::pluralize(&crate::utils::to_snake_case(model_name)),
    }
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
            value
                .split(',')
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
mod tests {
    use super::{
        DEFAULT_FACTORY_OUTPUT, DEFAULT_MIGRATION_OUTPUT, DEFAULT_MODEL_OUTPUT,
        DEFAULT_SEEDER_OUTPUT, make_factory, make_migration, make_model, make_seeder,
        output_override, prepare_model_migration_fields, resolve_table_name,
    };
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    #[test]
    fn test_resolve_table_name_honours_explicit_override() {
        assert_eq!(resolve_table_name("BlogPost", None), "blog_posts");
        assert_eq!(resolve_table_name("User", Some("app_users")), "app_users");
    }

    #[test]
    fn test_prepare_model_migration_fields_uses_configured_primary_key_type() {
        let fields = prepare_model_migration_fields(
            Some("title:string".to_string()),
            Some("author:belongs_to:User"),
            None,
            None,
            None,
            "uuid",
        )
        .unwrap()
        .unwrap();

        assert!(fields.contains("title:string"));
        assert!(fields.contains("user_id:uuid:indexed"));
    }

    #[test]
    fn test_prepare_model_migration_fields_adds_translations_column() {
        let fields = prepare_model_migration_fields(
            Some("title:string".to_string()),
            None,
            Some("title"),
            None,
            None,
            "i64",
        )
        .unwrap()
        .unwrap();

        assert!(fields.contains("title:string"));
        assert!(fields.contains("translations:jsonb:nullable"));
    }

    #[test]
    fn output_override_only_reports_an_explicit_directory() {
        assert_eq!(
            output_override(DEFAULT_SEEDER_OUTPUT, DEFAULT_SEEDER_OUTPUT),
            None
        );
        assert_eq!(output_override("   ", DEFAULT_SEEDER_OUTPUT), None);
        assert_eq!(
            output_override("  db/seeders  ", DEFAULT_SEEDER_OUTPUT),
            Some("db/seeders")
        );
        assert_eq!(
            output_override(DEFAULT_FACTORY_OUTPUT, DEFAULT_FACTORY_OUTPUT),
            None
        );
        assert_eq!(
            output_override(DEFAULT_MODEL_OUTPUT, DEFAULT_MIGRATION_OUTPUT),
            Some(DEFAULT_MODEL_OUTPUT)
        );
    }

    #[tokio::test]
    async fn make_migration_writes_to_the_output_directory() {
        let output = temp_output();

        make_migration(
            MISSING_CONFIG,
            "create_users_table",
            Some("users".to_string()),
            None,
            None,
            output.path(),
            false,
        )
        .await
        .expect("migration should be generated");

        assert!(output.contains_file_matching("_create_users_table.rs"));
        assert!(Path::new(&format!("{}/mod.rs", output.path())).exists());
    }

    #[tokio::test]
    async fn make_seeder_writes_to_the_output_directory() {
        let output = temp_output();

        make_seeder(
            MISSING_CONFIG,
            "UserSeeder",
            Some("User".to_string()),
            5,
            output.path(),
            false,
        )
        .await
        .expect("seeder should be generated");

        let content = fs::read_to_string(format!("{}/user_seeder.rs", output.path()))
            .expect("seeder should be written to --output");
        // Moving the file must not move the model: the import still follows
        // `[paths] models`.
        assert!(
            content.contains("use crate::models::user::User;"),
            "{}",
            content
        );
    }

    #[tokio::test]
    async fn make_factory_writes_to_the_output_directory() {
        let output = temp_output();

        make_factory(
            MISSING_CONFIG,
            "UserFactory",
            Some("User".to_string()),
            output.path(),
            false,
        )
        .await
        .expect("factory should be generated");

        let content = fs::read_to_string(format!("{}/user_factory.rs", output.path()))
            .expect("factory should be written to --output");
        assert!(
            content.contains("use crate::models::user::User;"),
            "{}",
            content
        );
    }

    #[tokio::test]
    async fn make_model_output_also_moves_the_companion_seeder_import() {
        let project = TestProject::new();
        let model_output = project.path("src/domain/models");

        make_model(
            project.config_path(),
            "User",
            None,
            Some("name:string".to_string()),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some(false),
            Some(false),
            Some(false),
            &model_output,
            false,
            true,
            false,
            false,
        )
        .await
        .expect("model and seeder should be generated");

        assert!(Path::new(&format!("{}/user.rs", model_output)).exists());

        let seeder = fs::read_to_string(format!("{}/user_seeder.rs", project.path("src/seeders")))
            .expect("seeder should be written to the configured seeders directory");
        // The configured paths are absolute here, so only the tail of the module path is
        // meaningful - what matters is that it follows the relocated model.
        assert!(seeder.contains("domain::models::user::User"), "{}", seeder);
    }

    /// A config path that does not exist, so the built-in defaults apply.
    const MISSING_CONFIG: &str = "tideorm.toml.does-not-exist";

    /// A throwaway directory to hand to `--output`.
    struct Output {
        _dir: TempDir,
        path: String,
    }

    impl Output {
        fn path(&self) -> &str {
            &self.path
        }

        fn contains_file_matching(&self, suffix: &str) -> bool {
            fs::read_dir(&self.path)
                .expect("output directory should exist")
                .flatten()
                .any(|entry| entry.file_name().to_string_lossy().ends_with(suffix))
        }
    }

    fn temp_output() -> Output {
        let dir = TempDir::new().expect("temp dir should be created");
        let path = slash_path(dir.path().join("generated"));
        Output { _dir: dir, path }
    }

    /// A project whose `tideorm.toml` keeps every generated file inside a temp directory.
    struct TestProject {
        _dir: TempDir,
        root: std::path::PathBuf,
        config_path: String,
    }

    impl TestProject {
        fn new() -> Self {
            let dir = TempDir::new().expect("temp dir should be created");
            let root = dir.path().to_path_buf();
            let config_path = root.join("tideorm.toml");

            let contents = format!(
                "[project]\nname = \"demo\"\nenvironment = \"development\"\n\n[database]\ndriver = \"sqlite\"\nsqlite_path = \"{database}\"\n\n[paths]\nmodels = \"{models}\"\nmigrations = \"{migrations}\"\nseeders = \"{seeders}\"\nfactories = \"{factories}\"\nconfig_file = \"{config_file}\"\n",
                database = slash_path(root.join("app.db")),
                models = slash_path(root.join("src").join("models")),
                migrations = slash_path(root.join("src").join("migrations")),
                seeders = slash_path(root.join("src").join("seeders")),
                factories = slash_path(root.join("src").join("factories")),
                config_file = slash_path(root.join("src").join("config.rs")),
            );
            fs::write(&config_path, contents).expect("config should be written");

            Self {
                _dir: dir,
                root,
                config_path: slash_path(&config_path),
            }
        }

        fn config_path(&self) -> &str {
            &self.config_path
        }

        fn path(&self, relative: &str) -> String {
            slash_path(self.root.join(relative))
        }
    }

    fn slash_path(path: impl AsRef<Path>) -> String {
        path.as_ref().to_string_lossy().replace('\\', "/")
    }
}
