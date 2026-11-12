//! Schema command for TideORM CLI

use crate::config::TideConfig;
use crate::runtime_db;
use crate::utils::print_info;
use colored::Colorize;
use tideorm::prelude::Database;

/// Show schema information
pub async fn show(config_path: &str, table: Option<String>, verbose: bool) -> Result<(), String> {
    let config = TideConfig::load(config_path)?;

    if verbose {
        print_info("Fetching schema information...");
    }

    let db = runtime_db::connect(&config).await?;
    if let Some(table_name) = table {
        show_table_schema(&db, &table_name).await
    } else {
        show_all_schemas(&db).await
    }
}

/// Show schema for a specific table
async fn show_table_schema(db: &Database, table_name: &str) -> Result<(), String> {
    let columns = runtime_db::table_columns(db, table_name).await?;

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
            runtime_db::column_key(col),
            col.default.as_deref().unwrap_or("NULL"),
            col.extra.as_deref().unwrap_or("")
        );
    }

    println!("{}", "─".repeat(100));

    // Show indexes
    let indexes = runtime_db::table_indexes(db, table_name).await?;
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
    let foreign_keys = runtime_db::table_foreign_keys(db, table_name).await?;
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
async fn show_all_schemas(db: &Database) -> Result<(), String> {
    let tables = runtime_db::list_tables(db).await?;

    println!("\n{}", "Database Schema:".cyan().bold());
    println!("{}", "═".repeat(80));

    if tables.is_empty() {
        println!("  No tables found");
        return Ok(());
    }

    // Read the tables' columns a few at a time; print them in the listed order.
    let permits = std::sync::Arc::new(tokio::sync::Semaphore::new(4));
    let reads: Vec<_> = tables
        .iter()
        .map(|table| {
            let (db, table, permits) = (db.clone(), table.clone(), permits.clone());
            tokio::spawn(async move {
                let _permit = permits.acquire_owned().await;
                runtime_db::table_columns(&db, &table).await
            })
        })
        .collect();

    for (table, read) in tables.iter().zip(reads) {
        let columns = read.await.map_err(|error| error.to_string())??;

        println!("\n  {} ({})", table.green().bold(), columns.len());

        for col in &columns {
            let key_marker = match runtime_db::column_key(col) {
                "PRI" => " 🔑",
                "UNI" => " ⚡",
                "FK" => " 🔗",
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
