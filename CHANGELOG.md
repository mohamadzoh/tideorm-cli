# Changelog

## 0.10.0

Seeding works, new projects are laid out for it, MySQL wipes actually drop tables, and migrations
run through TideORM's own builders, ledger and runner instead of the CLI's copies.

- Track TideORM `0.14.0`. It is breaking for library users (see its changelog); new projects pin
  it through `SCAFFOLD_TIDEORM_VERSION`. Publish this version only after `tideorm` 0.14.0 is on
  crates.io, or `tideorm init` scaffolds a dependency that does not resolve.
- **Generated migrations are rendered by TideORM's migration builders**, so `make migration`
  creates the table `Schema::create_table` would: the same column types (on SQLite a timestamp or
  a UUID is `TEXT`, a boolean `INTEGER` and a double `REAL`; on MySQL an `i8` is `SMALLINT`), a
  table-level `PRIMARY KEY`, and the same defaults, escaped per backend. An `auto_increment`
  field that is not the primary key is refused: SQLite rejected the `AUTOINCREMENT` it was given,
  and PostgreSQL ignored it.
- **`migrate` runs, rolls back and marks migrations through TideORM's migrator**: it takes the
  migrator's lock, so two runs at once no longer apply a migration twice, and re-reads the ledger
  under it. The migrations and seed ledgers are created in the shape TideORM creates them; a
  ledger's `name` column is no longer declared `UNIQUE` (an existing table keeps its constraint).
- **`config` masks a password given as `?password=` in a printed URL.** It now prints URLs as
  TideORM's own errors do, with the user name masked too.
- CI (`.github/workflows/ci.yml`) fetches a scaffolded project's dependencies for each driver
  before the tests: `generated_project` runs `cargo check --offline` on scaffolds, which ask for
  more than the CLI builds with, so a cache holding only the CLI's own dependencies failed it.
- `dialoguer` is built without its unused `editor` prompt, which took `tempfile` and `fastrand`
  out of the binary.
- **`tideorm db seed` runs seeders.** It used to stop at "requires an application-side seeder
  runner". It now runs the project's runner binary, named after the package
  (`cargo run --bin my-app-seed`) so that two projects installed with `cargo install` do not
  collide, with `DATABASE_URL` set to the database the CLI resolved; that binary runs every
  pending seeder in `seeders::all()`, or with `--seeder` re-runs one. `--seeder User` finds
  `UserSeeder`, which is how `make seeder User` names it. `db fresh`, `migrate fresh --seed` and
  `migrate refresh --seed` now work too, and still refuse before dropping anything when a project
  has no runner, has one that does not build, or has no seeder by the `--seeder` name. A full
  `migrate refresh --seed` clears the seed ledger, which no migration owns, so its seeders run
  again over the emptied tables.
- **New projects are a library plus two binaries.** `tideorm init` puts the modules in
  `src/lib.rs`, writes `src/main.rs` and the `src/bin/<package>-seed.rs` runner against it, and
  sets `default-run`. `src/seeders/mod.rs` holds the `all()` registry, and `tideorm make seeder`
  registers each new seeder there. Existing projects keep their layout; the README's "Seeding"
  section shows how to add the runner. A crate that has a `Cargo.toml` but no sources yet gets the
  same layout, and `default-run` naming its app, since a plain `cargo run` cannot choose between
  two binaries.
- A generated model seeder's `name()` is its own snake-cased name (`AdminSeeder` →
  `admin_seeder`), not its model's, so `--seeder AdminSeeder` finds it.
- Generated seeders no longer carry an empty `#[tokio::test]` whose `use super::*;` warned under
  `cargo test`; the generated-project test now checks every target for warnings.
- `db wipe` and `migrate fresh` run on one connection, so the foreign-key switch they turn off
  first (`SET FOREIGN_KEY_CHECKS = 0` on MySQL, `PRAGMA foreign_keys = OFF` on SQLite) holds for
  the drops after it. On a pooled connection it could land on another session, and a table that
  other rows still referenced failed to drop.
- `db check` creates the seed ledger TideORM actually writes, `_seeds` with `executed_at`,
  instead of an unused `_seeders` table.
- Removed the `[seeder] default_seeder` setting. Nothing reads it now that `db seed` runs the
  `all()` registry; existing `tideorm.toml` files that set it still load.
- **Generated MySQL migrations could not hold what the model writes.** A `uuid` field was a
  `VARCHAR(36)`, but TideORM binds a UUID as 16 raw bytes, so every insert failed with "Incorrect
  string value". Timestamps were `DATETIME`, which drops microseconds, and `text` and `bytes` were
  `TEXT` and `BLOB`, which stop at 64 KB. MySQL migrations now use `BINARY(16)`, `DATETIME(6)`
  (with `CURRENT_TIMESTAMP(6)` defaults), `TIME(6)`, `LONGTEXT`, `LONGBLOB`, `JSON` and `FLOAT`,
  a unique `text` field is a `VARCHAR(255)`, and tables are created `DEFAULT CHARSET=utf8mb4`. On
  SQLite a `decimal` is `REAL`: a whole-number value in a `DECIMAL` column is stored as an integer,
  which the model cannot read back.
- `make model` and `make migration` reject an `i8` field on PostgreSQL, whose driver cannot bind
  one; use `i16`.
- **Generated model seeders fill every field.** They saved default-valued models, so the second
  row repeated the first one's unique values and `tideorm db seed` failed. `make model --seeder`
  now writes a per-row value of each field's type (`sku: format!("sku-{}", i)`, `Uuid::new_v4()`,
  a date a day later per row, ...); optional fields are left to `Default`. The row is bound to
  `record`, where it was bound to the model's snake-cased name, which for a model like `Match` is a
  keyword the seeder did not compile with.
- **`make model --nullable/--unique/--indexed` reached the model only.** The migration made those
  columns `NOT NULL`, plain and unindexed, so the first insert that left a nullable field empty,
  the generated seeder's included, failed. They reach the migration too.
- **A generated model and its migration disagreed on columns with digits or capitals.** The
  migration names the column as the field was given (`sha256`), but TideORM derives a model's
  column by snake-casing the field name, which splits digits off and lowercases (`sha_256`), so
  every read and write of such a field failed with "no such column". `make model` now pins those
  fields with `#[tideorm(column = "..")]`.
- `make model` and `make migration` reject array field types (`text_array`, ...) unless the
  project uses PostgreSQL. On other drivers they generated a `Vec<T>` model field that could not
  compile over a `TEXT` column.
- A `driver = "postgresql"` in `tideorm.toml` is read as `postgres`, and `driver = "mariadb"` as
  `mysql`. Generators matched only `postgres` and `mysql`, so the aliases silently produced another
  backend's DDL.
- **MySQL: `db wipe`, `db fresh` and `migrate fresh` dropped nothing.** MySQL 8 labels the
  table-listing query's column `TABLE_NAME`, which the lookup did not match, so the table list
  was always empty: `db tables` showed "No tables found" and every wipe reported success without
  dropping a table. The query now aliases the column.
- The CLI runs on an 8 MiB thread stack. Windows gives the main thread 1 MiB, which a debug
  build's `db fresh` overflowed on MySQL.

## 0.9.0

Breaking: the web UI is gone and the CLI is commands-only.

- **Removed `tideorm ui` and `tideorm studio`** (TideORM Studio) along with `src/ui/`, the clap
  variants and their dispatch, and the `tiny_http` dependency. Its `/api/execute` endpoint shelled
  out to the CLI binary with caller-supplied argv, so the surface goes with it. If you scripted
  either command there is no replacement — drive the equivalent `tideorm` subcommands directly.
  This is what moves the minor version rather than the patch.
- Dropped four unused dependencies (`indicatif`, `console`, `walkdir`, dev-dep `predicates`) and
  narrowed `tokio` from `full` to the features actually used. With `tiny_http` that removes 25
  packages from the lockfile.
- Destructive-command safety: `db drop` gained the production guard every sibling already had;
  `db create`/`db drop` honour `--name` on SQLite instead of silently acting on the configured
  `sqlite_path`; `migrate fresh` validates the migration set before dropping any tables and refuses
  up front when `--seed` cannot run; `migrate up` no longer hardcodes `force = true`, and
  `up`/`down`/`redo` gained `--force`; a cancelled confirmation no longer exits `0`.
- Correctness: migration SQL runs one statement at a time and the extractor no longer truncates at
  a double-quoted identifier; rollback orders by application order rather than version string;
  `migrate mark` reconciles the ledger for backends that commit DDL implicitly; integers no longer
  decode as booleans; generators emit compilable code.
- Added `.github/workflows/ci.yml` — fmt, clippy and tests. The CLI is not standalone, so the lint
  and test jobs check out the sibling ORM and recreate the `path` override developers run with
  locally; the committed manifest keeps taking `tideorm` from the registry.
- Track TideORM `0.10.0`.
- Bump the `tideorm` version pinned into every scaffolded project's `Cargo.toml` from `0.9.19` to
  `0.10.0` (`SCAFFOLD_TIDEORM_VERSION` in `src/commands/init.rs`, the single source the template
  and its test both read).
- Scaffolded `Cargo.toml` files now pin `tideorm` with `default-features = false` alongside the
  explicit backend and `runtime-tokio` features. TideORM's defaults are `["postgres",
  "runtime-tokio"]`, so a `tideorm init --database sqlite` (or `mysql`) project previously built
  the entire PostgreSQL driver stack it never used.
- Note for anyone regenerating a project against TideORM `0.10.0`: `Encrypted<T>` is gone, several
  `ModelMeta` methods were removed, and `Error` variants now carry a structured source. See the
  TideORM `0.10.0` changelog before upgrading an existing scaffold.

## 0.8.8

- Fix migration tracking so applied migrations are recorded in the database and skipped on later runs.
- Align `_migrations` metadata handling with TideORM's runtime schema and row reads.
- Use TideORM transactions for migration apply and rollback so Postgres and MySQL writes stay on the same connection.
- Load effective database configuration from `.env`, `project.env_file`, and `DATABASE_URL` values.
- Improve `tideorm init` with interactive setup, deterministic non-interactive behavior for tests, and safer cwd restoration.
- Add `tideorm db check` to initialize TideORM metadata tables.
- Remove the `tideorm ui` / `tideorm studio` web interface (TideORM Studio). The CLI is commands-only.
- Drop the now-unused `tiny_http`, `indicatif`, `console`, and `walkdir` dependencies.