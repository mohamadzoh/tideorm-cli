use super::generate_migration;
use std::fs;
use tempfile::TempDir;

#[tokio::test]
async fn generate_reports_a_broken_config_instead_of_defaulting_to_postgres() {
    let dir = TempDir::new().expect("temp dir should be created");
    let config_path = dir.path().join("tideorm.toml");
    fs::write(&config_path, "[database]\ndriver = \n").expect("config should be written");

    let error = generate_migration(
        config_path.to_str().expect("config path should be utf-8"),
        "create_users_table",
        None,
        None,
        None,
        false,
    )
    .await
    .expect_err("a malformed tideorm.toml must not fall back to the Postgres defaults");

    assert!(error.contains("Failed to parse config file"), "{}", error);
}
