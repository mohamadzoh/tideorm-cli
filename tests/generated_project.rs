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