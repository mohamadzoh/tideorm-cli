//! Schema command for TideORM CLI

use crate::config::TideConfig;
use crate::runtime_db;
use crate::utils::print_info;
use colored::Colorize;

/// Show schema information
pub async fn show(config_path: &str, table: Option<String>, verbose: bool) -> Result<(), String> {
    let config = TideConfig::load(config_path)?;

    if verbose {
        print_info("Fetching schema information...");
    }

    if let Some(table_name) = table {
        show_table_schema(&config, &table_name).await
    } else {
        show_all_schemas(&config).await
    }
}

/// Show schema for a specific table
async fn show_table_schema(config: &TideConfig, table_name: &str) -> Result<(), String> {
    let columns = runtime_db::table_columns(config, table_name).await?;

    println!(
        "\n{}",
        format!("Schema for table: {}", table_name).cyan().bold()
    );
    println!("{}", "═".repeat(100));

    // Table header
    println!(
        "  {:<20} {:<20} {:<10} {:<10} {:<10} {:<20}",
        "Column", "Type", "Nullable", "Key", "Default", "Extra"
    );
    println!("{}", "─".repeat(100));

    for col in &columns {
        println!(
            "  {:<20} {:<20} {:<10} {:<10} {:<10} {:<20}",
            col.name,
            col.data_type,
            if col.nullable { "YES" } else { "NO" },
            col.key.as_deref().unwrap_or(""),
            col.default.as_deref().unwrap_or("NULL"),
            col.extra.as_deref().unwrap_or("")
        );
    }

    println!("{}", "─".repeat(100));

    // Show indexes
    let indexes = runtime_db::table_indexes(config, table_name).await?;
    if !indexes.is_empty() {
        println!("\n{}", "Indexes:".yellow());
        for idx in &indexes {
            println!(
                "  • {} ({}) - {}",
                idx.name,
                idx.columns.join(", "),
                if idx.unique { "UNIQUE" } else { "INDEX" }
            );
        }
    }

    // Show foreign keys
    let foreign_keys = runtime_db::table_foreign_keys(config, table_name).await?;
    if !foreign_keys.is_empty() {
        println!("\n{}", "Foreign Keys:".yellow());
        for fk in &foreign_keys {
            println!(
                "  • {} -> {}.{}",
                fk.column, fk.references_table, fk.references_column
            );
        }
    }

    println!("{}", "═".repeat(100));

    Ok(())
}

/// Show all table schemas
async fn show_all_schemas(config: &TideConfig) -> Result<(), String> {
    let tables = runtime_db::list_tables(config).await?;

    println!("\n{}", "Database Schema:".cyan().bold());
    println!("{}", "═".repeat(80));

    if tables.is_empty() {
        println!("  No tables found");
        return Ok(());
    }

    for table in &tables {
        let columns = runtime_db::table_columns(config, table).await?;

        println!("\n  {} ({})", table.green().bold(), columns.len());

        for col in &columns {
            let key_marker = match col.key.as_deref() {
                Some("PRI") => " 🔑",
                Some("UNI") => " ⚡",
                Some("MUL") | Some("FK") => " 🔗",
                _ => "",
            };

            let nullable = if col.nullable { "?" } else { "" };

            println!(
                "    ├─ {}: {}{}{}",
                col.name, col.data_type, nullable, key_marker
            );
        }
    }

    println!("\n{}", "═".repeat(80));
    println!("\nLegend: 🔑 Primary Key  ⚡ Unique  🔗 Foreign Key  ? Nullable");

    Ok(())
}
