# Changelog

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