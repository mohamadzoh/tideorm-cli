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

[migration]
table = "_migrations"
timestamps = true  # false numbers new migrations 0001, 0002, .. instead

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
tideorm migrate run --pretend     # Show the SQL without running it (reads the ledger)
tideorm migrate run --force       # Force run in production
tideorm migrate run --step=3      # Run only 3 migrations

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
tideorm migrate refresh --step=3  # Roll back and re-run only the last 3 migrations (no --seed)
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
tideorm migrate status --check   # Exit with an error when a migration is pending (CI)
tideorm migrate history
tideorm migrate history --limit=25   # Default: 10, most recently applied first
```

Rollback order follows the order migrations were *applied*, not their version
strings, so a migration merged in from a long-lived branch is rolled back in the
order it actually ran.

Migrations are Rust code in your project, so these commands run them through the
project's migration runner; see "Migrations" below.

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
#   (array types are PostgreSQL-only; on SQLite and MySQL store lists as json)
#   (SQL spellings are accepted as aliases: varchar, tinyint, smallint, int, integer,
#    bigint, float, double, boolean, timestamp, blob, binary, integer_array,
#    string_array, boolean_array)
# Field modifiers: nullable, unique, indexed, primary_key, auto_increment, default=value
# A `,` or `:` inside parentheses or a quoted SQL default stays in the field:
#   --fields="price:DECIMAL(10, 2),opens:time:default='09:00:00'"

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

When `--output` is omitted, each generator uses its matching `[paths]` directory
(`migrations`, `seeders`, `factories`, or `models`). Any explicit `--output` value
overrides that configured path for this run, even if it matches TideORM's built-in
default. The generated file and the `mod.rs` next to it both go to the chosen directory,
so remember to declare that directory as a module in your crate.

Moving a seeder or a factory does not move the model it imports: the generated
`use crate::..` path is still derived from `[paths] models`.

### Database Commands

```bash
# Run every seeder that has not run yet (see "Seeding" below)
tideorm db seed
tideorm db seed --seeder=UserSeeder   # Run one seeder again (alias: --class)
tideorm db seed --force               # Force run in production

# Drop all tables, re-run migrations and re-seed
tideorm db fresh
tideorm db fresh --force  # Skip the confirmation prompt (required in production)

# Show database connection status, and where the connection URL came from
# (the process environment, the env file or tideorm.toml)
tideorm db status

# Create the migrations ledger, through the migration runner
tideorm db check

# Create the database
tideorm db create
tideorm db create --name=other_db   # For SQLite this is the database file path

# Drop the database
tideorm db drop
tideorm db drop --name=other_db
tideorm db drop --force  # Skip confirmation

# Wipe all tables - this DROPS every table and view, schema included; it is not a TRUNCATE.
# `migrate fresh` relies on those drop semantics to rebuild from the migrations.
tideorm db wipe
tideorm db wipe --force        # Skip confirmation (required in production)
tideorm db wipe --drop-types   # Also drop user-defined enum types (PostgreSQL only)

# Show table information
tideorm db table users
tideorm db tables
# PostgreSQL lists non-public tables as schema-qualified names
tideorm db table app.users
```

Destructive commands (`db drop`, `db wipe`, `db fresh`, `migrate fresh`) prompt for
confirmation unless `--force` is given. A run that cannot prompt - no terminal, or
`CI` / `TIDEORM_NONINTERACTIVE` set - fails with a non-zero exit rather than
reporting a cancellation as success, so pass `--force` in scripts and CI.

### Seeding

Seeders are Rust code in your project, so the CLI runs them through a small binary
the project owns, named after its package: for `my-app`, `tideorm db seed` is
`cargo run --bin my-app-seed`, with `DATABASE_URL` set to the database the CLI resolved.
Without `--seeder` it runs every seeder in `seeders::all()` that has not run yet -
TideORM records them in the `_seeds` table and orders them by `priority()` and
`depends_on()`. `--seeder=UserSeeder`, or `--seeder=User`, runs that one seeder again.
`db fresh`, `migrate fresh --seed` and `migrate refresh --seed` check for the runner,
and for the seeder `--seeder` names, before dropping or resetting anything.

`tideorm init` lays a new project out for this - the modules live in `src/lib.rs`, so
both `src/main.rs` and `src/bin/my-app-seed.rs` can use them, and `default-run` keeps
plain `cargo run` on your app - and `tideorm make seeder` registers each new seeder in
`seeders::all()`. To add seeding to an existing project:

1. Move the module declarations from `src/main.rs` into `src/lib.rs` (`pub mod config;
   pub mod models; pub mod seeders; ...`), refer to them from `src/main.rs` as
   `my_app::config`, and add `default-run = "my-app"` to `[package]`, naming your
   app's binary: the package name as written, hyphens included.
2. Give `src/seeders/mod.rs` a registry:

   ```rust
   use tideorm::prelude::*;

   pub fn all() -> Seeder {
       Seeder::new()
           .add(database_seeder::DatabaseSeeder::default())
   }
   ```

3. Create `src/bin/my-app-seed.rs`, named after your package so that two projects
   installed with `cargo install` do not collide. `--list` prints the seeders' names,
   which the CLI checks `--seeder` against before it changes anything. It connects
   with your app's `TideConfig` (`tideorm_config()` here) through `connect_only()`,
   which skips the startup work that configuration enables (sync, migrations, seeds,
   a schema file):

   ```rust
   #[tokio::main]
   async fn main() -> Result<(), Box<dyn std::error::Error>> {
       let seeders = my_app::seeders::all();
       let name = std::env::args().nth(1);
       if name.as_deref() == Some("--list") {
           for seed in seeders.names() {
               println!("{seed}");
           }
           return Ok(());
       }

       // Without the application's startup work: a `force_sync` there would drop
       // the tables before the seeders fill them.
       my_app::config::tideorm_config().connect_only().await?;
       let outcome = match name {
           Some(name) => Ok(seeders.run_seed(&name).await?),
           None => seeders.run().await,
       };
       // A failed run still lists the seeders it ran before the failure.
       let (result, outcome) = match outcome {
           Ok(result) => (result, Ok(())),
           Err(failure) => (*failure.result, Err(failure.error)),
       };

       for seed in &result.executed {
           println!("  ran      {}", seed.name);
       }
       for seed in &result.skipped {
           println!("  skipped  {} (already ran)", seed.name);
       }
       outcome?;
       Ok(())
   }
   ```

### Migrations

Migrations are Rust code too, so `tideorm migrate` runs them the way `db seed` runs
seeders: through a binary named after the package - for `my-app`, `cargo run --bin
my-app-migrate` - with `DATABASE_URL` set to the database the CLI resolved. The runner
runs the `Migrator` that `migrations::all()` returns, recording applied migrations in
the `[migration] table` ledger, and prints what each command did as tab-separated
lines the CLI reads back. Each migration commits on its own, so a command that fails
part-way still lists the migrations it applied or rolled back before the failure.
`tideorm db check` creates the ledger through it.

`tideorm make migration` (and `make model --migration`) writes each migration with
TideORM's `Schema` builder, which renders it for the database it runs on, and
registers it in `migrations::all()`. A table may be named with its schema,
`app.users`: a PostgreSQL schema or a MySQL/MariaDB database; SQLite ignores the
schema part. `--pretend` hands each migration a `Schema` that collects its statements
instead of running them, inside a transaction that is rolled back. It connects to read
the ledger (and creates the ledger if it is missing), and what a migration runs other
than through `schema` is rolled back too, except DDL on MySQL and MariaDB, which
commits as it runs.

`tideorm init` writes the runner and the registry. To add them to an existing project:

1. Lay the project out as step 1 of "Seeding" describes, with `pub mod migrations;`
   in `src/lib.rs`.
2. Give `src/migrations/mod.rs` a registry. `tideorm make migration` adds each new
   migration below `Migrator::new()`, so keep that on the line after the signature:

   ```rust
   #[path = "0001_create_users_table.rs"]
   pub mod m_0001_create_users_table;

   use tideorm::prelude::*;

   pub fn all() -> Migrator {
       Migrator::new()
           .add(m_0001_create_users_table::CreateUsersTable)
   }
   ```

3. Create `src/bin/my-app-migrate.rs`:

   ```rust
   //! Migration runner: `tideorm migrate` runs this binary.
   //!
   //! It runs `migrations::all()`, recording them in the ledger table its first
   //! argument names, and prints what a command did as tab-separated lines that
   //! `tideorm` reads back: `applied`, `pending` or `rolled_back`, then the
   //! version and the name.

   use std::process::ExitCode;
   use tideorm::prelude::*;

   #[tokio::main]
   async fn main() -> ExitCode {
       let args: Vec<String> = std::env::args().skip(1).collect();
       match run(&args).await {
           Ok(()) => ExitCode::SUCCESS,
           Err(error) => {
               eprintln!("{error}");
               ExitCode::FAILURE
           }
       }
   }

   async fn run(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
       let [table, command, names @ ..] = args else {
           return Err("usage: <ledger table> <command> [migration ..]".into());
       };
       let steps = || -> Result<usize, Box<dyn std::error::Error>> {
           Ok(names.first().ok_or("missing the number of steps")?.parse()?)
       };
       // Without the application's startup work, which would sync, migrate or
       // seed before this command runs.
       my_app::config::tideorm_config().connect_only().await?;
       let migrator = my_app::migrations::all().migrations_table(table.as_str());

       match command.as_str() {
           "run" => report(migrator.run().await)?,
           "run-steps" => report(migrator.run_steps(steps()?).await)?,
           "rollback-steps" => report(migrator.rollback_steps(steps()?).await)?,
           "reset" => report(migrator.reset().await)?,
           "refresh" => report(migrator.refresh().await)?,
           "up" => {
               for name in names {
                   report(Ok(migrator.run_one(name).await?))?;
               }
           }
           "down" => {
               for name in names {
                   report(Ok(migrator.rollback_one(name).await?))?;
               }
           }
           "mark" | "unmark" => {
               for name in names {
                   migrator.mark(name, command == "mark").await?;
               }
           }
           "pretend-up" | "pretend-down" => {
               let up = command == "pretend-up";
               for name in names {
                   println!("\n-- Migration: {name}\n-- {}:", if up { "Up" } else { "Down" });
                   for statement in migrator.pretend(name, up).await? {
                       println!("{statement};");
                   }
               }
           }
           "status" => {
               for migration in migrator.status().await? {
                   let state = if migration.applied { "applied" } else { "pending" };
                   println!("{state}\t{}\t{}", migration.version, migration.name);
               }
           }
           "history" => {
               for migration in migrator.applied().await? {
                   println!(
                       "applied\t{}\t{}\t{}",
                       migration.version, migration.name, migration.applied_at
                   );
               }
           }
           other => return Err(format!("unknown command `{other}`").into()),
       }
       Ok(())
   }

   /// Print what a command rolled back and applied, a failed one's work before
   /// its failure included, and pass the failure on.
   fn report(outcome: Result<MigrationResult, MigrationFailure>) -> tideorm::Result<()> {
       let (result, outcome) = match outcome {
           Ok(result) => (result, Ok(())),
           Err(failure) => (*failure.result, Err(failure.error)),
       };
       for migration in &result.rolled_back {
           println!("rolled_back\t{}\t{}", migration.version, migration.name);
       }
       for migration in &result.applied {
           println!("applied\t{}\t{}", migration.version, migration.name);
       }
       outcome
   }
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

# Find missing indexes, slow query patterns and common mistakes
tideorm analyze
tideorm analyze --offline   # without the database: use the indexes the migrations and models make
```

`tideorm analyze` reads every `.rs` file under the project's `src/` and, when it can
connect, the indexes and columns of the tables the models read; offline, it takes the
indexes the migrations' `Schema` calls create and the models' `#[index]` attributes declare.
Each `src/bin` program's models are its own. It reports:

| Rule | Finds |
|---|---|
| `unknown-column` (error) | a `where_*` on a column the model does not have |
| `missing-column` (error) | a model field whose column the table lacks |
| `unindexed-filter` | a query whose filters no index can serve, so it reads every row |
| `unindexed-foreign-key` | a `HasOne`/`HasMany`/`SelfRefMany`/`HasManyThrough` key no index leads |
| `n-plus-one` | a query or relation `load()` awaited inside a loop |
| `write-in-loop` | `save()`/`create()`/`delete()` awaited inside a loop, outside `seeders`/`factories` |
| `sql-injection` | raw SQL (`where_raw`, `Database::raw_json`, ..) built with `format!` or `+` |
| `rows-to-count` | `.get().await?.len()` or `.is_empty()` instead of `count()`/`exists()` |
| `nullable-mismatch` | a nullable column read into a field that is not an `Option` |
| `sensitive-field` | a password, secret or token field that is neither hidden nor encrypted |
| `missing-table` | a model whose table the database lacks |
| hints | `count() > 0` instead of `exists()`, unbounded reads, `LIKE '%..'`, unindexed `ORDER BY` and joins, indexes the primary key covers, money in floats |

The command fails when it finds an error, so CI can run it. It reads the code without
running it: a query built inside a macro (`vec![..]`, `join!(..)`) or passed between
functions as anything but a `QueryBuilder<Model>` parameter is not followed. It stays
quiet where it cannot be sure: a query a function takes or returns as a `QueryBuilder`
is checked for misspelt columns only, since its caller adds the other filters; a query
that calls a scope or a condition it cannot read (`where_raw`, `where_has`, `when(..)`)
is not called unindexed; and a loop over pages (`page()`, `paginate()`, `chunk()`), or a
`loop`/`while` that takes a `limit()`ed batch each time, is not N+1. When `tideorm.toml`
cannot be read or the database cannot be reached, it says so and runs offline.

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

`tideorm make migration create_users_table --create=users --fields="name:string,email:string:unique,age:i32:nullable"`
with timestamps on writes:

```rust
//! Migration: create_users_table
//!
//! Creates the users table.

use tideorm::prelude::*;

/// Migration: create_users_table
pub struct CreateUsersTable;

#[async_trait]
impl Migration for CreateUsersTable {
    fn version(&self) -> &str {
        "20260316203329001"
    }

    fn name(&self) -> &str {
        "create_users_table"
    }

    async fn up(&self, schema: &mut Schema) -> tideorm::Result<()> {
        schema
            .create_table_if_not_exists("users", |t| {
                t.big_increments("id");
                t.string("name").not_null();
                t.string("email").not_null().unique();
                t.integer("age").nullable();
                t.timestamps();
            })
            .await?;

        Ok(())
    }

    async fn down(&self, schema: &mut Schema) -> tideorm::Result<()> {
        schema.drop_table_if_exists("users").await?;

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
