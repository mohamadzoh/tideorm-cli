//! TideORM CLI - Command-line interface for TideORM
//!
//! A comprehensive CLI tool for managing TideORM projects including:
//! - Database migrations
//! - Seeders
//! - Model generation
//! - Database utilities
//!
//! # Usage
//!
//! ```bash
//! # Run pending migrations
//! tideorm migrate
//!
//! # Generate a model
//! tideorm make model User --fields="name:string,email:string:unique"
//!
//! # Run seeders
//! tideorm db seed
//! ```

// `db::handle` awaits a chain of TideORM calls in one async fn, and rustc computes the layout of
// the whole generator as a single type. SeaORM 2.x (via sqlx 0.9) nests deeply enough that this
// crosses the default limit of 128 — `query depth increased by 130` — and the build fails before
// any of our own code is at fault. Raising the limit is the documented fix; it costs compile-time
// budget only, and nothing here recurses at runtime.
#![recursion_limit = "256"]

mod commands;
mod config;
mod generators;
mod runtime_db;
mod utils;

use clap::{Parser, Subcommand};
use colored::Colorize;

/// TideORM CLI - A powerful command-line interface for TideORM
#[derive(Parser)]
#[command(name = "tideorm")]
#[command(author = "TideORM Contributors")]
#[command(version)]
#[command(about = "Command-line interface for TideORM - A powerful Rust ORM", long_about = None)]
#[command(propagate_version = true)]
struct Cli {
    /// Path to the TideORM configuration file
    #[arg(short, long, global = true, default_value = "tideorm.toml")]
    config: String,

    /// Enable verbose output
    #[arg(short, long, global = true)]
    verbose: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    // =========================================================================
    // MIGRATION COMMANDS
    // =========================================================================
    /// Migration commands - run with a subcommand, or on its own to execute pending migrations
    Migrate {
        #[command(subcommand)]
        command: Option<MigrateCommands>,
    },

    // =========================================================================
    // MAKE COMMANDS (Generators)
    // =========================================================================
    /// Subcommands for generating files
    #[command(subcommand, name = "make")]
    Make(MakeCommands),

    // =========================================================================
    // DATABASE COMMANDS
    // =========================================================================
    /// Subcommands for database operations
    #[command(subcommand, name = "db")]
    Db(DbCommands),

    // =========================================================================
    // UTILITY COMMANDS
    // =========================================================================
    /// Initialize a new TideORM project
    Init {
        /// Project name
        #[arg(default_value = ".")]
        name: String,

        /// Database type (postgres, mysql, sqlite)
        #[arg(short, long, default_value = "postgres")]
        database: String,
    },

    /// Show TideORM configuration
    Config,

    /// List all models in the project
    Models,

    /// Show schema information
    Schema {
        /// Table name to show schema for
        #[arg(short, long)]
        table: Option<String>,
    },
}

#[derive(Subcommand)]
enum MigrateCommands {
    /// Run all pending migrations
    Run {
        /// Run migrations in a specific directory
        #[arg(short, long)]
        path: Option<String>,

        /// Pretend mode - show SQL without executing
        #[arg(long)]
        pretend: bool,

        /// Force run in production
        #[arg(long)]
        force: bool,

        /// Run a specific migration step
        #[arg(long)]
        step: Option<u32>,
    },

    /// Generate a new migration file
    #[command(name = "generate", alias = "gen")]
    Generate {
        /// Migration name (e.g., create_users_table)
        name: String,

        /// Create table migration
        #[arg(long)]
        create: Option<String>,

        /// Alter table migration
        #[arg(short = 'a', long)]
        table: Option<String>,

        /// Fields to add (format: name:type:modifiers)
        #[arg(short, long)]
        fields: Option<String>,
    },

    /// Run pending migrations - all of them unless --step or --migration narrows it down
    Up {
        /// Number of pending migrations to run (default: all of them)
        #[arg(long)]
        step: Option<u32>,

        /// Specific migration to run
        #[arg(long)]
        migration: Option<String>,

        /// Pretend mode
        #[arg(long)]
        pretend: bool,

        /// Force run in production
        #[arg(long)]
        force: bool,
    },

    /// Roll back applied migrations - only the most recent one unless --step says otherwise
    Down {
        /// Number of migrations to rollback (default: 1)
        #[arg(long, default_value = "1")]
        step: u32,

        /// Specific migration to rollback
        #[arg(long)]
        migration: Option<String>,

        /// Pretend mode
        #[arg(long)]
        pretend: bool,

        /// Force run in production
        #[arg(long)]
        force: bool,
    },

    /// Redo last migration (down then up)
    Redo {
        /// Number of migrations to redo
        #[arg(long, default_value = "1")]
        step: u32,

        /// Pretend mode
        #[arg(long)]
        pretend: bool,

        /// Force run in production
        #[arg(long)]
        force: bool,
    },

    /// Rollback all migrations and re-run
    Fresh {
        /// Also run seeders after migration
        #[arg(long)]
        seed: bool,

        /// Specific seeder to run
        #[arg(long)]
        seeder: Option<String>,

        /// Force run in production
        #[arg(long)]
        force: bool,
    },

    /// Reset all migrations (rollback all)
    Reset {
        /// Force run in production
        #[arg(long)]
        force: bool,

        /// Pretend mode
        #[arg(long)]
        pretend: bool,
    },

    /// Refresh migrations (reset + migrate)
    Refresh {
        /// Also run seeders after migration
        #[arg(long)]
        seed: bool,

        /// Number of migrations to refresh
        #[arg(long)]
        step: Option<u32>,

        /// Force run in production
        #[arg(long)]
        force: bool,
    },

    /// Record a migration as applied without running it
    ///
    /// Escape hatch for backends that commit DDL implicitly (MySQL/MariaDB): when a
    /// migration's schema change survived but its ledger row did not, this reconciles
    /// the ledger so later runs stop failing. Pass --unmark for the opposite repair.
    Mark {
        /// Migration to record (file name, version or logical name)
        #[arg(long)]
        migration: String,

        /// Remove the ledger entry instead of adding it
        #[arg(long)]
        unmark: bool,

        /// Force run in production
        #[arg(long)]
        force: bool,
    },

    /// Show migration status
    Status,

    /// Show migration history
    History {
        /// Number of migrations to show
        #[arg(short, long, default_value = "10")]
        limit: u32,
    },
}

#[derive(Subcommand)]
enum MakeCommands {
    /// Generate a new model
    #[command(name = "model")]
    Model {
        /// Model name (e.g., User, BlogPost)
        name: String,

        /// Table name (defaults to snake_case plural of model name)
        #[arg(short, long)]
        table: Option<String>,

        /// Fields (format: name:type[:modifiers...], comma-separated)
        /// Types: string, text, i32, i64, f32, f64, bool, datetime, date, time, uuid, json, jsonb, decimal, bytes, int_array, bigint_array, text_array, bool_array, float_array, json_array
        /// Modifiers: nullable, unique, indexed, primary_key, auto_increment, default=value
        /// Example: --fields="name:string,email:string:unique,age:i32:nullable"
        #[arg(short, long)]
        fields: Option<String>,

        /// Relations (format: name:type:Model[:foreign_key], comma-separated)
        /// Types: belongs_to, has_one, has_many
        /// Relations are defined as struct fields with proper TideORM types (HasOne, HasMany, BelongsTo)
        /// Example: --relations="posts:has_many:Post,company:belongs_to:Company:company_id"
        #[arg(short, long)]
        relations: Option<String>,

        /// Translatable fields (comma-separated field names)
        /// Example: --translatable="title,description,content"
        #[arg(long, alias = "trans")]
        translatable: Option<String>,

        /// Single attachment fields (comma-separated field names)
        /// Example: --attachments-single="avatar,thumbnail"
        #[arg(long, alias = "attach-single")]
        attachments_single: Option<String>,

        /// Multiple attachment fields (comma-separated field names)
        /// Example: --attachments-multi="photos,documents"
        #[arg(long, alias = "attach-multi")]
        attachments_multi: Option<String>,

        /// Indexed fields (comma-separated field names)
        /// Example: --indexed="email,username"
        #[arg(long, alias = "idx")]
        indexed: Option<String>,

        /// Unique fields (comma-separated field names)
        /// Example: --unique="email,username"
        #[arg(long, alias = "uniq")]
        unique: Option<String>,

        /// Nullable fields (comma-separated field names)
        /// Example: --nullable="bio,avatar_url"
        #[arg(long, alias = "null")]
        nullable: Option<String>,

        /// Enable soft deletes - defaults to `[model] soft_deletes` in tideorm.toml (false)
        #[arg(
            long,
            alias = "soft-delete",
            action = clap::ArgAction::Set,
            num_args = 0..=1,
            default_missing_value = "true"
        )]
        soft_deletes: Option<bool>,

        /// Enable timestamps (created_at, updated_at) - defaults to `[model] timestamps` in tideorm.toml (true); pass --timestamps=false to disable
        #[arg(
            long,
            action = clap::ArgAction::Set,
            num_args = 0..=1,
            default_missing_value = "true"
        )]
        timestamps: Option<bool>,

        /// Enable tokenization - defaults to `[model] tokenize` in tideorm.toml (false)
        #[arg(
            long,
            action = clap::ArgAction::Set,
            num_args = 0..=1,
            default_missing_value = "true"
        )]
        tokenize: Option<bool>,

        /// Output directory for the model file
        #[arg(short, long, default_value = "src/models")]
        output: String,

        /// Also generate a migration for this model
        #[arg(long)]
        migration: bool,

        /// Also generate a seeder for this model
        #[arg(long)]
        seeder: bool,

        /// Also generate a factory for this model
        #[arg(long)]
        factory: bool,

        /// Generate all (migration + seeder + factory)
        #[arg(short, long)]
        all: bool,
    },

    /// Generate a new migration
    #[command(name = "migration")]
    Migration {
        /// Migration name
        name: String,

        /// Create table migration
        #[arg(long)]
        create: Option<String>,

        /// Alter table migration  
        #[arg(short = 'a', long)]
        table: Option<String>,

        /// Fields to add
        #[arg(short, long)]
        fields: Option<String>,

        /// Output directory
        #[arg(short, long, default_value = "src/migrations")]
        output: String,
    },

    /// Generate a new seeder
    #[command(name = "seeder")]
    Seeder {
        /// Seeder name (e.g., UserSeeder)
        name: String,

        /// Model to seed
        #[arg(short, long)]
        model: Option<String>,

        /// Number of records to seed
        #[arg(short = 'n', long, default_value = "10")]
        count: u32,

        /// Output directory
        #[arg(short, long, default_value = "src/seeders")]
        output: String,
    },

    /// Generate a new factory
    #[command(name = "factory")]
    Factory {
        /// Factory name
        name: String,

        /// Model for the factory
        #[arg(short, long)]
        model: Option<String>,

        /// Output directory
        #[arg(short, long, default_value = "src/factories")]
        output: String,
    },
}

#[derive(Subcommand)]
enum DbCommands {
    /// Run database seeders
    Seed {
        /// Specific seeder class to run
        #[arg(short, long, alias = "class")]
        seeder: Option<String>,

        /// Force run in production
        #[arg(long)]
        force: bool,
    },

    /// Drop all tables and re-seed
    Fresh {
        /// Force run in production
        #[arg(long)]
        force: bool,
    },

    /// Show database connection status
    Status,

    /// Initialize TideORM metadata tables in the current database
    Check,

    /// Create the database
    Create {
        /// Database name
        #[arg(short, long)]
        name: Option<String>,
    },

    /// Drop the database
    Drop {
        /// Database name
        #[arg(short, long)]
        name: Option<String>,

        /// Force drop without confirmation
        #[arg(long)]
        force: bool,
    },

    /// Drop every table in the database (schema included - this is not a truncate)
    Wipe {
        /// Also drop user-defined enum types (PostgreSQL only; ignored elsewhere)
        #[arg(long)]
        drop_types: bool,

        /// Force run in production
        #[arg(long)]
        force: bool,
    },

    /// Show table information
    Table {
        /// Table name
        name: String,
    },

    /// List all tables
    Tables,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    // Print banner
    if cli.verbose {
        print_banner();
    }

    // Execute command
    let result = match cli.command {
        // `tideorm migrate` with no subcommand runs pending migrations, which is
        // what both the crate documentation and the command help advertise.
        Commands::Migrate { command } => match command {
            Some(cmd) => commands::migrate::handle_subcommand(&cli.config, cmd, cli.verbose).await,
            None => commands::migrate::run(&cli.config, None, false, false, None).await,
        },
        Commands::Make(cmd) => commands::make::handle(&cli.config, cmd, cli.verbose).await,
        Commands::Db(cmd) => commands::db::handle(&cli.config, cmd, cli.verbose).await,
        Commands::Init { name, database } => {
            commands::init::run(&name, &database, cli.verbose).await
        }
        Commands::Config => commands::config::show(&cli.config, cli.verbose).await,
        Commands::Models => commands::models::list(&cli.config, cli.verbose).await,
        Commands::Schema { table } => commands::schema::show(&cli.config, table, cli.verbose).await,
    };

    // Handle result
    match result {
        Ok(()) => {
            if cli.verbose {
                println!("\n{}", "✓ Command completed successfully".green());
            }
        }
        Err(e) => {
            eprintln!("{} {}", "Error:".red().bold(), e);
            std::process::exit(1);
        }
    }
}

fn print_banner() {
    println!(
        "{}",
        r#"
╔════════════════════════════════════════════════════════════════╗
║                                                                ║
║   ████████╗██╗██████╗ ███████╗ ██████╗ ██████╗ ███╗   ███╗    ║
║   ╚══██╔══╝██║██╔══██╗██╔════╝██╔═══██╗██╔══██╗████╗ ████║    ║
║      ██║   ██║██║  ██║█████╗  ██║   ██║██████╔╝██╔████╔██║    ║
║      ██║   ██║██║  ██║██╔══╝  ██║   ██║██╔══██╗██║╚██╔╝██║    ║
║      ██║   ██║██████╔╝███████╗╚██████╔╝██║  ██║██║ ╚═╝ ██║    ║
║      ╚═╝   ╚═╝╚═════╝ ╚══════╝ ╚═════╝ ╚═╝  ╚═╝╚═╝     ╚═╝    ║
║                                                                ║
║                    Command Line Interface                      ║
╚════════════════════════════════════════════════════════════════╝
"#
        .cyan()
    );
}

#[cfg(test)]
mod tests {
    use super::{Cli, Commands, MakeCommands};
    use clap::Parser;

    fn model_flags(args: &[&str]) -> (Option<bool>, Option<bool>, Option<bool>) {
        let cli = Cli::try_parse_from(args).expect("make model should parse");

        match cli.command {
            Commands::Make(MakeCommands::Model {
                timestamps,
                soft_deletes,
                tokenize,
                ..
            }) => (timestamps, soft_deletes, tokenize),
            _ => panic!("expected a `make model` command"),
        }
    }

    /// The `[model]` config can only win when an unsupplied flag is distinguishable from
    /// one that was passed explicitly, which is what `Option<bool>` buys.
    #[test]
    fn model_flags_are_absent_until_supplied() {
        assert_eq!(
            model_flags(&["tideorm", "make", "model", "User"]),
            (None, None, None)
        );
    }

    #[test]
    fn model_flags_record_explicit_values() {
        assert_eq!(
            model_flags(&["tideorm", "make", "model", "User", "--timestamps=true"]),
            (Some(true), None, None)
        );
        assert_eq!(
            model_flags(&["tideorm", "make", "model", "User", "--timestamps=false"]),
            (Some(false), None, None)
        );
        assert_eq!(
            model_flags(&["tideorm", "make", "model", "User", "--timestamps"]),
            (Some(true), None, None)
        );
        assert_eq!(
            model_flags(&["tideorm", "make", "model", "User", "--soft-deletes"]),
            (None, Some(true), None)
        );
        assert_eq!(
            model_flags(&["tideorm", "make", "model", "User", "--tokenize"]),
            (None, None, Some(true))
        );
    }
}
