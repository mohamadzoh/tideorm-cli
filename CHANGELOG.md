# Changelog

## 0.11.0

- New projects pin TideORM `0.16.0` through `SCAFFOLD_TIDEORM_VERSION`. It is breaking for library
  users (see its changelog). Publish this version only after `tideorm` 0.16.0 is on crates.io, or
  `tideorm init` scaffolds a dependency that does not resolve.
- The CLI itself builds on TideORM `0.16.0`, and so does not build from crates.io until that
  release is out. It uses the ORM's foreign-key pragma check, URL masking and index DDL instead of
  its own copies.
- **SQLite URLs are read the way sqlx reads them, and `db drop` deletes the database in use.**
  The CLI stripped the scheme as text, so `sqlite:///var/app.db` became the relative
  `var/app.db`, `?mode=rwc` stayed in the file name, and `sqlite:app.db` was not recognised at
  all. `db drop` then reported success after deleting `sqlite_path` (or nothing) while the real
  database survived. URLs now take `sqlite://` or `sqlite:` in any case, drop the query string, are
  percent-decoded and keep an absolute path absolute (`sqlite:///C:/...` on Windows too);
  `sqlite::memory:` and `mode=memory` have no file. When a URL is set, `db create`, `db drop`,
  `db status`, `db wipe` and `config` all use its file, never `sqlite_path`. A path written into a
  URL has `%`, `?` and spaces escaped, so it opens the same file.
- **A `mariadb://` URL broke every database command and made generators write SQLite DDL.** One
  driver normaliser now serves the URL, `tideorm.toml`, `init --database` and the generators, in
  any case: `mariadb` is `mysql` and `postgresql` is `postgres`.
- **`migrate redo` and `migrate refresh --step` re-apply exactly the migrations they rolled
  back.** They ran the first pending migrations instead, which are other ones while an older
  migration is still pending. A migration with no SQL in `up()` is refused before anything is
  rolled back.
- **`db wipe` and `migrate fresh` drop views.** SQLite listed only tables and MySQL ran
  `DROP TABLE` on its views, so a view survived and `migrate fresh` failed on it after every table
  was gone. Views are dropped with `DROP VIEW` (PostgreSQL included, for one that reads no table),
  and `db tables` on MySQL lists base tables only. They also drop the sequences no table owns
  (PostgreSQL, MariaDB) and MariaDB's system-versioned tables, which survived the same way. On
  PostgreSQL they leave what an extension installed (`pg_stat_statements`, PostGIS's
  `spatial_ref_sys`), which PostgreSQL refuses to drop on its own.
- **An inline comment in `.env` disabled the production guards.** `TIDEORM_ENV=production  # live`
  was read with its comment, so `migrate reset` ran without `--force`. Unquoted values now end at
  a ` #` comment, as dotenvy reads them; `Production` counts in any case; and `${VAR}` in a value
  expands from the lines above it.
- `db create` and `db drop` build the server URL with percent-encoded credentials (a password
  holding `?` or `@` corrupted it, and showed in error messages), and IPv6 hosts are bracketed
  there, in `config` and in the URL `init` writes. `config` and connection errors mask a password
  holding `?`, `#` or `/` whole; TideORM 0.15.0's masking cut the URL at the `?` and printed the
  rest of it.
- **A unique field added to a SQLite table migrates.** SQLite refuses `ADD COLUMN .. UNIQUE`, so
  the alter migration adds the column and a unique index, `idx_<table>_<column>_unique` as TideORM
  names it, and its `down()` drops the index before the column.
- **Generated models build without warnings.** A model with no unique field imported the prelude
  and used none of it, which failed a `-D warnings` build; the import is kept for the code added
  later, without the warning. Models are also laid out as rustfmt writes them, with no empty
  `impl` block.
- **`make migration` and `make model --migration` follow `[model] primary_key_type`.** A `uuid`
  model got a `BIGSERIAL` key; the key now has the configured type, auto-incremented only for
  `i32` (`INTEGER`) and `i64` (`BIGINT`). The model generator drops `auto_increment` from a
  non-integer key, which TideORM refuses to compile.
- **`indexed` fields get their index.** The 0.10.0 notes said `--indexed` reached the migration,
  but only the model's `#[index]` did. `:indexed` fields, the `--indexed` list and `belongs_to`
  foreign keys now get a `CREATE INDEX` named as the model's (`idx_<table>_<column>`); an alter
  migration's `down()` drops the index before the column, which SQLite requires. The statements
  are `schema.raw(..)` calls, the form `tideorm migrate` runs.
- The factory of a keyword-named model (`Match`, `Box`, ...) compiles: its record is bound to
  `record`, as the seeder's already was.
- `tideorm init` escapes its answers in `tideorm.toml`, where an interactive Windows path left the
  file unparsable, and names a project like `2048-game` `app_2048_game`, since Cargo refuses a
  package name that starts with a digit (or is a keyword).
- **New projects load their `.env`.** The scaffolded `src/config.rs` read only the process
  environment, so `cargo run` panicked without `DATABASE_URL` (or, on SQLite, opened
  `database.db` whatever `.env` said). It now loads the configured env file with `dotenvy`, which
  scaffolds depend on; a variable already set still wins.
- An unresolved `${DATABASE_URL}` (a fresh clone without its `.env`) no longer replaces the
  declarative `[database]` settings with its literal text.
- **A schema-qualified table (`--create app.users`, `make model --table app.users`) is one
  table everywhere.** The `CREATE TABLE` and `ALTER TABLE` statements named a table called
  `app.users` while `down()` and the indexes named `users` in schema `app`, which `down()` could
  drop instead. Every statement now names `"app"."users"`, indexes are named after `users` and
  dropped with their schema, and the model gets `table = "users", schema = "app"`.
- **A database URL with part of it unresolved is an error.** `url = "postgres://app:${DB_PASSWORD}@db/app"`
  with `DB_PASSWORD` unset was dropped, and the CLI connected with the `[database]` settings or
  the defaults instead: `migrate --force` or `db wipe --force` ran against another database. Only
  a URL that is one placeholder and nothing else still falls back to the `[database]` settings.
- **`migrate reset`, `rollback --step`, `redo` and `refresh` check every migration first.** One
  with nothing in `down()`, or whose file is gone, was reached only after the newer ones were
  rolled back, which were then left so; `refresh` also checks every `up()` before it resets.
- On MySQL and MariaDB an `indexed` `text` field is a `VARCHAR(255)`, as a `unique` one is: MySQL
  cannot index a `TEXT` column, so its migration always failed.
- A numbered `i16` key gets a `SMALLSERIAL` (PostgreSQL) or `SMALLINT AUTO_INCREMENT` (MySQL)
  column, where it got a `BIGINT` the model could not read; `auto_increment` on a key that is not
  an integer is refused instead of becoming a `BIGSERIAL`.
- **With `[migration] timestamps = false`, migrations run in the order they were made.** The
  version was the migration's name, so `add_age_to_users` ran before `create_users_table`. New
  migrations are numbered instead: one past the highest numbered migration, `0001_..` onwards.
- **A SQLite migration turning foreign keys off rebuilds a table safely.** Inside the migration's
  transaction `PRAGMA foreign_keys = OFF` did nothing, so a rebuild's `DROP TABLE` deleted the rows
  pointing at the table. Such a migration now runs on a connection that turns foreign keys off
  before its transaction begins, and is rolled back if a foreign key then points at no row. Every
  form SQLite reads is recognised, inside a multi-statement script too: `PRAGMA main.foreign_keys`,
  `foreign_keys(0)`, quoted names, comments, and any value SQLite takes for false. A migration
  returning `true` from `foreign_keys_off()`, as TideORM's `Migrator` reads it, runs the same way
  without the pragma; one whose `foreign_keys_off()` returns anything but a literal `true` or
  `false` is refused. A database that already has a foreign key pointing at no row is refused
  before such a migration runs, instead of the migration being blamed for it.
- CI runs the `db wipe` tests against PostgreSQL and MariaDB servers
  (`TIDEORM_CLI_POSTGRES_URL`, `TIDEORM_CLI_MARIADB_URL`); without them those tests are skipped.
- **`migrate` refuses a migration it cannot run whole, instead of running part of it.** It reads
  `up()` and `down()` from the source and cut each at the first line starting with `}`: the end
  of a loop, a closure or a line inside multi-line SQL, so the statements after it never ran, and
  builder calls (`schema.create_table(..)`) were skipped, yet the migration was recorded as
  applied. The body is now parsed as Rust (with `syn`), and one holding anything but
  `schema.raw(..)` statements fails with the code it cannot run. `schema.raw("..")` with a plain
  string literal runs too. A migration file that is not valid Rust is refused with the parser's
  error instead of being scanned for statements.
- `tideorm init` reads an existing `DATABASE_URL` as dotenvy does, quotes and inline comments
  included, so an unchanged quoted URL is no longer taken for a different one.
- A SQLite URL holding `mode=ro` anywhere no longer makes the CLI create the database file: sqlx
  opens such a URL read-only whatever `mode` follows, so the empty file was all it got.
- A related model named `_` is refused, as `Self` already was.

## 0.10.1

- Track TideORM `0.15.0`. It is breaking for library users (see its changelog); new projects pin
  it through `SCAFFOLD_TIDEORM_VERSION`. Publish this version only after `tideorm` 0.15.0 is on
  crates.io, or `tideorm init` scaffolds a dependency that does not resolve.
- `migrate` on SQLite now holds TideORM's migration lock, a file beside the database, so two runs
  at once against one database apply each migration once, as they already did on PostgreSQL and
  MySQL.

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