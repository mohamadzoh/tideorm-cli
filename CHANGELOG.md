# Changelog

## Unreleased

- Track TideORM `0.10.0`. That release is breaking for library users; the CLI's own commands and
  flags are unchanged, so the CLI version stays `0.8.8`.
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