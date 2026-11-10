use super::{
    make_factory, make_migration, make_model, make_seeder, output_override,
    prepare_model_migration_fields, resolve_table_name,
};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

#[test]
fn listed_modifiers_reach_the_migration_fields() {
    assert_eq!(
        super::with_listed_modifiers(
            "title:string,summary:text,email:string:unique",
            &[
                ("nullable", Some("summary")),
                ("unique", Some("email, title")),
                ("indexed", None),
            ],
        ),
        "title:string:unique,summary:text:nullable,email:string:unique"
    );
}

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
fn output_override_preserves_every_explicit_directory() {
    assert_eq!(output_override(None), Ok(None));
    assert_eq!(output_override(Some("src/models")), Ok(Some("src/models")));
    assert_eq!(
        output_override(Some(" db/seeders ")),
        Ok(Some(" db/seeders "))
    );
    assert_eq!(
        output_override(Some("")),
        Err("--output requires a non-empty directory".to_string())
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
        Some(output.path()),
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
        Some(output.path()),
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
        Some(output.path()),
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
        Some(model_output.as_str()),
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

/// A companion file that fails its checks leaves no model behind: every file is
/// checked before any is written.
#[tokio::test]
async fn make_model_writes_nothing_when_a_companion_file_is_refused() {
    let project = TestProject::new();

    let error = make_model(
        project.config_path(),
        "User",
        None,
        Some("active:bool:unique,price:DECIMAL(10, 2)".to_string()),
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
        None,
        true,
        true,
        true,
        false,
    )
    .await
    .unwrap_err();

    assert!(error.contains("boolean field 'active'"), "{error}");
    for dir in [
        "src/models",
        "src/migrations",
        "src/seeders",
        "src/factories",
    ] {
        assert!(!Path::new(&project.path(dir)).exists(), "{dir} was written");
    }
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
            "[project]\nname = \"demo\"\nenvironment = \"development\"\n\n[database]\ndriver = \"sqlite\"\nsqlite_path = \"{database}\"\n\n[paths]\nmodels = \"{models}\"\nmigrations = \"{migrations}\"\nseeders = \"{seeders}\"\nfactories = \"{factories}\"\n",
            database = slash_path(root.join("app.db")),
            models = slash_path(root.join("src").join("models")),
            migrations = slash_path(root.join("src").join("migrations")),
            seeders = slash_path(root.join("src").join("seeders")),
            factories = slash_path(root.join("src").join("factories")),
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
