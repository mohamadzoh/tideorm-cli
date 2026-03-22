# Changelog

## 0.8.8

- Fix migration tracking so applied migrations are recorded in the database and skipped on later runs.
- Align `_migrations` metadata handling with TideORM's runtime schema and row reads.
- Use TideORM transactions for migration apply and rollback so Postgres and MySQL writes stay on the same connection.
- Load effective database configuration from `.env`, `project.env_file`, and `DATABASE_URL` values.
- Improve `tideorm init` with interactive setup, deterministic non-interactive behavior for tests, and safer cwd restoration.
- Add `tideorm db check` to initialize TideORM metadata tables.