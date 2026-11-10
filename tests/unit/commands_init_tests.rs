use super::options::DatabaseInit;
use super::{
    InitOptions, SCAFFOLD_TIDEORM_VERSION, generate_cargo_toml, generate_config_rs,
    generate_gitignore, generate_tideorm_toml, infer_package_name, normalize_database,
    read_env_value, run, upsert_env_value,
};
use crate::config::server_url;
use std::fs;
use std::sync::LazyLock;
use tempfile::TempDir;

static INIT_TEST_LOCK: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));

#[test]
fn generated_cargo_toml_uses_current_tideorm_version() {
    let cargo_toml = generate_cargo_toml("demo_app", "sqlite");

    assert!(cargo_toml.contains(&format!(
        "tideorm = {{ version = \"{}\"",
        SCAFFOLD_TIDEORM_VERSION
    )));
    assert!(cargo_toml.contains("serde = { version = \"1\", features = [\"derive\"] }"));
    assert!(cargo_toml.contains("serde_json = \"1\""));
    assert!(cargo_toml.contains("chrono = \"0.4\""));
    // Without `default-features = false` a non-PostgreSQL project still builds the
    // PostgreSQL driver, because tideorm's defaults are ["postgres", "runtime-tokio"].
    assert!(cargo_toml.contains("default-features = false"));
    assert!(cargo_toml.contains("features = [\"sqlite\", \"runtime-tokio\"]"));
    assert!(cargo_toml.contains("entity-manager = [\"tideorm/entity-manager\"]"));
}

#[test]
fn infer_package_name_falls_back_for_empty_paths() {
    let package_name = infer_package_name(std::path::Path::new(""));
    assert_eq!(package_name, "tideorm_app");
}

/// Cargo refuses a package name that starts with a digit.
#[test]
fn infer_package_name_prefixes_names_cargo_or_rust_refuse() {
    let infer = |name: &str| infer_package_name(std::path::Path::new(name));
    assert_eq!(infer("2048-game"), "app_2048_game");
    assert_eq!(infer("match"), "app_match");
    assert_eq!(infer("shop-app"), "shop_app");
}

#[test]
fn init_database_names_follow_the_shared_driver_aliases() {
    assert_eq!(normalize_database("mariadb").unwrap(), "mysql");
    assert_eq!(normalize_database("PostgreSQL").unwrap(), "postgres");
    assert_eq!(normalize_database("SQLite").unwrap(), "sqlite");
    assert!(normalize_database("unknown").is_err());
}

/// A Windows path written unescaped into a TOML basic string left the project
/// unparsable.
#[test]
fn generated_tideorm_toml_escapes_answers() {
    let options = InitOptions {
        env_file_name: ".env".to_string(),
        overwrite_config: true,
        database: DatabaseInit::Sqlite {
            path: "C:\\Users\\me\\app \"x\".db".to_string(),
        },
        replace_database_url: true,
        create_database_now: false,
        run_migrations_now: false,
    };

    let config: crate::config::TideConfig =
        toml::from_str(&generate_tideorm_toml(&options)).expect("tideorm.toml should parse");
    assert_eq!(
        config.database.sqlite_path.as_deref(),
        Some("C:\\Users\\me\\app \"x\".db")
    );
    assert_eq!(config.project.env_file, ".env");
}

/// The app read only the process environment, so `cargo run` in a new project
/// panicked without `DATABASE_URL` (or opened another SQLite file).
#[test]
fn generated_config_loads_the_env_file() {
    let config_rs = generate_config_rs("postgres", ".env.local");
    assert!(
        config_rs.contains("let _ = dotenvy::from_filename(\".env.local\");"),
        "{config_rs}"
    );
    assert!(generate_cargo_toml("demo_app", "postgres").contains("dotenvy = \"0.15\""));
}

#[test]
fn generate_tideorm_toml_uses_custom_env_file() {
    let options = InitOptions {
        env_file_name: ".env.local".to_string(),
        overwrite_config: true,
        database: DatabaseInit::Postgres {
            host: "localhost".to_string(),
            port: 5432,
            database: "app_db".to_string(),
            username: "postgres".to_string(),
            password: "secret".to_string(),
        },
        replace_database_url: true,
        create_database_now: false,
        run_migrations_now: false,
    };

    let toml = generate_tideorm_toml(&options);
    assert!(toml.contains("env_file = \".env.local\""));
    assert!(toml.contains("url = \"${DATABASE_URL}\""));
    assert!(toml.contains("database = \"app_db\""));
    // The production guards are useless if the scaffold pins the environment to a
    // literal, so `TIDEORM_ENV` has to be able to reach them.
    assert!(toml.contains("environment = \"${TIDEORM_ENV}\""));
}

#[test]
fn server_url_percent_encodes_credentials() {
    let url = server_url(
        "postgres",
        "a@b",
        "p@ss/w#1",
        "localhost",
        Some(5432),
        "app",
    );

    assert_eq!(url, "postgres://a%40b:p%40ss%2Fw%231@localhost:5432/app");
}

#[test]
fn server_url_brackets_ipv6_hosts() {
    let url = server_url("mysql", "root", "", "::1", Some(3306), "app");

    assert_eq!(url, "mysql://root@[::1]:3306/app");
}

#[test]
fn generated_gitignore_covers_secrets_and_build_output() {
    let gitignore = generate_gitignore(".env");

    assert!(gitignore.contains("/target"));
    assert!(gitignore.contains(".env"));
    assert!(gitignore.contains("*.db"));

    let custom = generate_gitignore("secrets.conf");
    assert!(custom.contains("secrets.conf"));
}

#[test]
fn upsert_env_value_updates_existing_database_url() {
    let dir = TempDir::new().unwrap();
    let env_path = dir.path().join(".env");
    fs::write(&env_path, "APP_NAME=demo\nDATABASE_URL=old\n").unwrap();

    upsert_env_value(&env_path, "DATABASE_URL", "new").unwrap();

    let updated = fs::read_to_string(&env_path).unwrap();
    assert!(updated.contains("APP_NAME=demo"));
    assert!(updated.contains("DATABASE_URL=new"));
    assert!(!updated.contains("DATABASE_URL=old"));
}

/// dotenvy reads `KEY = value` and `export KEY = value`, so they are replaced too, not
/// left beside a second assignment.
#[test]
fn upsert_env_value_replaces_assignments_spaced_around_the_equals_sign() {
    let dir = TempDir::new().unwrap();
    let env_path = dir.path().join(".env");
    fs::write(
        &env_path,
        "DATABASE_URL = old\nexport  DATABASE_URL =older\n",
    )
    .unwrap();

    upsert_env_value(&env_path, "DATABASE_URL", "new").unwrap();

    assert_eq!(
        fs::read_to_string(&env_path).unwrap(),
        "DATABASE_URL=new\nDATABASE_URL=new\n"
    );
}

#[test]
fn read_env_value_uses_the_last_duplicate_key() {
    let dir = TempDir::new().unwrap();
    let env_path = dir.path().join(".env");
    fs::write(&env_path, "DATABASE_URL=first\nexport DATABASE_URL=last\n").unwrap();

    assert_eq!(
        read_env_value(&env_path, "DATABASE_URL").unwrap(),
        Some("last".to_string())
    );
}

/// The value is read as the project's dotenvy reads it, so an unchanged
/// quoted URL is not taken for a different one.
#[test]
fn read_env_value_reads_quoted_values_as_dotenvy_does() {
    let dir = TempDir::new().unwrap();
    let env_path = dir.path().join(".env");
    fs::write(&env_path, "DATABASE_URL=\"sqlite://app.db\"  # local\n").unwrap();

    assert_eq!(
        read_env_value(&env_path, "DATABASE_URL").unwrap(),
        Some("sqlite://app.db".to_string())
    );
}

#[tokio::test]
async fn run_restores_working_directory() {
    let _guard = INIT_TEST_LOCK.lock().await;
    let workspace = TempDir::new().unwrap();
    let original_dir = std::env::current_dir().unwrap();
    unsafe {
        std::env::set_var("TIDEORM_NONINTERACTIVE", "1");
    }
    std::env::set_current_dir(workspace.path()).unwrap();

    let result = run("generated", "sqlite", false).await;
    let restored_dir = std::env::current_dir().unwrap();

    std::env::set_current_dir(&original_dir).unwrap();
    unsafe {
        std::env::remove_var("TIDEORM_NONINTERACTIVE");
    }

    assert!(result.is_ok());
    assert_eq!(restored_dir, workspace.path());
}

#[tokio::test]
async fn run_scaffolds_a_library_with_a_seeder_runner() {
    let _guard = INIT_TEST_LOCK.lock().await;
    let workspace = TempDir::new().unwrap();
    let original_dir = std::env::current_dir().unwrap();
    unsafe {
        std::env::set_var("TIDEORM_NONINTERACTIVE", "1");
    }
    std::env::set_current_dir(workspace.path()).unwrap();

    let result = run("shop-app", "sqlite", false).await;

    std::env::set_current_dir(&original_dir).unwrap();
    unsafe {
        std::env::remove_var("TIDEORM_NONINTERACTIVE");
    }
    assert!(result.is_ok(), "init failed: {:?}", result);

    let project = workspace.path().join("shop-app");
    let read = |path: &str| fs::read_to_string(project.join(path)).unwrap();
    assert!(read("Cargo.toml").contains("default-run = \"shop_app\""));
    assert!(read("src/lib.rs").contains("pub mod seeders;"));
    assert!(read("src/main.rs").contains("shop_app::config::connect_tideorm()"));
    // The runners connect without the app's startup work.
    for (runner, registry) in [
        ("src/bin/shop_app-seed.rs", "shop_app::seeders::all()"),
        ("src/bin/shop_app-migrate.rs", "shop_app::migrations::all()"),
    ] {
        let source = read(runner);
        assert!(source.contains(registry), "{source}");
        assert!(
            source.contains("shop_app::config::tideorm_config().connect_only()"),
            "{source}"
        );
    }
    assert!(read("src/migrations/mod.rs").contains("pub fn all() -> Migrator {"));
    assert!(
        read("src/seeders/mod.rs")
            .contains("        .add(database_seeder::DatabaseSeeder::default())")
    );
}

#[tokio::test]
async fn run_names_the_app_as_default_run_in_an_existing_manifest() {
    let _guard = INIT_TEST_LOCK.lock().await;
    let workspace = TempDir::new().unwrap();
    let project = workspace.path().join("existing-app");
    fs::create_dir_all(&project).unwrap();
    fs::write(
        project.join("Cargo.toml"),
        "[package]\nname = \"existing-app\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[workspace.metadata]\ndefault-runner = \"not-the-package-setting\"\n",
    )
    .unwrap();
    let original_dir = std::env::current_dir().unwrap();
    unsafe {
        std::env::set_var("TIDEORM_NONINTERACTIVE", "1");
    }
    std::env::set_current_dir(workspace.path()).unwrap();

    let result = run("existing-app", "sqlite", false).await;

    std::env::set_current_dir(&original_dir).unwrap();
    unsafe {
        std::env::remove_var("TIDEORM_NONINTERACTIVE");
    }
    assert!(result.is_ok(), "init failed: {:?}", result);

    // The seeder runner is a second binary, so a plain `cargo run` needs
    // to be told which one is the app.
    let manifest = fs::read_to_string(project.join("Cargo.toml")).unwrap();
    assert!(
        manifest.starts_with("[package]\ndefault-run = \"existing-app\"\nname"),
        "{manifest}"
    );
}

#[tokio::test]
async fn run_keeps_existing_config_without_mutating_env_file() {
    let _guard = INIT_TEST_LOCK.lock().await;
    let workspace = TempDir::new().unwrap();
    let project_dir = workspace.path().join("existing_project");
    fs::create_dir_all(&project_dir).unwrap();
    fs::write(
        project_dir.join("tideorm.toml"),
        "[project]\nname = \"demo\"\n",
    )
    .unwrap();
    fs::write(project_dir.join(".env"), "DATABASE_URL=preserve-me\n").unwrap();

    let original_dir = std::env::current_dir().unwrap();
    unsafe {
        std::env::set_var("TIDEORM_NONINTERACTIVE", "1");
    }
    std::env::set_current_dir(workspace.path()).unwrap();

    let result = run(project_dir.to_str().unwrap(), "sqlite", false).await;
    let env_contents = fs::read_to_string(project_dir.join(".env")).unwrap();
    let manifest = fs::read_to_string(project_dir.join("Cargo.toml")).unwrap();
    let app_config = fs::read_to_string(project_dir.join("src/config.rs")).unwrap();

    std::env::set_current_dir(&original_dir).unwrap();
    unsafe {
        std::env::remove_var("TIDEORM_NONINTERACTIVE");
    }

    assert!(result.is_ok());
    assert_eq!(env_contents, "DATABASE_URL=preserve-me\n");
    assert!(manifest.contains("features = [\"postgres\", \"runtime-tokio\"]"));
    assert!(app_config.contains("PostgreSQL setup"));
}

/// The unguarded branch: no `tideorm.toml`, so `init` does write the project config,
/// but an env file that already carries a working `DATABASE_URL` must survive it.
#[tokio::test]
async fn run_keeps_existing_database_url_without_tideorm_toml() {
    let _guard = INIT_TEST_LOCK.lock().await;
    let workspace = TempDir::new().unwrap();
    let project_dir = workspace.path().join("adopted_project");
    fs::create_dir_all(&project_dir).unwrap();
    fs::write(
        project_dir.join(".env"),
        "DATABASE_URL=sqlite://existing.db\n",
    )
    .unwrap();

    let original_dir = std::env::current_dir().unwrap();
    unsafe {
        std::env::set_var("TIDEORM_NONINTERACTIVE", "1");
    }
    std::env::set_current_dir(workspace.path()).unwrap();

    let result = run(project_dir.to_str().unwrap(), "postgres", false).await;
    let env_contents = fs::read_to_string(project_dir.join(".env")).unwrap();
    let gitignore = fs::read_to_string(project_dir.join(".gitignore")).unwrap();
    let manifest = fs::read_to_string(project_dir.join("Cargo.toml")).unwrap();
    let app_config = fs::read_to_string(project_dir.join("src/config.rs")).unwrap();
    let config_exists = project_dir.join("tideorm.toml").exists();

    std::env::set_current_dir(&original_dir).unwrap();
    unsafe {
        std::env::remove_var("TIDEORM_NONINTERACTIVE");
    }

    assert!(result.is_ok(), "init failed: {:?}", result);
    assert!(config_exists);
    assert_eq!(env_contents, "DATABASE_URL=sqlite://existing.db\n");
    assert!(gitignore.contains(".env"));
    assert!(manifest.contains("features = [\"sqlite\", \"runtime-tokio\"]"));
    assert!(app_config.contains("SQLite setup"));
}
