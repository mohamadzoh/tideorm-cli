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
//! # Run migrations
//! tideorm migrate
//!
//! # Generate a model
//! tideorm make:model User --fields="name:string,email:string:unique"
//!
//! # Run seeders
//! tideorm db:seed
//! ```

mod commands;
mod config;
mod generators;
mod utils;

use clap::{Parser, Subcommand};
use colored::Colorize;

/// TideORM CLI - A powerful command-line interface for TideORM
#[derive(Parser)]
#[command(name = "tideorm")]
#[command(author = "TideORM Contributors")]
#[command(version = "0.1.0")]
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
    /// Migration commands - run with subcommands or directly to execute pending migrations
    #[command(subcommand)]
    Migrate(MigrateCommands),

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

    // =========================================================================
    // WEB UI
    // =========================================================================
    /// Launch TideORM Studio - Web-based UI for TideORM
    #[command(name = "ui", alias = "studio")]
    Ui {
        /// Host address to bind to
        #[arg(short = 'H', long, default_value = "127.0.0.1")]
        host: String,

        /// Port to run the server on
        #[arg(short, long, default_value = "8080")]
        port: u16,
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

    /// Run migration up
    Up {
        /// Number of migrations to run
        #[arg(long)]
        step: Option<u32>,

        /// Specific migration to run
        #[arg(long)]
        migration: Option<String>,

        /// Pretend mode
        #[arg(long)]
        pretend: bool,
    },

    /// Run migration down (rollback)
    Down {
        /// Number of migrations to rollback
        #[arg(long, default_value = "1")]
        step: u32,

        /// Specific migration to rollback
        #[arg(long)]
        migration: Option<String>,

        /// Pretend mode
        #[arg(long)]
        pretend: bool,
    },

    /// Redo last migration (down then up)
    Redo {
        /// Number of migrations to redo
        #[arg(long, default_value = "1")]
        step: u32,

        /// Pretend mode
        #[arg(long)]
        pretend: bool,
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
        /// Types: string, text, i32, i64, f32, f64, bool, datetime, date, time, uuid, json, decimal
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

        /// Enable soft deletes
        #[arg(long, alias = "soft-delete")]
        soft_deletes: bool,

        /// Enable timestamps (created_at, updated_at) - enabled by default, use --no-timestamps to disable
        #[arg(long, default_value = "true", action = clap::ArgAction::Set)]
        timestamps: bool,

        /// Enable tokenization
        #[arg(long)]
        tokenize: bool,

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

    /// Wipe all tables (truncate)
    Wipe {
        /// Also drop all types
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
        Commands::Migrate(cmd) => {
            commands::migrate::handle_subcommand(&cli.config, cmd, cli.verbose).await
        }
        Commands::Make(cmd) => {
            commands::make::handle(&cli.config, cmd, cli.verbose).await
        }
        Commands::Db(cmd) => {
            commands::db::handle(&cli.config, cmd, cli.verbose).await
        }
        Commands::Init { name, database } => {
            commands::init::run(&name, &database, cli.verbose).await
        }
        Commands::Config => {
            commands::config::show(&cli.config, cli.verbose).await
        }
        Commands::Models => {
            commands::models::list(&cli.config, cli.verbose).await
        }
        Commands::Schema { table } => {
            commands::schema::show(&cli.config, table, cli.verbose).await
        }
        Commands::Ui { host, port } => {
            commands::ui::run(&host, port, cli.verbose).await
        }
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
