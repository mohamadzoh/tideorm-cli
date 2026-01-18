# TideORM CLI

A comprehensive command-line interface for TideORM - A powerful Rust ORM.

## Installation
install globally:

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

# Run migrations
tideorm migrate run

# Seed the database
tideorm db seed
```

## Configuration

TideORM CLI uses a `tideorm.toml` configuration file:

```toml
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

[migrations]
table = "migrations"

[seeders]
default = "DatabaseSeeder"

[model_generation]
timestamps = true
soft_deletes = false
tokenize = false
```

## Commands

### Migration Commands

```bash
# Run all pending migrations
tideorm migrate run

# Run migrations with options
tideorm migrate run --pretend     # Show SQL without executing
tideorm migrate run --force       # Force run in production
tideorm migrate run --step=3      # Run only 3 migrations

# Generate a new migration
tideorm migrate generate create_users_table
tideorm migrate generate create_users_table --create=users --fields="name:string,email:string"
tideorm migrate generate add_avatar_to_users --table=users --fields="avatar_url:string:nullable"

# Migration up/down
tideorm migrate up                # Run next pending migration
tideorm migrate up --step=3       # Run 3 migrations
tideorm migrate down              # Rollback last migration
tideorm migrate down --step=3     # Rollback 3 migrations

# Redo migrations
tideorm migrate redo              # Rollback and re-run last migration
tideorm migrate redo --step=3     # Redo last 3 migrations

# Fresh migrations (drop all tables and re-run)
tideorm migrate fresh
tideorm migrate fresh --seed      # Also run seeders after

# Reset migrations (rollback all)
tideorm migrate reset

# Refresh migrations (reset + migrate)
tideorm migrate refresh
tideorm migrate refresh --seed    # Also run seeders after

# View migration status
tideorm migrate status
tideorm migrate history
```

### Model Generation

The `make model` command is the most powerful generator, supporting:

```bash
tideorm make model <NAME> [OPTIONS]

# Basic model
tideorm make model User

# Model with fields
tideorm make model User --fields="name:string,email:string:unique,age:i32:nullable"

# Field types: string, text, i32, i64, f32, f64, bool, datetime, date, time, uuid, json, decimal
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

# Generate with migration and seeder
tideorm make model User --fields="name:string" --migration --seeder
tideorm make model User --all  # Same as --migration --seeder

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

# Generate a seeder
tideorm make seeder UserSeeder --model=User --count=50

# Generate a factory
tideorm make factory UserFactory --model=User

# Generate a controller/handler
tideorm make controller UserController --model=User --resource
```

### Database Commands

```bash
# Run all seeders
tideorm db seed

# Run a specific seeder
tideorm db seed --seeder=UserSeeder

# Drop all tables and re-seed
tideorm db fresh

# Show database connection status
tideorm db status

# Create the database
tideorm db create

# Drop the database
tideorm db drop
tideorm db drop --force  # Skip confirmation

# Wipe all tables (truncate)
tideorm db wipe

# Show table information
tideorm db table users
tideorm db tables
```

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

## Generated File Examples

### Generated Model

```rust
//! User Model

use tideorm::prelude::*;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::post::Post;
use super::company::Company;

#[derive(Debug, Clone, Serialize, Deserialize, Model)]
#[tide(table = "users", soft_delete, tokenize)]
pub struct User {
    #[tide(primary_key, auto_increment)]
    pub id: i64,
    pub name: String,
    #[tide(unique)]
    pub email: String,
    pub age: Option<i32>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl User {
    /// Get all related posts
    pub async fn posts(&self) -> tideorm::Result<Vec<Post>> {
        Post::where_eq("user_id", self.id).get().await
    }

    /// Get the related Company
    pub async fn company(&self) -> tideorm::Result<Option<Company>> {
        Company::find(self.company_id).await
    }

    /// Find by email
    pub async fn find_by_email(email: &String) -> tideorm::Result<Option<Self>> {
        Self::where_eq("email", email).first().await
    }
}
```

### Generated Migration

```rust
//! Migration: create_users_table

use tideorm::migration::{Migration, MigrationContext};

pub struct CreateUsersTable;

#[async_trait::async_trait]
impl Migration for CreateUsersTable {
    fn name(&self) -> &'static str {
        "create_users_table"
    }

    async fn up(&self, ctx: &MigrationContext) -> tideorm::Result<()> {
        ctx.execute(r#"
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

    async fn down(&self, ctx: &MigrationContext) -> tideorm::Result<()> {
        ctx.execute(r#"DROP TABLE IF EXISTS users"#).await?;
        Ok(())
    }
}
```

### Generated Seeder

```rust
//! UserSeeder

use tideorm::prelude::*;
use crate::models::User;

pub struct UserSeeder;

impl UserSeeder {
    pub async fn run() -> tideorm::Result<()> {
        for i in 1..=10 {
            let user = User {
                id: 0,
                name: format!("User {}", i),
                email: format!("user{}@example.com", i),
                ..Default::default()
            };
            user.save().await?;
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
