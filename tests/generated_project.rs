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

/// Scaffold a SQLite project. With a `tideorm` checkout beside the CLI's, it builds
/// against that checkout; without one (CI), against crates.io, as a user's would.
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
        .join("tideorm");
    if !tideorm_path.join("Cargo.toml").exists() {
        return Ok(());
    }
    let tideorm_path = tideorm_path.to_string_lossy().replace('\\', "/");
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
    // A model with no unique field gets no method using the prelude it imports.
    tideorm()?
        .current_dir(&project_dir)
        .args(["make", "model", "Tag", "--fields", "label:string"])
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

    // The app itself loads `.env`, where `tideorm init` wrote `DATABASE_URL`: pointed
    // at another file there, it opens that one rather than a built-in default. (Built
    // here, where the seeder runner already compiled everything it needs.)
    std::fs::write(
        project_dir.join(".env"),
        "DATABASE_URL=sqlite://from_env.db?mode=rwc\n",
    )?;
    let app = Command::new("cargo")
        .args(["run", "--quiet", "--offline", "--bin", "seeded_app"])
        .current_dir(&project_dir)
        .env_remove("DATABASE_URL")
        .output()?;
    assert!(
        app.status.success(),
        "the generated app failed to start\n{}",
        format_output(&app)
    );
    assert!(
        project_dir.join("from_env.db").exists(),
        "the generated app did not open the database its .env names\n{}",
        format_output(&app)
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
        .env("CARGO_NET_OFFLINE", "true")
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

/// `tideorm migrate` runs the project's migrations through its migration
/// runner, which records them in the ledger, without the app's startup work.
#[test]
fn generated_sqlite_project_runs_its_migrations() -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = TempDir::new()?;
    let project_dir = temp_dir.path().join("sqlite_app");
    init_sqlite_project(&project_dir)?;

    // Startup work the app's configuration enables, which the runner skips.
    let config_rs = project_dir.join("src").join("config.rs");
    let config = std::fs::read_to_string(&config_rs)?;
    assert!(config.contains(".min_connections(1)"), "{config}");
    std::fs::write(
        &config_rs,
        config.replace(
            ".min_connections(1)",
            ".min_connections(1)\n        .schema_file(\"startup_schema.sql\")",
        ),
    )?;
    let schema_file = project_dir.join("startup_schema.sql");

    for args in [
        [
            "create_users_table",
            "--create=users",
            "--fields=name:string,email:string:unique",
        ],
        [
            "add_age_to_users",
            "--table=users",
            "--fields=age:i32:nullable",
        ],
    ] {
        tideorm()?
            .current_dir(&project_dir)
            .args(["make", "migration"])
            .args(args)
            .assert()
            .success();
    }

    let run = |args: &[&str]| -> Result<String, Box<dyn std::error::Error>> {
        let output = tideorm()?
            .current_dir(&project_dir)
            .env("CARGO_NET_OFFLINE", "true")
            .args(args)
            .output()?;
        assert!(
            output.status.success(),
            "tideorm {} failed\n{}",
            args.join(" "),
            format_output(&output)
        );
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    };

    // Pretending shows the SQL and changes nothing.
    let pretend = run(&["migrate", "run", "--pretend"])?;
    assert!(pretend.contains("CREATE TABLE"), "{pretend}");
    let status = run(&["migrate", "status"])?;
    assert!(
        status.contains("Total: 2 | Ran: 0 | Pending: 2"),
        "{status}"
    );
    tideorm()?
        .current_dir(&project_dir)
        .env("CARGO_NET_OFFLINE", "true")
        .args(["migrate", "status", "--check"])
        .assert()
        .failure();

    let first = run(&["migrate", "run"])?;
    assert!(first.contains("Ran 2 migration(s) successfully"), "{first}");
    run(&["migrate", "status", "--check"])?;
    let second = run(&["migrate", "run"])?;
    assert!(second.contains("Nothing to migrate"), "{second}");

    let history = run(&["migrate", "history"])?;
    assert!(history.contains("create_users_table"), "{history}");
    assert!(history.contains("add_age_to_users"), "{history}");

    let down = run(&["migrate", "down"])?;
    assert!(down.contains("Rolling back: "), "{down}");
    assert!(down.contains("_add_age_to_users"), "{down}");
    let status = run(&["migrate", "status"])?;
    assert!(
        status.contains("Total: 2 | Ran: 1 | Pending: 1"),
        "{status}"
    );

    let redo = run(&["migrate", "redo"])?;
    assert!(redo.contains("Redid 1 migration(s)"), "{redo}");
    assert!(redo.contains("_create_users_table"), "{redo}");

    let up = run(&["migrate", "up", "--migration", "add_age_to_users"])?;
    assert!(up.contains("completed"), "{up}");
    let check = run(&["db", "check"])?;
    assert!(check.contains("2 registered"), "{check}");
    assert!(
        !schema_file.exists(),
        "the migration runner ran the app's startup work"
    );

    // A batch that fails part-way reports the migrations it applied before
    // the failure: each commits on its own, so they stay applied.
    tideorm()?
        .current_dir(&project_dir)
        .args(["make", "migration", "break_the_batch"])
        .assert()
        .success();
    let broken = std::fs::read_dir(project_dir.join("src").join("migrations"))?
        .flatten()
        .map(|entry| entry.path())
        .find(|path| path.to_string_lossy().ends_with("_break_the_batch.rs"))
        .expect("the migration should be generated");
    let source = std::fs::read_to_string(&broken)?;
    std::fs::write(
        &broken,
        source.replacen(
            "let _ = schema; // Remove once implemented.",
            "schema.raw(\"NOT A STATEMENT\").await?;",
            1,
        ),
    )?;
    run(&["migrate", "reset"])?;
    let failed = tideorm()?
        .current_dir(&project_dir)
        .env("CARGO_NET_OFFLINE", "true")
        .args(["migrate", "run"])
        .output()?;
    let report = format_output(&failed);
    assert!(!failed.status.success(), "{report}");
    assert!(report.contains("Migration failed"), "{report}");
    let migrated: Vec<&str> = report
        .lines()
        .filter(|line| line.contains("Migrating: "))
        .collect();
    assert_eq!(migrated.len(), 2, "{report}");
    assert!(migrated[0].contains("_create_users_table"), "{report}");
    assert!(migrated[1].contains("_add_age_to_users"), "{report}");
    let status = run(&["migrate", "status"])?;
    assert!(
        status.contains("Total: 3 | Ran: 2 | Pending: 1"),
        "{status}"
    );

    // The app itself still does its startup work.
    let app = Command::new("cargo")
        .args(["run", "--quiet", "--offline", "--bin", "sqlite_app"])
        .current_dir(&project_dir)
        .env_remove("DATABASE_URL")
        .output()?;
    assert!(
        app.status.success(),
        "the generated app failed to start\n{}",
        format_output(&app)
    );
    assert!(schema_file.exists(), "{}", format_output(&app));

    Ok(())
}
