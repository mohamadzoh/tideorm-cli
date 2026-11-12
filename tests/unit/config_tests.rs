use super::env::{parse_env_value, unresolved_placeholders};
use super::*;
use std::collections::HashMap;
use std::fs;
use tempfile::TempDir;

#[test]
fn test_default_config() {
    let config = TideConfig::default();
    assert_eq!(config.database.driver, "postgres");
    assert_eq!(config.paths.models, "src/models");
}

#[test]
fn test_connection_url_postgres() {
    let config = DatabaseConfig {
        driver: "postgres".to_string(),
        username: Some("user".to_string()),
        password: Some("pass".to_string()),
        database: Some("mydb".to_string()),
        ..Default::default()
    };

    let url = config.connection_url();
    assert!(url.starts_with("postgres://"));
    assert!(url.contains("user"));
    assert!(url.contains("mydb"));
}

#[test]
fn test_connection_url_sqlite() {
    let config = DatabaseConfig {
        driver: "sqlite".to_string(),
        sqlite_path: Some("test.db".to_string()),
        ..Default::default()
    };

    let url = config.connection_url();
    assert_eq!(url, "sqlite://test.db");
}

#[test]
fn test_connection_url_sqlite_windows_absolute_path() {
    let config = DatabaseConfig {
        driver: "sqlite".to_string(),
        sqlite_path: Some("C:\\Users\\alice\\project\\data.db".to_string()),
        ..Default::default()
    };

    let url = config.connection_url();
    assert_eq!(url, "sqlite:///C:/Users/alice/project/data.db");
}

#[test]
fn test_load_uses_database_url_from_dotenv() {
    let fixture = TempDir::new().unwrap();
    let config_path = fixture.path().join("tideorm.toml");
    let env_path = fixture.path().join(".env");

    fs::write(
        &config_path,
        r#"[project]
name = "demo"

[database]
driver = "postgres"
host = "localhost"
port = 5432
database = "tideorm_db"
username = "postgres"
password = ""
"#,
    )
    .unwrap();
    fs::write(
        &env_path,
        "DATABASE_URL=postgres://postgres:postgres@localhost:5432/test_tide_ormx\n",
    )
    .unwrap();

    let config = TideConfig::load(config_path.to_str().unwrap()).unwrap();

    assert_eq!(
        config.database.url.as_deref(),
        Some("postgres://postgres:postgres@localhost:5432/test_tide_ormx")
    );
    assert_eq!(config.database.driver, "postgres");
    assert_eq!(config.database.host, "localhost");
    assert_eq!(config.database.port, Some(5432));
    assert_eq!(config.database.database.as_deref(), Some("test_tide_ormx"));
    assert_eq!(config.database.username.as_deref(), Some("postgres"));
    assert_eq!(config.database.password.as_deref(), Some("postgres"));
}

#[test]
fn test_load_expands_database_url_placeholder_from_dotenv() {
    let fixture = TempDir::new().unwrap();
    let config_path = fixture.path().join("tideorm.toml");
    let env_path = fixture.path().join(".env");

    fs::write(
        &config_path,
        r#"[database]
driver = "postgres"
url = "${DATABASE_URL}"
"#,
    )
    .unwrap();
    fs::write(
        &env_path,
        "DATABASE_URL=postgres://postgres:secret@db.example.com:5433/app_db\n",
    )
    .unwrap();

    let config = TideConfig::load(config_path.to_str().unwrap()).unwrap();

    assert_eq!(
        config.database.connection_url(),
        "postgres://postgres:secret@db.example.com:5433/app_db"
    );
    assert_eq!(config.database.host, "db.example.com");
    assert_eq!(config.database.port, Some(5433));
    assert_eq!(config.database.database.as_deref(), Some("app_db"));
    assert_eq!(config.database.username.as_deref(), Some("postgres"));
}

#[test]
fn the_url_names_where_it_came_from() {
    let fixture = TempDir::new().unwrap();
    let config_path = fixture.path().join("tideorm.toml");
    let load = |url: &str| {
        fs::write(
            &config_path,
            format!("[database]\ndriver = \"sqlite\"\n{url}"),
        )
        .unwrap();
        TideConfig::load(config_path.to_str().unwrap())
            .unwrap()
            .database
            .url_source
    };
    fs::write(
        fixture.path().join(".env"),
        "TIDEORM_TEST_SOURCE_URL=sqlite://from_env.db\nTIDEORM_TEST_SOURCE_FILE=part.db\n",
    )
    .unwrap();

    assert_eq!(
        load("url = \"${TIDEORM_TEST_SOURCE_URL}\"").as_deref(),
        Some("TIDEORM_TEST_SOURCE_URL in the env file")
    );
    assert_eq!(
        load("url = \"sqlite://${TIDEORM_TEST_SOURCE_FILE}\"").as_deref(),
        Some("database.url in tideorm.toml")
    );
    assert_eq!(load("url = \"${TIDEORM_TEST_UNSET_URL}\""), None);
    assert_eq!(load("sqlite_path = \"app.db\""), None);
}

#[test]
fn test_connection_url_percent_encodes_credentials() {
    let config = DatabaseConfig {
        driver: "postgres".to_string(),
        username: Some("a@b".to_string()),
        password: Some("p@ss:w/rd#1".to_string()),
        database: Some("mydb".to_string()),
        ..Default::default()
    };

    assert_eq!(
        config.connection_url(),
        "postgres://a%40b:p%40ss%3Aw%2Frd%231@localhost:5432/mydb"
    );
}

#[test]
fn test_connection_url_supports_mariadb() {
    let config = DatabaseConfig {
        driver: "mariadb".to_string(),
        database: Some("shop".to_string()),
        ..Default::default()
    };

    let url = config.connection_url();
    assert_eq!(url, "mysql://root@localhost:3306/shop");
}

#[test]
fn test_connection_url_does_not_panic_on_unknown_driver() {
    let config = DatabaseConfig {
        driver: "oracle".to_string(),
        database: Some("app".to_string()),
        ..Default::default()
    };

    assert!(config.try_connection_url().is_err());
    assert_eq!(config.connection_url(), "oracle://localhost/app");
}

#[test]
fn resolve_environment_prefers_tideorm_env() {
    let mut env_values = HashMap::new();
    env_values.insert("TIDEORM_ENV".to_string(), "production".to_string());
    let resolved = resolve_environment("development", &env_values);

    assert_eq!(resolved, "production");
}

#[test]
fn resolve_environment_falls_back_for_unresolved_placeholder() {
    assert_eq!(
        resolve_environment("${TIDEORM_ENV}", &HashMap::new()),
        default_environment()
    );
}

#[test]
fn test_load_environment_placeholder_enables_production_guards() {
    let fixture = TempDir::new().unwrap();
    let config_path = fixture.path().join("tideorm.toml");

    fs::write(
        &config_path,
        r#"[project]
name = "demo"
environment = "${TIDEORM_ENV}"

[database]
driver = "sqlite"
sqlite_path = "app.db"
"#,
    )
    .unwrap();
    fs::write(fixture.path().join(".env"), "TIDEORM_ENV=production\n").unwrap();

    let config = TideConfig::load(config_path.to_str().unwrap()).unwrap();

    assert_eq!(config.project.environment, "production");
    assert!(config.is_production());
}

#[test]
fn test_load_or_default_reports_broken_config() {
    let fixture = TempDir::new().unwrap();
    let config_path = fixture.path().join("tideorm.toml");
    fs::write(&config_path, "this is not = valid toml [\n").unwrap();

    let path = config_path.to_str().unwrap();
    assert!(TideConfig::load_or_default(path).is_err());

    let absent = fixture.path().join("missing.toml");
    let absent = absent.to_str().unwrap();
    assert!(TideConfig::load_or_default(absent).is_ok());
}

#[test]
fn test_load_or_default_does_not_fall_back_to_postgres_for_a_broken_config() {
    let fixture = TempDir::new().unwrap();
    let config_path = fixture.path().join("tideorm.toml");
    fs::write(
        &config_path,
        "[database]\ndriver = \"sqlite\"\nsqlite_path = \n",
    )
    .unwrap();

    let error = TideConfig::load_or_default(config_path.to_str().unwrap())
        .expect_err("a config file that exists but cannot be parsed must be reported");
    assert!(error.contains("Failed to parse config file"), "{}", error);
}

#[test]
fn load_or_default_does_not_treat_a_blocked_config_path_as_missing() {
    let fixture = TempDir::new().unwrap();
    let blocker = fixture.path().join("not-a-directory");
    fs::write(&blocker, "").unwrap();
    let config_path = blocker.join("tideorm.toml");

    let error = TideConfig::load_or_default(config_path.to_str().unwrap())
        .expect_err("an unreadable config path must not silently use default settings");
    assert!(error.contains("Failed to inspect config file"), "{error}");
}

#[test]
fn test_load_uses_custom_env_file_name() {
    let fixture = TempDir::new().unwrap();
    let config_path = fixture.path().join("tideorm.toml");
    let env_path = fixture.path().join(".env.local");

    fs::write(
        &config_path,
        r#"[project]
name = "demo"
env_file = ".env.local"

[database]
driver = "postgres"
"#,
    )
    .unwrap();
    fs::write(
        &env_path,
        "DATABASE_URL=postgres://postgres:postgres@localhost:5432/custom_env_db\n",
    )
    .unwrap();

    let config = TideConfig::load(config_path.to_str().unwrap()).unwrap();
    assert_eq!(config.project.env_file, ".env.local");
    assert_eq!(config.database.database.as_deref(), Some("custom_env_db"));
}

#[test]
fn load_reports_an_env_file_path_blocked_by_a_file() {
    let fixture = TempDir::new().unwrap();
    let config_path = fixture.path().join("tideorm.toml");
    let blocker = fixture.path().join("not-a-directory");
    fs::write(&blocker, "").unwrap();
    fs::write(
        &config_path,
        "[project]\nenv_file = \"not-a-directory/.env\"\n\n[database]\ndriver = \"sqlite\"\n",
    )
    .unwrap();

    let error = TideConfig::load(config_path.to_str().unwrap())
        .expect_err("a broken env-file path must not be silently ignored");
    assert!(error.contains("Failed to read env file"), "{error}");
}

fn applied(url: &str) -> DatabaseConfig {
    let mut config = DatabaseConfig::default();
    config.apply_connection_url(url).unwrap();
    config
}

#[test]
fn sqlite_urls_name_the_file_the_driver_opens() {
    let absolute = if cfg!(windows) {
        "C:/data/app.db"
    } else {
        "/C:/data/app.db"
    };
    for (url, file) in [
        ("sqlite://app.db?mode=rwc", "app.db"),
        ("SQLite://app.db", "app.db"),
        ("sqlite:app.db", "app.db"),
        ("sqlite:data/app.db?mode=rwc&cache=shared", "data/app.db"),
        ("sqlite:///var/data/app.db", "/var/data/app.db"),
        ("sqlite:///C:/data/app.db", absolute),
        ("sqlite://my%20data/app%3F.db", "my data/app?.db"),
    ] {
        let config = applied(url);
        assert_eq!(config.driver, "sqlite", "{url}");
        assert_eq!(config.sqlite_path.as_deref(), Some(file), "{url}");
        assert_eq!(config.sqlite_file().as_deref(), Some(file), "{url}");
    }

    for url in [
        "sqlite::memory:",
        "sqlite://:memory:",
        "sqlite://app.db?mode=memory",
        "sqlite://app.db?mode=%6Demory",
        "sqlite://app.db?%6Dode=memory",
    ] {
        let config = applied(url);
        assert_eq!(config.driver, "sqlite", "{url}");
        assert_eq!(config.sqlite_file(), None, "{url}");
    }
}

/// A URL its driver refuses fails the load instead of leaving the declarative
/// settings, or another file, in force; the error leaves the URL out.
#[test]
fn a_url_the_driver_refuses_fails_the_load() {
    for url in [
        "sqlite://app.db?mode=wat",
        "postgres://app:secret@db:99999/app",
        "mysql://app:secret@db/app?ssl-mode=wat",
    ] {
        let mut config = DatabaseConfig::default();
        let error = config.apply_connection_url(url).unwrap_err();
        assert!(error.contains("is not a valid"), "{error}");
        assert!(!error.contains("secret"), "{error}");
    }
}

#[test]
fn unresolved_database_url_placeholders_cover_both_supported_forms() {
    assert_eq!(
        unresolved_placeholders("sqlite://$DATABASE_URL"),
        ["DATABASE_URL"]
    );
    assert_eq!(
        unresolved_placeholders("sqlite://${DATABASE_URL}"),
        ["DATABASE_URL"]
    );
    assert!(unresolved_placeholders("sqlite://database.db").is_empty());
}

#[test]
fn sqlite_paths_survive_a_round_trip_through_their_url() {
    // The driver decodes `%20` and cuts at `?`, so both must be escaped.
    assert_eq!(sqlite_connection_url("a%20b.db"), "sqlite://a%2520b.db");
    assert_eq!(sqlite_connection_url("what?.db"), "sqlite://what%3F.db");
    assert_eq!(sqlite_connection_url("my app.db"), "sqlite://my%20app.db");
    for path in ["a%20b.db", "what?.db", "my app.db", "data/app.db"] {
        let config = applied(&sqlite_connection_url(path));
        assert_eq!(config.sqlite_file().as_deref(), Some(path));
    }
}

#[test]
fn a_sqlite_url_wins_over_the_configured_path() {
    let fixture = TempDir::new().unwrap();
    let config_path = fixture.path().join("tideorm.toml");
    fs::write(
        &config_path,
        "[database]\ndriver = \"sqlite\"\nsqlite_path = \"decoy.db\"\nurl = \"${TIDEORM_TEST_SQLITE_URL}\"\n",
    )
    .unwrap();
    fs::write(
        fixture.path().join(".env"),
        "TIDEORM_TEST_SQLITE_URL=sqlite:real.db?mode=rwc\n",
    )
    .unwrap();

    let config = TideConfig::load(config_path.to_str().unwrap()).unwrap();
    assert_eq!(config.database.sqlite_file().as_deref(), Some("real.db"));
}

#[test]
fn every_spelling_of_mariadb_and_mysql_is_the_mysql_driver() {
    for url in [
        "mariadb://root:secret@db:3306/shop",
        "MariaDB://root@db/shop",
        "MySQL://root@db/shop",
    ] {
        let config = applied(url);
        assert_eq!(config.driver, "mysql", "{url}");
        assert_eq!(config.database.as_deref(), Some("shop"), "{url}");
    }
    assert_eq!(applied("PostgreSQL://u@db/app").driver, "postgres");

    let mut config = DatabaseConfig {
        driver: "MariaDB".to_string(),
        ..Default::default()
    };
    config.resolve_with_env(&HashMap::new()).unwrap();
    assert_eq!(config.driver, "mysql");
}

#[test]
fn server_urls_bracket_ipv6_hosts() {
    let config = DatabaseConfig {
        driver: "postgres".to_string(),
        host: "::1".to_string(),
        username: Some("app".to_string()),
        password: Some("p?ss".to_string()),
        database: Some("app_db".to_string()),
        ..Default::default()
    };

    assert_eq!(
        config.connection_url(),
        "postgres://app:p%3Fss@[::1]:5432/app_db"
    );
    assert_eq!(
        config.server_url_for_database("postgres").unwrap(),
        "postgres://app:p%3Fss@[::1]:5432/postgres"
    );
    // A URL's bracketed host round-trips.
    assert_eq!(
        applied("postgres://app@[::1]:5433/app_db").host,
        "::1".to_string()
    );
}

fn parse(value: &str) -> Result<String, String> {
    let values = HashMap::from([("TIDEORM_TEST_X".to_string(), "x".to_string())]);
    parse_env_value(value, &values)
}

#[test]
fn env_file_values_drop_inline_comments() {
    assert_eq!(parse("production  # live"), Ok("production".to_string()));
    assert_eq!(parse("p#ss"), Ok("p#ss".to_string()));
    assert_eq!(parse("\"a # b\" # note"), Ok("a # b".to_string()));
    assert_eq!(
        parse("'${TIDEORM_TEST_X}'"),
        Ok("${TIDEORM_TEST_X}".to_string())
    );
    assert_eq!(parse("\"${TIDEORM_TEST_X}\""), Ok("x".to_string()));
    assert_eq!(parse("\"x\""), Ok("x".to_string()));
}

/// A double-quoted value reads dotenvy's escapes, and a backslash escaped before the
/// closing quote does not hide that quote.
#[test]
fn double_quoted_env_values_read_escapes_as_dotenvy_does() {
    assert_eq!(parse(r#""a\nb""#), Ok("a\nb".to_string()));
    assert_eq!(parse(r#""C:\\data\\""#), Ok(r"C:\data\".to_string()));
    assert_eq!(
        parse(r#""say \"hi\" \'x\'""#),
        Ok(r#"say "hi" 'x'"#.to_string())
    );
    assert_eq!(
        parse(r#""\${TIDEORM_TEST_X} ${TIDEORM_TEST_X}""#),
        Ok("${TIDEORM_TEST_X} x".to_string())
    );
    assert!(parse(r#""a\tb""#).unwrap_err().contains("invalid escape"));
}

#[test]
fn production_guards_hold_with_a_commented_or_capitalised_env() {
    for line in ["TIDEORM_ENV=production  # live", "TIDEORM_ENV=Production"] {
        let fixture = TempDir::new().unwrap();
        let config_path = fixture.path().join("tideorm.toml");
        fs::write(
            &config_path,
            "[project]\nenvironment = \"${TIDEORM_ENV}\"\n\n[database]\ndriver = \"sqlite\"\n",
        )
        .unwrap();
        fs::write(fixture.path().join(".env"), format!("{line}\n")).unwrap();

        let config = TideConfig::load(config_path.to_str().unwrap()).unwrap();
        assert!(config.is_production(), "{line}");
    }
}

#[test]
fn an_unresolved_url_placeholder_keeps_the_declarative_settings() {
    let fixture = TempDir::new().unwrap();
    let config_path = fixture.path().join("tideorm.toml");
    fs::write(
        &config_path,
        "[database]\ndriver = \"sqlite\"\nsqlite_path = \"app.db\"\nurl = \"${TIDEORM_TEST_UNSET_URL}\"\n",
    )
    .unwrap();

    let config = TideConfig::load(config_path.to_str().unwrap()).unwrap();
    assert_eq!(config.database.url, None);
    assert_eq!(config.database.connection_url(), "sqlite://app.db");
}

/// A URL whose password placeholder nothing sets used to be dropped, and
/// the CLI then connected with the `[database]` settings or the defaults:
/// `migrate --force` ran against another database.
#[test]
fn a_partly_unresolved_url_placeholder_is_an_error() {
    let fixture = TempDir::new().unwrap();
    let config_path = fixture.path().join("tideorm.toml");
    fs::write(
        &config_path,
        "[database]\ndriver = \"postgres\"\nurl = \"postgres://app:${TIDEORM_TEST_UNSET_PASSWORD}@db.internal/app\"\n",
    )
    .unwrap();

    let error = TideConfig::load(config_path.to_str().unwrap()).unwrap_err();
    assert!(error.contains("TIDEORM_TEST_UNSET_PASSWORD"), "{error}");
}

#[test]
fn env_file_values_expand_variables_defined_above_them() {
    let fixture = TempDir::new().unwrap();
    let config_path = fixture.path().join("tideorm.toml");
    fs::write(
        &config_path,
        "[database]\ndriver = \"postgres\"\nurl = \"${TIDEORM_TEST_URL}\"\n",
    )
    .unwrap();
    fs::write(
        fixture.path().join(".env"),
        "TIDEORM_TEST_HOST=db.internal\nTIDEORM_TEST_URL=postgres://app@${TIDEORM_TEST_HOST}:5433/app_db\n",
    )
    .unwrap();

    let config = TideConfig::load(config_path.to_str().unwrap()).unwrap();
    assert_eq!(config.database.host, "db.internal");
    assert_eq!(config.database.port, Some(5433));
}
