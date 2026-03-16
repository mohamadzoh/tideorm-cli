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
    let columns = get_table_schema(config, table_name).await?;

    println!("\n{}", format!("Schema for table: {}", table_name).cyan().bold());
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
    let indexes = get_table_indexes(config, table_name).await?;
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
    let foreign_keys = get_foreign_keys(config, table_name).await?;
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
    let tables = get_all_tables(config).await?;

    println!("\n{}", "Database Schema:".cyan().bold());
    println!("{}", "═".repeat(80));

    if tables.is_empty() {
        println!("  No tables found");
        return Ok(());
    }

    for table in &tables {
        let columns = get_table_schema(config, table).await?;
        
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
                col.name,
                col.data_type,
                nullable,
                key_marker
            );
        }
    }

    println!("\n{}", "═".repeat(80));
    println!("\nLegend: 🔑 Primary Key  ⚡ Unique  🔗 Foreign Key  ? Nullable");

    Ok(())
}

// =============================================================================
// HELPER TYPES
// =============================================================================

/// Column schema information
#[derive(Debug)]
struct ColumnSchema {
    name: String,
    data_type: String,
    nullable: bool,
    key: Option<String>,
    default: Option<String>,
    extra: Option<String>,
}

/// Index information
#[derive(Debug)]
struct IndexInfo {
    name: String,
    columns: Vec<String>,
    unique: bool,
}

/// Foreign key information
#[derive(Debug)]
struct ForeignKeyInfo {
    column: String,
    references_table: String,
    references_column: String,
}

/// Get all tables from the database
async fn get_all_tables(config: &TideConfig) -> Result<Vec<String>, String> {
    runtime_db::list_tables(config).await
}

/// Get schema for a table
async fn get_table_schema(config: &TideConfig, table_name: &str) -> Result<Vec<ColumnSchema>, String> {
    runtime_db::table_columns(config, table_name)
        .await
        .map(|columns| {
            columns
                .into_iter()
                .map(|column| ColumnSchema {
                    name: column.name,
                    data_type: column.data_type,
                    nullable: column.nullable,
                    key: column.key,
                    default: column.default,
                    extra: column.extra,
                })
                .collect()
        })
}

/// Get indexes for a table
async fn get_table_indexes(config: &TideConfig, table_name: &str) -> Result<Vec<IndexInfo>, String> {
    runtime_db::table_indexes(config, table_name)
        .await
        .map(|indexes| {
            indexes
                .into_iter()
                .map(|index| IndexInfo {
                    name: index.name,
                    columns: index.columns,
                    unique: index.unique,
                })
                .collect()
        })
}

/// Get foreign keys for a table
async fn get_foreign_keys(config: &TideConfig, table_name: &str) -> Result<Vec<ForeignKeyInfo>, String> {
    runtime_db::table_foreign_keys(config, table_name)
        .await
        .map(|foreign_keys| {
            foreign_keys
                .into_iter()
                .map(|foreign_key| ForeignKeyInfo {
                    column: foreign_key.column,
                    references_table: foreign_key.references_table,
                    references_column: foreign_key.references_column,
                })
                .collect()
        })
}
