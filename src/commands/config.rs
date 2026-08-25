//! Config command for TideORM CLI

use crate::config::TideConfig;
use crate::utils::print_info;
use colored::Colorize;

/// Show TideORM configuration
pub async fn show(config_path: &str, verbose: bool) -> Result<(), String> {
    if verbose {
        print_info(&format!("Reading configuration from: {}", config_path));
    }

    let config = TideConfig::load(config_path)?;

    println!("\n{}", "TideORM Configuration:".cyan().bold());
    println!("{}", "═".repeat(60));

    // Project
    println!("\n{}", "[project]".yellow());
    println!("  name = \"{}\"", config.project.name);
    println!("  environment = \"{}\"", config.project.environment);
    println!("  env_file = \"{}\"", config.project.env_file);

    // Database
    println!("\n{}", "[database]".yellow());
    println!("  driver = \"{}\"", config.database.driver);

    match config.database.driver.as_str() {
        "sqlite" => {
            println!(
                "  sqlite_path = \"{}\"",
                config
                    .database
                    .sqlite_path
                    .as_deref()
                    .unwrap_or("database.db")
            );
        }
        _ => {
            println!("  host = \"{}\"", config.database.host);
            if let Some(port) = config.database.port {
                println!("  port = {}", port);
            }
            if let Some(db) = &config.database.database {
                println!("  database = \"{}\"", db);
            }
            if let Some(user) = &config.database.username {
                println!("  username = \"{}\"", user);
            }
            println!("  password = \"********\"");
        }
    }

    if let Some(url) = &config.database.url {
        println!("  url = \"{}\"", mask_password(url));
    }

    println!("  pool_size = {}", config.database.pool_size);
    println!("  timeout = {}", config.database.timeout);

    // Paths
    println!("\n{}", "[paths]".yellow());
    println!("  models = \"{}\"", config.paths.models);
    println!("  migrations = \"{}\"", config.paths.migrations);
    println!("  seeders = \"{}\"", config.paths.seeders);
    println!("  factories = \"{}\"", config.paths.factories);
    println!("  config_file = \"{}\"", config.paths.config_file);

    // Migration
    println!("\n{}", "[migration]".yellow());
    println!("  table = \"{}\"", config.migration.table);
    println!("  timestamps = {}", config.migration.timestamps);

    // Seeder
    println!("\n{}", "[seeder]".yellow());
    println!("  default_seeder = \"{}\"", config.seeder.default_seeder);

    // Model
    println!("\n{}", "[model]".yellow());
    println!("  timestamps = {}", config.model.timestamps);
    println!("  soft_deletes = {}", config.model.soft_deletes);
    println!("  tokenize = {}", config.model.tokenize);
    println!("  primary_key = \"{}\"", config.model.primary_key);
    println!("  primary_key_type = \"{}\"", config.model.primary_key_type);

    println!("\n{}", "═".repeat(60));

    // Show connection URL
    println!("\n{}", "Connection URL:".cyan());
    match config.database.try_connection_url() {
        Ok(url) => println!("  {}", mask_password(&url)),
        Err(error) => println!("  {}", error.yellow()),
    }

    Ok(())
}

/// Mask the password in a connection URL.
///
/// The password ends at the *last* `@` of the authority, not the first: a non-greedy match
/// stops at an `@` inside the password itself and prints the rest of it verbatim. A
/// password holding an unencoded `/` pushes that `@` past the authority, so the whole URL
/// is scanned as a fallback rather than leaving the credential on screen.
fn mask_password(url: &str) -> String {
    let Some(scheme_end) = url.find("://") else {
        return url.to_string();
    };

    let userinfo_start = scheme_end + 3;
    let rest = &url[userinfo_start..];
    let authority_end = rest.find('/').unwrap_or(rest.len());

    let at = rest[..authority_end].rfind('@').or_else(|| rest.rfind('@'));
    let Some(at) = at else {
        return url.to_string();
    };

    let Some(colon) = rest[..at].find(':') else {
        return url.to_string();
    };

    let head = &url[..userinfo_start + colon];
    let tail = &rest[at..];

    format!("{}:********{}", head, tail)
}

#[cfg(test)]
mod tests {
    use super::mask_password;

    #[test]
    fn masks_a_plain_password() {
        assert_eq!(
            mask_password("postgres://user:pass@localhost:5432/db"),
            "postgres://user:********@localhost:5432/db"
        );
    }

    #[test]
    fn masks_a_password_containing_an_at_sign() {
        assert_eq!(
            mask_password("postgres://user:p@ss@localhost:5432/db"),
            "postgres://user:********@localhost:5432/db"
        );
    }

    #[test]
    fn masks_a_password_containing_an_unencoded_slash() {
        assert_eq!(
            mask_password("postgres://user:pa/ss@localhost/db"),
            "postgres://user:********@localhost/db"
        );
    }

    #[test]
    fn leaves_urls_without_credentials_alone() {
        assert_eq!(mask_password("sqlite://app.db"), "sqlite://app.db");
        assert_eq!(
            mask_password("postgres://user@localhost:5432/db"),
            "postgres://user@localhost:5432/db"
        );
    }
}
