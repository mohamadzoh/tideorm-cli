use assert_cmd::prelude::*;
use std::process::Command;
use tempfile::TempDir;

fn format_output(output: &std::process::Output) -> String {
    format!(
        "stdout:\n{}\n\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn generated_project_builds_after_model_generation() -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = TempDir::new()?;
    let project_dir = temp_dir.path().join("sample_app");
    let project_dir_arg = project_dir.to_string_lossy().into_owned();

    Command::cargo_bin("tideorm")?
        .env("TIDEORM_NONINTERACTIVE", "1")
        .args(["init", &project_dir_arg, "--database", "sqlite"])
        .assert()
        .success();

    Command::cargo_bin("tideorm")?
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
        .args(["check", "--offline"])
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
fn interactive_init_accepts_scripted_postgres_answers() -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = TempDir::new()?;
    let project_dir = temp_dir.path().join("postgres_app");
    let project_dir_arg = project_dir.to_string_lossy().into_owned();

    Command::cargo_bin("tideorm")?
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

    Command::cargo_bin("tideorm")?
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
fn generated_sqlite_project_tracks_migrations_across_repeated_runs(
) -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = TempDir::new()?;
    let project_dir = temp_dir.path().join("sqlite_app");
    let project_dir_arg = project_dir.to_string_lossy().into_owned();

    Command::cargo_bin("tideorm")?
        .env("TIDEORM_NONINTERACTIVE", "1")
        .args(["init", &project_dir_arg, "--database", "sqlite"])
        .assert()
        .success();

    Command::cargo_bin("tideorm")?
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

    let first_run = Command::cargo_bin("tideorm")?
        .current_dir(&project_dir)
        .args(["migrate", "run"])
        .output()?;
    assert!(
        first_run.status.success(),
        "first migrate run failed\n{}",
        format_output(&first_run)
    );

    let second_run = Command::cargo_bin("tideorm")?
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

    let history = Command::cargo_bin("tideorm")?
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