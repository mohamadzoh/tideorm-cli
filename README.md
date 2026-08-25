# TideORM CLI

A comprehensive command-line interface for TideORM - A powerful Rust ORM.

## Installation

Install globally:

```bash
cargo install tideorm-cli
```

## Quick Start

```bash
# Initialize a new TideORM project
tideorm init my_project

# Generate a model with fields, relations, and more
tideorm make model User \
  --fields="name:string,email:string:unique,age:i32:nullable" \
  --relations="posts:has_many:Post,company:belongs_to:Company" \
  --timestamps --soft-deletes --tokenize --migration

# Run migrations (`tideorm migrate` on its own does the same thing)
tideorm migrate run

# Seed the database
tideorm db seed
```

## Configuration

TideORM CLI uses a `tideorm.toml` configuration file:

```toml
[project]
name = "my-tideorm-project"
environment = "development"

[database]
driver = "postgres"
host = "localhost"
port = 5432
database = "myapp"
username = "postgres"
password = "password"
# Or use a connection URL:
# url = "postgres://postgres:password@localhost/myapp"

[paths]
models = "src/models"
migrations = "src/migrations"
seeders = "src/seeders"
factories = "src/factories"
config_file = "src/config.rs"

[migration]
table = "_migrations"
timestamps = true

[seeder]
default_seeder = "DatabaseSeeder"

[model]
timestamps = true
soft_deletes = false
tokenize = false
primary_key = "id"
primary_key_type = "i64"
```

## Commands

### Migration Commands

```bash
# Run all pending migrations
tideorm migrate                   # Same as `tideorm migrate run`
tideorm migrate run

# Run migrations with options
tideorm migrate run --pretend     # Show SQL without executing (never contacts the database)
tideorm migrate run --force       # Force run in production
tideorm migrate run --step=3      # Run only 3 migrations
tideorm migrate run --path=other/migrations   # Run migrations from another directory

# Generate a new migration
tideorm migrate generate create_users_table   # alias: `gen`
tideorm migrate generate create_users_table --create=users --fields="name:string,email:string"
tideorm migrate generate add_avatar_to_users --table=users --fields="avatar_url:string:nullable"

# Migration up/down
tideorm migrate up                            # Run all pending migrations
tideorm migrate up --step=3                   # Run 3 migrations
tideorm migrate up --migration=create_users_table   # Run one specific migration
tideorm migrate up --pretend                  # Show SQL without executing
tideorm migrate up --force                    # Force run in production
tideorm migrate down                          # Rollback the most recently applied migration
tideorm migrate down --step=3                 # Rollback the last 3 applied migrations
tideorm migrate down --migration=create_users_table  # Rollback one specific migration
tideorm migrate down --pretend                # Show SQL without executing
tideorm migrate down --force                  # Force run in production

# Redo migrations
tideorm migrate redo              # Rollback and re-run last migration
tideorm migrate redo --step=3     # Redo last 3 migrations
tideorm migrate redo --pretend    # Show SQL without executing
tideorm migrate redo --force      # Force run in production

# Fresh migrations (drop all tables and re-run)
# Without --force this asks for confirmation and fails if it cannot prompt.
tideorm migrate fresh
tideorm migrate fresh --force            # Skip the confirmation prompt (required in production)
tideorm migrate fresh --seed             # Also run seeders after
tideorm migrate fresh --seed --seeder=UserSeeder   # Run one specific seeder after

# Reset migrations (rollback all)
tideorm migrate reset
tideorm migrate reset --pretend   # List the migrations that would be rolled back
tideorm migrate reset --force     # Force run in production

# Refresh migrations (reset + migrate)
tideorm migrate refresh
tideorm migrate refresh --seed    # Also run seeders after
tideorm migrate refresh --step=3  # Roll back and re-run only the last 3 migrations
tideorm migrate refresh --force   # Force run in production

# Reconcile the migration ledger without running any SQL
# MySQL and MariaDB commit DDL implicitly, so a migration whose schema change
# succeeded but whose ledger row was never written leaves later runs stuck on
# "table already exists". These repair the ledger by hand.
tideorm migrate mark --migration=create_users_table            # Record it as applied
tideorm migrate mark --migration=create_users_table --unmark   # Record it as not applied
tideorm migrate mark --migration=create_users_table --force    # Force run in production

# View migration status
tideorm migrate status
tideorm migrate history
tideorm migrate history --limit=25   # Default: 10, most recently applied first
```

Rollback order follows the order migrations were *applied*, not their version
strings, so a migration merged in from a long-lived branch is rolled back in the
order it actually ran.

### Model Generation

The `make model` command is the most powerful generator, supporting:

```bash
tideorm make model <NAME> [OPTIONS]

# Basic model
tideorm make model User

# Model with fields
tideorm make model User --fields="name:string,email:string:unique,age:i32:nullable"

# Field types:
#   string, text, i32, i64, f32, f64, bool, datetime, date, time,
#   uuid, json, jsonb, decimal, bytes,
#   int_array, bigint_array, text_array, bool_array, float_array, json_array
#   (SQL spellings are accepted as aliases: varchar, tinyint, smallint, int, integer,
#    bigint, float, double, boolean, timestamp, blob, binary, integer_array,
#    string_array, boolean_array)
# Field modifiers: nullable, unique, indexed, primary_key, auto_increment, default=value

# Model with relations
tideorm make model Post --relations="user:belongs_to:User,comments:has_many:Comment"


# Model with translatable fields
tideorm make model Article --translatable="title,description,content"

# Model with attachments
tideorm make model Product \
  --attachments-single="thumbnail,featured_image" \
  --attachments-multi="gallery,documents"

# Model with indexes
tideorm make model User --indexed="email,username" --unique="email"

# Model with nullable fields
tideorm make model Profile --nullable="bio,avatar_url,website"

# Enable special features
tideorm make model User --soft-deletes --timestamps --tokenize

# Generate with migration, seeder and factory
tideorm make model User --fields="name:string" --migration --seeder --factory
tideorm make model User --all  # Same as --migration --seeder --factory

# Write the model somewhere other than the configured [paths] models directory.
# A companion --seeder/--factory generated in the same run imports the model from
# wherever --output put it.
tideorm make model User --output=src/domain/models

# Full example
tideorm make model BlogPost \
  --table=blog_posts \
  --fields="title:string,slug:string:unique,body:text,views:i64:default=0,published_at:datetime:nullable" \
  --relations="author:belongs_to:User,comments:has_many:Comment,tags:has_many:Tag" \
  --translatable="title,body" \
  --attachments-single="featured_image" \
  --attachments-multi="gallery" \
  --indexed="slug,published_at" \
  --unique="slug" \
  --soft-deletes \
  --timestamps \
  --tokenize \
  --migration \
  --seeder
```

### Other Generators

```bash
# Generate a migration
tideorm make migration create_posts_table
tideorm make migration create_posts_table --create=posts --fields="title:string,body:text"

# Generate a seeder (--count sets how many records the generated seeder creates)
tideorm make seeder UserSeeder --model=User --count=50

# Generate a factory
tideorm make factory UserFactory --model=User

# Every `make` generator accepts --output to choose the target directory
tideorm make migration create_posts_table --output=db/migrations
tideorm make seeder UserSeeder --model=User --output=db/seeders
tideorm make factory UserFactory --model=User --output=db/factories
```

`--output` defaults to the same value as the matching `[paths]` entry
(`src/migrations`, `src/seeders`, `src/factories`, `src/models`). Left alone, the
configured `[paths]` directory wins; passing anything else overrides it for that run
only. The generated file and the `mod.rs` next to it both go to the chosen directory,
so remember to declare that directory as a module in your crate.

Moving a seeder or a factory does not move the model it imports: the generated
`use crate::..` path is still derived from `[paths] models`.

### Database Commands

```bash
# Run all seeders
tideorm db seed
tideorm db seed --seeder=UserSeeder   # Run a specific seeder (alias: --class)
tideorm db seed --force               # Force run in production

# Drop all tables, re-run migrations and re-seed
tideorm db fresh
tideorm db fresh --force  # Skip the confirmation prompt (required in production)

# Show database connection status
tideorm db status

# Initialize TideORM metadata tables
tideorm db check

# Create the database
tideorm db create
tideorm db create --name=other_db   # For SQLite this is the database file path

# Drop the database
tideorm db drop
tideorm db drop --name=other_db
tideorm db drop --force  # Skip confirmation

# Wipe all tables - this DROPS every table, schema included; it is not a TRUNCATE.
# `migrate fresh` relies on those drop semantics to rebuild from the migrations.
tideorm db wipe
tideorm db wipe --force        # Skip confirmation (required in production)
tideorm db wipe --drop-types   # Also drop user-defined enum types (PostgreSQL only)

# Show table information
tideorm db table users
tideorm db tables
```

Destructive commands (`db drop`, `db wipe`, `db fresh`, `migrate fresh`) prompt for
confirmation unless `--force` is given. A run that cannot prompt - no terminal, or
`CI` / `TIDEORM_NONINTERACTIVE` set - fails with a non-zero exit rather than
reporting a cancellation as success, so pass `--force` in scripts and CI.

### Utility Commands

```bash
# Initialize a new project
tideorm init my_project
tideorm init my_project --database=mysql

# Show configuration
tideorm config

# List all models
tideorm models

# Show schema information
tideorm schema
tideorm schema --table=users
```

### Global Options

All commands support these global options:

```bash
-c, --config <FILE>    Path to tideorm.toml (default: tideorm.toml)
-v, --verbose          Enable verbose output
-h, --help             Show help
-V, --version          Show version
```

The generator commands (`tideorm make ...`, `tideorm migrate generate`, `tideorm models`)
run without a `tideorm.toml` and fall back to the built-in defaults. A `tideorm.toml` that
exists but cannot be read or parsed is always reported as an error instead - otherwise a
typo in the config would silently generate Postgres code into `src/` for a project
configured for another backend.

## Generated File Examples

### Generated Model

```rust
//! User Model
//!
//! Auto-generated by TideORM CLI

use tideorm::prelude::*;

use super::post::Post;
use super::company::Company;

#[tideorm::model(table = "users", soft_delete, tokenize)]
#[index("email")]
#[unique_index("email")]
pub struct User {
    #[tideorm(primary_key, auto_increment)]
    pub id: i64,
    pub name: String,
    pub email: String,
    #[tideorm(nullable)]
    pub age: Option<i32>,
    #[tideorm(has_many = "Post", foreign_key = "user_id")]
    pub posts: HasMany<Post>,
    #[tideorm(belongs_to = "Company", foreign_key = "company_id")]
    pub company: BelongsTo<Company>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub deleted_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl User {
    /// Find by email
    pub async fn find_by_email(email: &str) -> tideorm::Result<Option<Self>> {
        Self::query().where_eq("email", email).first().await
    }
}
```

### Generated Migration

```rust
//! Migration: create_users_table

use tideorm::prelude::*;

pub struct CreateUsersTable;

#[async_trait]
impl Migration for CreateUsersTable {
    fn version(&self) -> &str {
        "202603160001"
    }

    fn name(&self) -> &str {
        "create_users_table"
    }

    async fn up(&self, schema: &mut Schema) -> tideorm::Result<()> {
        schema.raw(r#"
        CREATE TABLE IF NOT EXISTS users (
            id BIGSERIAL PRIMARY KEY,
            name VARCHAR(255) NOT NULL,
            email VARCHAR(255) NOT NULL UNIQUE,
            age INT,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
        )
        "#).await?;
        
        Ok(())
    }

    async fn down(&self, schema: &mut Schema) -> tideorm::Result<()> {
        schema.raw(r#"DROP TABLE IF EXISTS users"#).await?;
        Ok(())
    }
}
```

### Generated Seeder

```rust
//! UserSeeder

use tideorm::prelude::*;
use crate::models::user::User;

#[derive(Default)]
pub struct UserSeeder;

#[async_trait]
impl Seed for UserSeeder {
    fn name(&self) -> &str {
        "user_seeder"
    }

    async fn run(&self, _db: &Database) -> tideorm::Result<()> {
        for _i in 1..=10 {
            User {
                // Fill in the model fields for your project.
                ..Default::default()
            }
            .save()
            .await?;
        }
        Ok(())
    }
}
```

## Environment Variables

The CLI supports environment variable expansion in `tideorm.toml`:

```toml
[database]
password = "${DATABASE_PASSWORD}"
```

Create a `.env` file:

```env
DATABASE_PASSWORD=secret
```

## License

MIT License - See LICENSE file for details.
