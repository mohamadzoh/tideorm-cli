use assert_cmd::prelude::*;
use std::process::Command;
use tempfile::TempDir;

/// The CLI under test, without the shell's `DATABASE_URL` or `TIDEORM_ENV`,
/// which it prefers over a project's own `.env`: a developer's would point a
/// scaffold at their database, or refuse to seed it as production.
fn tideorm() -> Result<Command, Box<dyn std::error::Error>> {
    let mut command = Command::cargo_bin("tideorm")?;
    command.env_remove("DATABASE_URL").env_remove("TIDEORM_ENV");
    Ok(command)
}

fn format_output(output: &std::process::Output) -> String {
    format!(
        "stdout:\n{}\n\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// Scaffold a SQLite project that builds against the sibling `tideorm` checkout.
fn init_sqlite_project(project_dir: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    tideorm()?
        .env("TIDEORM_NONINTERACTIVE", "1")
        .args([
            "init",
            &project_dir.to_string_lossy(),
            "--database",
            "sqlite",
        ])
        .assert()
        .success();

    let tideorm_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("CLI crate should have a parent directory")
        .join("tideorm")
        .to_string_lossy()
        .replace('\\', "/");
    let cargo_toml_path = project_dir.join("Cargo.toml");
    let mut cargo_toml = std::fs::read_to_string(&cargo_toml_path)?;
    cargo_toml.push_str(&format!(
        "\n[patch.crates-io]\ntideorm = {{ path = \"{}\" }}\n",
        tideorm_path
    ));
    std::fs::write(&cargo_toml_path, cargo_toml)?;
    Ok(())
}

#[test]
fn generated_project_builds_after_model_generation() -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = TempDir::new()?;
    let project_dir = temp_dir.path().join("sample_app");
    init_sqlite_project(&project_dir)?;

    tideorm()?
        .current_dir(&project_dir)
        .args([
            "make",
            "model",
            "User",
            "--fields",
            "name:string,email:string:unique",
            "--factory",
            "--seeder",
            "--migration",
        ])
        .assert()
        .success();

    let output = Command::new("cargo")
        .args(["check", "--offline", "--all-targets"])
        .current_dir(&project_dir)
        .output()?;

    assert!(
        output.status.success(),
        "generated project failed to build\n{}",
        format_output(&output)
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("warning:"),
        "generated project produced warnings\n{}",
        format_output(&output)
    );

    Ok(())
}

#[test]
fn generated_sqlite_project_runs_its_seeders() -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = TempDir::new()?;
    let project_dir = temp_dir.path().join("seeded_app");
    init_sqlite_project(&project_dir)?;

    tideorm()?
        .current_dir(&project_dir)
        .args(["make", "seeder", "Demo"])
        .assert()
        .success();

    let seed = |args: &[&str]| -> Result<String, Box<dyn std::error::Error>> {
        let output = tideorm()?
            .current_dir(&project_dir)
            .env("CARGO_NET_OFFLINE", "true")
            .args(["db", "seed"])
            .args(args)
            .output()?;
        assert!(
            output.status.success(),
            "db seed failed\n{}",
            format_output(&output)
        );
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    };

    let first = seed(&[])?;
    assert!(first.contains("ran      database_seeder"), "{}", first);
    assert!(first.contains("ran      demo_seeder"), "{}", first);

    let second = seed(&[])?;
    assert!(
        second.contains("skipped  demo_seeder (already ran)"),
        "{}",
        second
    );

    let rerun = seed(&["--seeder", "DemoSeeder"])?;
    assert!(rerun.contains("ran      demo_seeder"), "{}", rerun);
    assert!(!rerun.contains("database_seeder"), "{}", rerun);

    // `make seeder Demo` named it `DemoSeeder`, which `Demo` finds too.
    let short = seed(&["--seeder", "Demo"])?;
    assert!(short.contains("ran      demo_seeder"), "{}", short);

    // A seeder name that matches none is refused before anything is dropped:
    // the ledger survives, so the seeders count as run.
    let unknown = tideorm()?
        .current_dir(&project_dir)
        .env("CARGO_NET_OFFLINE", "true")
        .args(["migrate", "fresh", "--seed", "--seeder", "Nope", "--force"])
        .output()?;
    let report = format_output(&unknown);
    assert!(!unknown.status.success(), "{}", report);
    assert!(report.contains("No seeder named `Nope`"), "{}", report);
    assert!(report.contains("demo_seeder"), "{}", report);
    let after = seed(&[])?;
    assert!(
        after.contains("skipped  demo_seeder (already ran)"),
        "{}",
        after
    );

    Ok(())
}

/// A field whose name the model would snake-case differently (`sha256` derives
/// `sha_256`) still reads and writes the column its migration created.
#[test]
fn generated_model_and_migration_agree_on_digit_columns() -> Result<(), Box<dyn std::error::Error>>
{
    let temp_dir = TempDir::new()?;
    let project_dir = temp_dir.path().join("digits_app");
    init_sqlite_project(&project_dir)?;

    tideorm()?
        .current_dir(&project_dir)
        .args([
            "make",
            "model",
            "Checksum",
            "--fields=sha256:string,line_1:string",
            "--migration",
            "--seeder",
        ])
        .assert()
        .success();

    let migrate = tideorm()?
        .current_dir(&project_dir)
        .args(["migrate", "run"])
        .output()?;
    assert!(
        migrate.status.success(),
        "migrate run failed
{}",
        format_output(&migrate)
    );

    // The generated seeder writes a value into every column.
    let seed = tideorm()?
        .current_dir(&project_dir)
        .env("CARGO_NET_OFFLINE", "true")
        .args(["db", "seed", "--seeder", "ChecksumSeeder"])
        .output()?;
    assert!(
        seed.status.success(),
        "seeding the generated model failed
{}",
        format_output(&seed)
    );

    Ok(())
}

#[test]
fn interactive_init_accepts_scripted_postgres_answers() -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = TempDir::new()?;
    let project_dir = temp_dir.path().join("postgres_app");
    let project_dir_arg = project_dir.to_string_lossy().into_owned();

    tideorm()?
        .env_remove("TIDEORM_NONINTERACTIVE")
        .env(
            "TIDEORM_PROMPT_SCRIPT",
            ".env.local\nlocalhost\n5433\napp_db\napp_user\nsecret\ny\nn\n",
        )
        .args(["init", &project_dir_arg, "--database", "postgres"])
        .assert()
        .success();

    let env_contents = std::fs::read_to_string(project_dir.join(".env.local"))?;
    let config_contents = std::fs::read_to_string(project_dir.join("tideorm.toml"))?;

    assert!(env_contents.contains("DATABASE_URL=postgres://app_user:secret@localhost:5433/app_db"));
    assert!(config_contents.contains("env_file = \".env.local\""));
    assert!(config_contents.contains("driver = \"postgres\""));
    assert!(config_contents.contains("port = 5433"));

    Ok(())
}

#[test]
fn interactive_init_accepts_scripted_mysql_answers() -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = TempDir::new()?;
    let project_dir = temp_dir.path().join("mysql_app");
    let project_dir_arg = project_dir.to_string_lossy().into_owned();

    tideorm()?
        .env_remove("TIDEORM_NONINTERACTIVE")
        .env(
            "TIDEORM_PROMPT_SCRIPT",
            ".env.mysql\n127.0.0.1\n3307\nshop_db\nshop_user\nsecret\ny\nn\n",
        )
        .args(["init", &project_dir_arg, "--database", "mysql"])
        .assert()
        .success();

    let env_contents = std::fs::read_to_string(project_dir.join(".env.mysql"))?;
    let config_contents = std::fs::read_to_string(project_dir.join("tideorm.toml"))?;

    assert!(env_contents.contains("DATABASE_URL=mysql://shop_user:secret@127.0.0.1:3307/shop_db"));
    assert!(config_contents.contains("env_file = \".env.mysql\""));
    assert!(config_contents.contains("driver = \"mysql\""));
    assert!(config_contents.contains("port = 3307"));

    Ok(())
}

#[test]
fn generated_sqlite_project_tracks_migrations_across_repeated_runs()
-> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = TempDir::new()?;
    let project_dir = temp_dir.path().join("sqlite_app");
    let project_dir_arg = project_dir.to_string_lossy().into_owned();

    tideorm()?
        .env("TIDEORM_NONINTERACTIVE", "1")
        .args(["init", &project_dir_arg, "--database", "sqlite"])
        .assert()
        .success();

    tideorm()?
        .current_dir(&project_dir)
        .args([
            "make",
            "migration",
            "create_users_table",
            "--create=users",
            "--fields=name:string,email:string:unique",
        ])
        .assert()
        .success();

    let first_run = tideorm()?
        .current_dir(&project_dir)
        .args(["migrate", "run"])
        .output()?;
    assert!(
        first_run.status.success(),
        "first migrate run failed\n{}",
        format_output(&first_run)
    );

    let second_run = tideorm()?
        .current_dir(&project_dir)
        .args(["migrate", "run"])
        .output()?;
    assert!(
        second_run.status.success(),
        "second migrate run failed\n{}",
        format_output(&second_run)
    );

    let second_stdout = String::from_utf8_lossy(&second_run.stdout);
    assert!(
        second_stdout.contains("Nothing to migrate"),
        "expected second run to skip applied migration\n{}",
        format_output(&second_run)
    );

    let history = tideorm()?
        .current_dir(&project_dir)
        .args(["migrate", "history"])
        .output()?;
    assert!(
        history.status.success(),
        "migrate history failed\n{}",
        format_output(&history)
    );

    let history_stdout = String::from_utf8_lossy(&history.stdout);
    assert!(
        history_stdout.contains("create_users_table"),
        "expected migration history to include the applied migration\n{}",
        format_output(&history)
    );

    Ok(())
}
