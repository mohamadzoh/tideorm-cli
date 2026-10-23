//! Migration generator for TideORM CLI

use crate::config::{TideConfig, canonical_driver};
use crate::utils::{
    FieldDefinition, append_source_block, auto_increments, database_type, ensure_directory,
    ensure_writable, escape_ident, has_module_declaration, migration_timestamp,
    quote_sql_identifier, read_source_or_empty, render_template, split_table, to_snake_case,
    validate_rust_type_name, validate_sql_identifier, validate_sql_object_name,
};
use serde::Serialize;
use tideorm::config::DatabaseType;
use tideorm::migration::{AlterTableBuilder, ColumnType, DefaultValue, TableBuilder};
use tideorm::model::IndexDefinition;

/// `sql` with every line indented by `prefix`, to sit inside a generated file.
fn indent(sql: &str, prefix: &str) -> String {
    sql.lines()
        .map(|line| format!("{prefix}{line}"))
        .collect::<Vec<_>>()
        .join(
            "
",
        )
}

/// The `schema.raw(..)` line that runs one statement.
///
/// Every statement is its own raw string literal, which is what the CLI's migration
/// runner reads back. The trailing space keeps a quoted name from sitting directly
/// against the `"#` that closes the literal.
fn raw_call(sql: &str) -> String {
    format!("        schema.raw(r#\"{} \"#).await?;", sql)
}

/// The `schema.raw(..)` call running one statement already indented over
/// several lines, as the template's `raw_sql` mode writes it.
fn raw_block(indented_sql: &str) -> String {
    format!("        schema.raw(r#\"\n{indented_sql}\n        \"#).await?;")
}

/// Migration generator
pub struct MigrationGenerator<'a> {
    config: &'a TideConfig,
    output_dir: Option<String>,
}

impl<'a> MigrationGenerator<'a> {
    /// Create a new migration generator
    pub fn new(config: &'a TideConfig) -> Self {
        Self {
            config,
            output_dir: None,
        }
    }

    /// Override the directory migrations are written to (the `--output` flag).
    ///
    /// `None` keeps the configured `[paths] migrations` directory.
    pub fn output_dir(mut self, dir: Option<&str>) -> Self {
        self.output_dir = dir
            .map(str::trim)
            .filter(|dir| !dir.is_empty())
            .map(ToOwned::to_owned);
        self
    }

    /// Directory the generated migration and its `mod.rs` belong to.
    fn migrations_dir(&self) -> &str {
        self.output_dir
            .as_deref()
            .unwrap_or(&self.config.paths.migrations)
    }

    /// Generate a migration file
    pub fn generate(
        &self,
        name: &str,
        create_table: Option<String>,
        alter_table: Option<String>,
        fields: Option<String>,
        include_timestamps: bool,
        include_soft_deletes: bool,
    ) -> Result<String, String> {
        if create_table.is_some() && alter_table.is_some() {
            return Err("A migration cannot create and alter a table at the same time".to_string());
        }
        if fields.is_some() && create_table.is_none() && alter_table.is_none() {
            return Err("Migration fields require --create or --table".to_string());
        }

        let migration_name = to_snake_case(name);
        let struct_name = to_pascal_case(&migration_name);
        validate_rust_type_name("migration", &struct_name)?;

        if create_table.is_some() || alter_table.is_some() {
            ensure_ddl_driver(&self.config.database.driver)?;
        }

        let directory = self.migrations_dir();
        ensure_directory(directory)?;

        // Migrations run in version order. Without a timestamp the version is
        // the next number in the directory, so they still run in the order
        // they were made rather than by name.
        let version = if self.config.migration.timestamps {
            migration_timestamp()
        } else {
            next_migration_number(directory)
        };
        let file_name = format!("{}_{}.rs", version, migration_name);

        let file_path = format!("{}/{}", directory, file_name);

        ensure_writable(&file_path)?;
        Self::ensure_no_conflicting_migration(directory, &version, &migration_name)?;

        // Parse fields
        let parsed_fields = Self::parse_fields(fields.as_deref(), &self.config.database.driver)?;

        // Generate content
        let content = if let Some(table) = create_table {
            self.generate_create_table(
                &migration_name,
                &version,
                &table,
                &parsed_fields,
                include_timestamps,
                include_soft_deletes,
            )?
        } else if let Some(table) = alter_table {
            self.generate_alter_table(&migration_name, &version, &table, &parsed_fields)?
        } else {
            self.generate_empty(&migration_name, &version)?
        };

        std::fs::write(&file_path, content)
            .map_err(|e| format!("Failed to write migration file: {}", e))?;

        // Update mod.rs
        self.update_mod_file(&file_name)?;

        Ok(file_path)
    }

    /// Refuse to generate a migration that could never be applied.
    ///
    /// `version` is UNIQUE in the migrations ledger, so two files sharing one can be
    /// written but only one of them can ever run, and a name picks a migration out for
    /// `migrate up --migration`, which a second file of that name would make ambiguous.
    /// Timestamps carry milliseconds, which makes a version clash unlikely, but reusing a
    /// name is easy to do by accident and is worth reporting at generation time.
    fn ensure_no_conflicting_migration(
        directory: &str,
        timestamp: &str,
        migration_name: &str,
    ) -> Result<(), String> {
        let Ok(entries) = std::fs::read_dir(directory) else {
            return Ok(());
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|extension| extension != "rs") {
                continue;
            }

            let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
                continue;
            };

            if stem == "mod" {
                continue;
            }

            let existing_name = match stem.split_once('_') {
                Some((version, name))
                    if !version.is_empty()
                        && version.chars().all(|character| character.is_ascii_digit()) =>
                {
                    name
                }
                _ => stem,
            };

            if existing_name == migration_name {
                return Err(format!(
                    "Migration '{}' already exists as '{}.rs'. Migration names are unique - pick a different name.",
                    migration_name, stem
                ));
            }

            if stem.starts_with(&format!("{}_", timestamp)) {
                return Err(format!(
                    "Migration version '{}' is already used by '{}.rs'. Re-run to get a fresh one.",
                    timestamp, stem
                ));
            }
        }

        Ok(())
    }

    /// Quote a table or column name for the configured driver.
    fn quote(&self, kind: &str, name: &str) -> Result<String, String> {
        quote_sql_identifier(&self.config.database.driver, kind, name)
    }

    /// The `CREATE INDEX` of an `indexed` field, rendered by TideORM's `TableBuilder`.
    ///
    /// It is named as TideORM names a model's `#[index(..)]`, so schema sync finds the
    /// index the migration created. `if_not_exists` goes with a `CREATE TABLE IF NOT
    /// EXISTS`; MySQL has no `CREATE INDEX IF NOT EXISTS`.
    fn create_index_sql(
        &self,
        table: &str,
        column: &str,
        unique: bool,
        if_not_exists: bool,
    ) -> Result<String, String> {
        validate_sql_identifier("column", column)?;
        let (_, bare) = split_table(table);
        let mut builder = TableBuilder::new(bare, database_type(&self.config.database.driver));
        if unique {
            builder.unique_index(&[column]);
        } else {
            builder.index(&[column]);
        }
        self.qualify_table(&builder.__index_sql(if_not_exists).concat(), table)
    }

    /// The `DROP INDEX` undoing [`Self::create_index_sql`], as `Schema::drop_index`
    /// renders it. An index lives in its table's schema, so it is named with it.
    fn drop_index_sql(&self, table: &str, column: &str, unique: bool) -> Result<String, String> {
        validate_sql_object_name("table", table)?;
        let (schema, bare) = split_table(table);
        // `IF EXISTS` for a unique one, as TideORM drops the index of a unique
        // column with it.
        Ok(tideorm::migration::__drop_index_sql(
            database_type(&self.config.database.driver),
            schema,
            bare,
            &index_name(bare, column, unique),
            unique,
        ))
    }

    /// `statement`, rendered by TideORM's migration builders for the bare
    /// name of `table`, naming `table` schema-qualified. The builders quote a
    /// name whole, so they would create a table called `app.users`; the
    /// table is the first name a statement quotes.
    fn qualify_table(&self, statement: &str, table: &str) -> Result<String, String> {
        let (Some(_), bare) = split_table(table) else {
            return Ok(statement.to_string());
        };
        Ok(statement.replacen(&self.quote("table", bare)?, &self.quote("table", table)?, 1))
    }

    fn is_mysql(&self) -> bool {
        database_type(&self.config.database.driver) == DatabaseType::MySQL
    }

    /// Generate a create table migration
    fn generate_create_table(
        &self,
        name: &str,
        version: &str,
        table: &str,
        fields: &[FieldDefinition],
        include_timestamps: bool,
        include_soft_deletes: bool,
    ) -> Result<String, String> {
        let struct_name = to_pascal_case(name);
        let driver = &self.config.database.driver;
        let quoted_table = self.quote("table", table)?;

        // The table is rendered by TideORM's own migration builder, so a generated
        // migration creates the table `Schema::create_table` would, on every driver.
        let mut builder = TableBuilder::new(split_table(table).1, database_type(driver));
        if !fields
            .iter()
            .any(|field| field.primary_key || field.name == self.config.model.primary_key)
        {
            // The key the model generator declares: of the configured type, and numbered
            // by the database only when that type is an integer.
            let mut key = FieldDefinition::parse(&format!(
                "{}:{}:primary_key",
                self.config.model.primary_key, self.config.model.primary_key_type
            ))?;
            key.auto_increment = auto_increments(&key.field_type);
            self.add_column(&mut builder, &key, driver)?;
        }
        for field in fields {
            self.add_column(&mut builder, field, driver)?;
        }
        if include_timestamps {
            builder.timestamps();
        }
        if include_soft_deletes {
            builder.soft_deletes();
        }
        let raw_sql = indent(
            &self.qualify_table(&builder.__create_sql(), table)?,
            "        ",
        );
        let indexes = fields
            .iter()
            .filter(|field| field.indexed)
            .map(|field| self.create_index_sql(table, &field.name, false, true))
            .collect::<Result<Vec<_>, _>>()?;

        // A table with indexes needs a statement each, so the table is one statement
        // among them; a template that renders either mode keeps working.
        let (up_mode, up_raw_sql, up_statements) = if indexes.is_empty() {
            ("raw_sql", Some(raw_sql), Vec::new())
        } else {
            let statements = std::iter::once(raw_block(&raw_sql))
                .chain(indexes.iter().map(|sql| raw_call(sql)))
                .collect();
            ("statements", None, statements)
        };

        let context = MigrationTemplateContext {
            name: name.to_string(),
            version: version.to_string(),
            struct_name,
            description: format!("Creates the {} table.", table),
            up_mode: up_mode.to_string(),
            down_mode: "raw_sql".to_string(),
            up_raw_sql,
            down_raw_sql: Some(format!("DROP TABLE IF EXISTS {}", quoted_table)),
            up_statements,
            down_statements: Vec::new(),
        };

        self.render_migration_template(&context)
    }

    /// Generate an alter table migration
    fn generate_alter_table(
        &self,
        name: &str,
        version: &str,
        table: &str,
        fields: &[FieldDefinition],
    ) -> Result<String, String> {
        let struct_name = to_pascal_case(name);
        let driver = &self.config.database.driver;
        // Validated here; the builders quote it themselves.
        self.quote("table", table)?;

        let bare = split_table(table).1;
        // SQLite refuses `ADD COLUMN .. UNIQUE`: the column is added plain and a unique
        // index, named as TideORM names one, enforces it.
        let unique_index =
            |field: &FieldDefinition| field.unique && canonical_driver(driver) == Some("sqlite");
        let mut up = AlterTableBuilder::new(bare, database_type(driver));
        let mut down = AlterTableBuilder::new(bare, database_type(driver));
        for field in fields {
            let column = up.add_column(&field.name, field.migration_column_type(driver));
            let column = if field.nullable {
                column.nullable()
            } else {
                column.not_null()
            };
            let column = if field.unique && !unique_index(field) {
                column.unique()
            } else {
                column
            };
            if let Some(default) = &field.default {
                column.default(DefaultValue::Raw(default.clone()));
            }
            down.drop_column(&field.name);
        }
        let raw_statements = |alter: &AlterTableBuilder| -> Result<Vec<String>, String> {
            alter
                .__statements()
                .map_err(|error| error.to_string())?
                .iter()
                .map(|statement| Ok(raw_call(&self.qualify_table(statement, table)?)))
                .collect()
        };
        // The indexes come after the columns they cover, and are dropped before them:
        // SQLite refuses to drop a column an index still uses.
        let mut up_statements = raw_statements(&up)?;
        let dropped_columns = raw_statements(&down)?;
        let mut down_statements = Vec::new();
        for field in fields.iter().filter(|field| unique_index(field)) {
            let create_index = self.create_index_sql(table, &field.name, true, false)?;
            up_statements.push(raw_call(&create_index));
            // A TideORM that drops this index with the column already says so.
            let drop_index = raw_call(&self.drop_index_sql(table, &field.name, true)?);
            if !dropped_columns.contains(&drop_index) {
                down_statements.push(drop_index);
            }
        }
        for field in fields.iter().filter(|field| field.indexed) {
            let create_index = self.create_index_sql(table, &field.name, false, false)?;
            up_statements.push(raw_call(&create_index));
            down_statements.push(raw_call(&self.drop_index_sql(table, &field.name, false)?));
        }
        down_statements.extend(dropped_columns);

        let context = MigrationTemplateContext {
            name: name.to_string(),
            version: version.to_string(),
            struct_name,
            description: format!("Alters the {} table.", table),
            up_mode: "statements".to_string(),
            down_mode: "statements".to_string(),
            up_raw_sql: None,
            down_raw_sql: None,
            up_statements,
            down_statements,
        };

        self.render_migration_template(&context)
    }

    /// Generate an empty migration
    fn generate_empty(&self, name: &str, version: &str) -> Result<String, String> {
        let struct_name = to_pascal_case(name);

        let context = MigrationTemplateContext {
            name: name.to_string(),
            version: version.to_string(),
            struct_name,
            description: "TODO: Describe what this migration does.".to_string(),
            up_mode: "comments".to_string(),
            down_mode: "comments".to_string(),
            up_raw_sql: None,
            down_raw_sql: None,
            // No commented out `schema.raw(..)` example here on purpose: the migration
            // runner recovers statements by scanning this file for raw string literals, so
            // an example that looks like real SQL is one comment stripping bug away from
            // being executed. The stub describes the call instead of spelling it out.
            up_statements: vec![
                "        // TODO: Implement the forward migration.".to_string(),
                "        // Call schema.raw(..) once per SQL statement.".to_string(),
                "        let _ = schema; // Remove once implemented.".to_string(),
            ],
            down_statements: vec![
                "        // TODO: Implement the reverse migration.".to_string(),
                "        // Undo every statement that up() applies.".to_string(),
                "        let _ = schema; // Remove once implemented.".to_string(),
            ],
        };

        self.render_migration_template(&context)
    }

    fn render_migration_template(
        &self,
        context: &MigrationTemplateContext,
    ) -> Result<String, String> {
        render_template(
            "migration",
            DEFAULT_MIGRATION_TEMPLATE,
            self.config.migration.template.as_deref(),
            context,
        )
    }

    fn parse_fields(fields: Option<&str>, driver: &str) -> Result<Vec<FieldDefinition>, String> {
        let mut parsed_fields = Vec::new();

        if let Some(fields_str) = fields {
            for field in fields_str.split(',') {
                let field = field.trim();
                if field.is_empty() {
                    continue;
                }

                let parsed = FieldDefinition::parse(field)?;
                parsed.check_driver_support(driver)?;
                parsed_fields.push(parsed);
            }
        }

        Ok(parsed_fields)
    }

    /// Declare `field` on the table `builder` creates.
    fn add_column(
        &self,
        builder: &mut TableBuilder,
        field: &FieldDefinition,
        driver: &str,
    ) -> Result<(), String> {
        // A field carrying the configured primary key name is the table's primary key even
        // without the modifier - the model generator treats it the same way.
        let is_primary_key = field.primary_key || field.name == self.config.model.primary_key;

        if field.auto_increment && !is_primary_key {
            return Err(format!(
                "Field '{}' is auto_increment but not the primary key; only a primary key column is numbered by the database",
                field.name
            ));
        }
        if is_primary_key && field.auto_increment {
            // A numbered key keeps the width the model reads it as: a wider one is a
            // column the driver will not decode into the field.
            match field.migration_column_type(driver) {
                ColumnType::Integer => {
                    builder.increments(&field.name);
                }
                ColumnType::BigInteger => {
                    builder.big_increments(&field.name);
                }
                // SQLite numbers only an `INTEGER PRIMARY KEY`, which holds any width.
                ColumnType::SmallInteger if canonical_driver(driver) == Some("sqlite") => {
                    builder.increments(&field.name);
                }
                ColumnType::SmallInteger => {
                    let serial = if self.is_mysql() {
                        "SMALLINT AUTO_INCREMENT"
                    } else {
                        "SMALLSERIAL"
                    };
                    builder
                        .column(&field.name, ColumnType::Custom(serial.to_string()))
                        .primary_key();
                }
                _ => {
                    return Err(format!(
                        "Field '{}' is auto_increment, which takes an integer key (i16, i32 or i64)",
                        field.name
                    ));
                }
            }
            return Ok(());
        }

        let column = builder.column(&field.name, field.migration_column_type(driver));
        let column = if is_primary_key {
            column.primary_key()
        } else if field.nullable {
            column.nullable()
        } else {
            column.not_null()
        };
        let column = if field.unique {
            column.unique()
        } else {
            column
        };
        if let Some(default) = &field.default {
            column.default(DefaultValue::Raw(default.clone()));
        }
        Ok(())
    }

    /// Update mod.rs with new migration
    fn update_mod_file(&self, file_name: &str) -> Result<(), String> {
        let mod_path = format!("{}/mod.rs", self.migrations_dir());
        let file_stem = file_name.trim_end_matches(".rs");
        let module_name = migration_module_name(file_stem);

        let existing = read_source_or_empty(&mod_path)?;

        let module_decl = if module_name == file_stem {
            format!("pub mod {};", module_name)
        } else {
            format!("#[path = \"{}\"]\npub mod {};", file_name, module_name)
        };

        if has_module_declaration(&existing, &module_name) {
            return Ok(());
        }

        let new_content = append_source_block(&existing, &module_decl);

        std::fs::write(&mod_path, new_content)
            .map_err(|e| format!("Failed to update mod.rs: {}", e))?;

        Ok(())
    }
}

/// The version of a migration made without a timestamp: one past the highest
/// numbered migration in `directory`, four digits or more, so that versions
/// compare as text in the order they were made.
fn next_migration_number(directory: &str) -> String {
    let highest = std::fs::read_dir(directory)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let (number, _) = name.strip_suffix(".rs")?.split_once('_')?;
            if number.is_empty() || !number.bytes().all(|byte| byte.is_ascii_digit()) {
                return None;
            }
            number.parse::<u128>().ok()
        })
        .max()
        .unwrap_or(0);
    format!("{:04}", highest + 1)
}

const DEFAULT_MIGRATION_TEMPLATE: &str = r##"//! Migration: {{ name }}
//!
//! {{ description }}

use tideorm::prelude::*;

/// Migration: {{ name }}
pub struct {{ struct_name }};

#[async_trait]
impl Migration for {{ struct_name }} {
    fn version(&self) -> &str {
        "{{ version }}"
    }

    fn name(&self) -> &str {
        "{{ name }}"
    }

    async fn up(&self, schema: &mut Schema) -> tideorm::Result<()> {
{% if up_mode == "raw_sql" %}        schema.raw(r#"
{{ up_raw_sql }}
        "#).await?;
{% else %}{% for statement in up_statements %}{{ statement }}
{% endfor %}{% endif %}

        Ok(())
    }

    async fn down(&self, schema: &mut Schema) -> tideorm::Result<()> {
{% if down_mode == "raw_sql" %}        schema.raw(r#"{{ down_raw_sql }} "#).await?;
{% else %}{% for statement in down_statements %}{{ statement }}
{% endfor %}{% endif %}

        Ok(())
    }
}
"##;

#[derive(Serialize)]
struct MigrationTemplateContext {
    name: String,
    version: String,
    struct_name: String,
    description: String,
    up_mode: String,
    down_mode: String,
    up_raw_sql: Option<String>,
    down_raw_sql: Option<String>,
    up_statements: Vec<String>,
    down_statements: Vec<String>,
}

/// Convert string to PascalCase
fn to_pascal_case(s: &str) -> String {
    heck::AsPascalCase(s).to_string()
}

/// The name TideORM gives an unnamed index on `table`'s `column`: `idx_<table>_<column>`.
///
/// A schema-qualified table contributes its own name only, so the index name stays one
/// identifier.
/// Refuse a driver the generators write no migration DDL for.
pub(crate) fn ensure_ddl_driver(driver: &str) -> Result<(), String> {
    match canonical_driver(driver) {
        Some(_) => Ok(()),
        None => Err(format!(
            "Unsupported database driver for migration DDL: {driver}"
        )),
    }
}

/// The name TideORM gives the index of `column` on the bare table `table`.
fn index_name(table: &str, column: &str, unique: bool) -> String {
    IndexDefinition::__generated(table, vec![column.to_string()], unique).name
}

fn migration_module_name(file_stem: &str) -> String {
    if file_stem
        .chars()
        .next()
        .is_some_and(|ch| ch.is_ascii_digit())
    {
        format!("m_{}", file_stem)
    } else {
        escape_ident(file_stem)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_default_mysql_primary_key_sql_has_single_auto_increment() {
        let mut config = TideConfig::default();
        config.database.driver = "mysql".to_string();
        config.migration.timestamps = false;

        let generator = MigrationGenerator::new(&config);
        let content = generator
            .generate_create_table(
                "create_users_table",
                "20260316_001",
                "users",
                &[],
                false,
                false,
            )
            .unwrap();

        assert!(content.contains("`id` BIGINT AUTO_INCREMENT"));
        assert!(content.contains("PRIMARY KEY (`id`)"));
        assert_eq!(content.matches("AUTO_INCREMENT").count(), 1);
    }

    #[test]
    fn test_sqlite_explicit_auto_increment_primary_key_uses_integer() {
        let mut config = TideConfig::default();
        config.database.driver = "sqlite".to_string();
        config.migration.timestamps = false;

        let generator = MigrationGenerator::new(&config);
        let fields =
            vec![FieldDefinition::parse("custom_id:i64:primary_key:auto_increment").unwrap()];
        let content = generator
            .generate_create_table(
                "create_users_table",
                "20260316_001",
                "users",
                &fields,
                false,
                false,
            )
            .unwrap();

        // An `INTEGER` column that is the whole primary key is SQLite's rowid.
        assert!(content.contains("\"custom_id\" INTEGER NOT NULL"));
        assert!(content.contains("PRIMARY KEY (\"custom_id\")"));
        assert!(!content.contains("\"custom_id\" BIGINT"));
    }

    #[test]
    fn test_auto_increment_is_refused_on_a_column_that_is_not_the_key() {
        let mut config = TideConfig::default();
        config.database.driver = "sqlite".to_string();
        config.migration.timestamps = false;

        let generator = MigrationGenerator::new(&config);
        let fields = vec![FieldDefinition::parse("counter:i64:auto_increment").unwrap()];
        let error = generator
            .generate_create_table(
                "create_counters_table",
                "1",
                "counters",
                &fields,
                false,
                false,
            )
            .unwrap_err();

        assert!(error.contains("only a primary key column"), "{error}");
    }

    #[test]
    fn test_timestamped_migration_module_name_is_sanitized() {
        assert_eq!(
            migration_module_name("20260316203329_create_posts_table"),
            "m_20260316203329_create_posts_table"
        );
        assert_eq!(
            migration_module_name("create_posts_table"),
            "create_posts_table"
        );
        assert_eq!(migration_module_name("type"), "r#type");
    }

    #[test]
    fn test_explicit_id_field_still_gets_a_primary_key() {
        let mut config = TideConfig::default();
        config.database.driver = "postgres".to_string();
        config.migration.timestamps = false;

        let generator = MigrationGenerator::new(&config);
        let fields = vec![FieldDefinition::parse("id:i32").unwrap()];
        let content = generator
            .generate_create_table(
                "create_readings_table",
                "20260316_001",
                "readings",
                &fields,
                false,
                false,
            )
            .unwrap();

        assert!(content.contains("\"id\" INTEGER"));
        assert!(content.contains("PRIMARY KEY (\"id\")"));
        assert!(!content.contains("BIGSERIAL"));
    }

    #[test]
    fn test_migration_template_override_is_used() {
        let dir = tempdir().unwrap();
        let template_path = dir.path().join("migration.rs.j2");
        std::fs::write(
            &template_path,
            "// custom migration {{ name }} {{ description }}\n",
        )
        .unwrap();

        let mut config = TideConfig::default();
        config.migration.template = Some(template_path.to_string_lossy().into_owned());
        config.migration.timestamps = false;

        let generator = MigrationGenerator::new(&config);
        let content = generator
            .generate_create_table(
                "create_users_table",
                "20260316_001",
                "users",
                &[],
                false,
                false,
            )
            .unwrap();

        assert_eq!(
            content,
            "// custom migration create_users_table Creates the users table."
        );
    }

    #[test]
    fn test_reserved_words_are_quoted_in_generated_ddl() {
        let mut config = TideConfig::default();
        config.database.driver = "postgres".to_string();
        config.migration.timestamps = false;

        let generator = MigrationGenerator::new(&config);
        let fields = vec![
            FieldDefinition::parse("order:i32").unwrap(),
            FieldDefinition::parse("user:string").unwrap(),
        ];
        let content = generator
            .generate_create_table("create_orders_table", "1", "order", &fields, true, true)
            .unwrap();

        assert!(content.contains("CREATE TABLE IF NOT EXISTS \"order\" ("));
        assert!(content.contains("\"order\" INTEGER NOT NULL"));
        assert!(content.contains("\"user\" VARCHAR(255) NOT NULL"));
        assert!(content.contains("\"created_at\" TIMESTAMPTZ NOT NULL"));
        assert!(content.contains("\"deleted_at\" TIMESTAMPTZ"));
        assert!(!content.contains("\"deleted_at\" TIMESTAMPTZ NOT NULL"));
        assert!(content.contains("DROP TABLE IF EXISTS \"order\""));
    }

    #[test]
    fn test_alter_table_quotes_table_and_columns() {
        let mut config = TideConfig::default();
        config.database.driver = "mysql".to_string();
        config.migration.timestamps = false;

        let generator = MigrationGenerator::new(&config);
        let fields = vec![FieldDefinition::parse("order:i32:nullable").unwrap()];
        let content = generator
            .generate_alter_table("add_order_to_users", "1", "user", &fields)
            .unwrap();

        assert!(content.contains("ALTER TABLE `user` ADD COLUMN `order` INT"));
        assert!(content.contains("ALTER TABLE `user` DROP COLUMN `order`"));
    }

    #[test]
    fn test_table_names_that_would_break_the_raw_string_are_rejected() {
        let config = TideConfig::default();
        let generator = MigrationGenerator::new(&config);

        // `"#` closes the raw string literal the DDL is emitted into, which would let a
        // table name inject arbitrary Rust into the generated migration.
        let error = generator
            .generate_create_table(
                "create_evil_table",
                "1",
                "t\"#; fn evil() {}",
                &[],
                false,
                false,
            )
            .unwrap_err();
        assert!(error.contains("Invalid table name"));
    }

    #[test]
    fn test_empty_migration_template_contains_no_example_sql() {
        let config = TideConfig::default();
        let generator = MigrationGenerator::new(&config);
        let content = generator.generate_empty("do_something", "1").unwrap();

        // The migration runner recovers statements from raw string literals, so a stub
        // must not carry anything that could be read back as SQL.
        assert!(!content.contains("r#\""));
        assert!(!content.contains("CREATE TABLE"));
        assert!(!content.contains("DROP TABLE"));
        assert!(content.contains("// TODO: Implement the forward migration."));
    }

    #[test]
    fn test_repeated_migration_names_are_rejected() {
        let dir = tempdir().unwrap();
        let migrations = dir.path().join("migrations");
        std::fs::create_dir_all(&migrations).unwrap();
        std::fs::write(
            migrations.join("20260316203329001_create_users_table.rs"),
            "// existing\n",
        )
        .unwrap();

        let mut config = TideConfig::default();
        config.paths.migrations = migrations.to_string_lossy().into_owned();

        let generator = MigrationGenerator::new(&config);
        let error = generator
            .generate(
                "create_users_table",
                Some("users".to_string()),
                None,
                None,
                false,
                false,
            )
            .unwrap_err();

        // `name` is UNIQUE in the migrations table, so the second file could never run.
        assert!(error.contains("already exists"), "{}", error);
    }

    #[test]
    fn unsupported_drivers_do_not_generate_sqlite_migrations() {
        let dir = tempdir().unwrap();
        let output = dir.path().join("migrations");
        let mut config = TideConfig::default();
        config.database.driver = "oracle".to_string();
        config.migration.timestamps = false;

        let error = MigrationGenerator::new(&config)
            .output_dir(Some(output.to_str().unwrap()))
            .generate(
                "create_users_table",
                Some("users".to_string()),
                None,
                None,
                false,
                false,
            )
            .unwrap_err();

        assert!(error.contains("Unsupported database driver"));
        assert!(!output.exists());
    }

    #[test]
    fn conflicting_migration_targets_are_rejected_before_creating_output() {
        let dir = tempdir().unwrap();
        let output = dir.path().join("migrations");
        let config = TideConfig::default();

        let error = MigrationGenerator::new(&config)
            .output_dir(Some(output.to_str().unwrap()))
            .generate(
                "change_users",
                Some("users".to_string()),
                Some("users".to_string()),
                None,
                false,
                false,
            )
            .unwrap_err();

        assert!(error.contains("cannot create and alter"));
        assert!(!output.exists());
    }

    #[test]
    fn fields_without_a_migration_target_are_rejected_before_creating_output() {
        let dir = tempdir().unwrap();
        let output = dir.path().join("migrations");
        let config = TideConfig::default();

        let error = MigrationGenerator::new(&config)
            .output_dir(Some(output.to_str().unwrap()))
            .generate(
                "change_users",
                None,
                None,
                Some("name:string".to_string()),
                false,
                false,
            )
            .unwrap_err();

        assert!(error.contains("fields require --create or --table"));
        assert!(!output.exists());
    }

    #[test]
    fn invalid_migration_type_names_are_rejected_before_creating_output() {
        let dir = tempdir().unwrap();
        let output = dir.path().join("migrations");
        let config = TideConfig::default();

        for name in ["", "123", "self"] {
            let error = MigrationGenerator::new(&config)
                .output_dir(Some(output.to_str().unwrap()))
                .generate(name, None, None, None, false, false)
                .unwrap_err();

            assert!(error.contains("migration"), "{error}");
            assert!(!output.exists());
        }
    }

    /// Without timestamps, a migration made after another runs after it,
    /// whatever their names: `add_age_to_users` sorts before
    /// `create_users_table` and used to run first.
    #[test]
    fn test_migrations_without_timestamps_keep_the_order_they_were_made_in() {
        let dir = tempdir().unwrap();
        let mut config = TideConfig::default();
        config.paths.migrations = dir.path().to_string_lossy().into_owned();
        config.migration.timestamps = false;
        let generator = MigrationGenerator::new(&config);

        let created = generator
            .generate(
                "create_users_table",
                Some("users".to_string()),
                None,
                None,
                false,
                false,
            )
            .unwrap();
        let altered = generator
            .generate(
                "add_age_to_users",
                None,
                Some("users".to_string()),
                Some("age:i32".to_string()),
                false,
                false,
            )
            .unwrap();

        assert!(
            created.ends_with("/0001_create_users_table.rs"),
            "{created}"
        );
        assert!(altered.ends_with("/0002_add_age_to_users.rs"), "{altered}");
        let altered = std::fs::read_to_string(altered).unwrap();
        assert!(altered.contains("\"0002\""), "{altered}");
    }

    /// Every statement of a migration for `app.users` names the same table:
    /// the create and alter statements quoted it whole, as a table called
    /// `app.users`, where the drop and the indexes named `users` in `app`.
    #[test]
    fn test_schema_qualified_tables_are_named_alike_everywhere() {
        let mut config = TideConfig::default();
        config.database.driver = "postgres".to_string();
        let generator = MigrationGenerator::new(&config);
        let fields = [FieldDefinition::parse("email:string:indexed").unwrap()];

        let created = generator
            .generate_create_table(
                "create_users_table",
                "1",
                "app.users",
                &fields,
                false,
                false,
            )
            .unwrap();
        assert!(
            created.contains("CREATE TABLE IF NOT EXISTS \"app\".\"users\" ("),
            "{created}"
        );
        assert!(!created.contains("\"app.users\""), "{created}");
        assert!(
            created.contains(
                "CREATE INDEX IF NOT EXISTS \"idx_users_email\" ON \"app\".\"users\" (\"email\")"
            ),
            "{created}"
        );
        assert!(
            created.contains("DROP TABLE IF EXISTS \"app\".\"users\""),
            "{created}"
        );

        let altered = generator
            .generate_alter_table("add_email_to_users", "2", "app.users", &fields)
            .unwrap();
        assert!(
            altered.contains("ALTER TABLE \"app\".\"users\" ADD COLUMN"),
            "{altered}"
        );
        assert!(
            altered.contains("ALTER TABLE \"app\".\"users\" DROP COLUMN"),
            "{altered}"
        );
        assert!(
            altered.contains("DROP INDEX \"app\".\"idx_users_email\""),
            "{altered}"
        );
        assert!(!altered.contains("\"app.users\""), "{altered}");
    }

    /// SQLite refuses `ADD COLUMN .. UNIQUE`, so a unique field added there is a column
    /// and a unique index named as TideORM names one, dropped before the column.
    #[test]
    fn test_a_unique_column_added_on_sqlite_is_a_unique_index() {
        let mut config = TideConfig::default();
        config.database.driver = "sqlite".to_string();
        let fields = [FieldDefinition::parse("email:string:unique").unwrap()];
        let content = MigrationGenerator::new(&config)
            .generate_alter_table("add_email_to_users", "1", "users", &fields)
            .unwrap();

        assert!(!content.contains(" UNIQUE \"#"), "{content}");
        assert!(
            content.contains(
                "CREATE UNIQUE INDEX \"idx_users_email_unique\" ON \"users\" (\"email\")"
            ),
            "{content}"
        );
        let drop_index = "DROP INDEX IF EXISTS \"idx_users_email_unique\"";
        assert_eq!(content.matches(drop_index).count(), 1, "{content}");
        let down = &content[content.find("async fn down").unwrap()..];
        assert!(
            down.find(drop_index).unwrap() < down.find("DROP COLUMN").unwrap(),
            "{content}"
        );
    }

    /// An `i16` key numbered by the database got a `BIGSERIAL`/`BIGINT` column,
    /// which the driver refuses to read into the model's `i16`.
    #[test]
    fn test_a_numbered_key_keeps_its_width() {
        let fields = [FieldDefinition::parse("id:i16:primary_key:auto_increment").unwrap()];
        let mut config = TideConfig::default();
        for (driver, column) in [
            ("postgres", "\"id\" SMALLSERIAL"),
            ("mysql", "`id` SMALLINT AUTO_INCREMENT"),
            ("sqlite", "\"id\" INTEGER"),
        ] {
            config.database.driver = driver.to_string();
            let content = MigrationGenerator::new(&config)
                .generate_create_table("create_users_table", "1", "users", &fields, false, false)
                .unwrap();
            assert!(content.contains(column), "{driver}: {content}");
            assert!(!content.contains("BIG"), "{driver}: {content}");
        }

        let uuid = [FieldDefinition::parse("id:uuid:primary_key:auto_increment").unwrap()];
        let error = MigrationGenerator::new(&config)
            .generate_create_table("create_users_table", "1", "users", &uuid, false, false)
            .unwrap_err();
        assert!(error.contains("integer key"), "{error}");
    }

    #[test]
    fn test_output_dir_overrides_the_configured_migrations_path() {
        let dir = tempdir().unwrap();
        let output = dir.path().join("custom").to_string_lossy().into_owned();

        let mut config = TideConfig::default();
        config.paths.migrations = dir.path().join("configured").to_string_lossy().into_owned();
        config.migration.timestamps = false;

        let path = MigrationGenerator::new(&config)
            .output_dir(Some(&output))
            .generate("do_something", None, None, None, false, false)
            .unwrap();

        assert!(path.starts_with(&output), "{}", path);
        assert!(std::path::Path::new(&format!("{}/mod.rs", output)).exists());
    }

    fn create_users_table(config: &TideConfig, fields: &[&str]) -> String {
        let fields: Vec<_> = fields
            .iter()
            .map(|field| FieldDefinition::parse(field).unwrap())
            .collect();
        MigrationGenerator::new(config)
            .generate_create_table("create_users_table", "1", "users", &fields, false, false)
            .unwrap()
    }

    /// A `mariadb` driver used to fall through to SQLite's DDL.
    #[test]
    fn test_mariadb_projects_get_mysql_ddl() {
        let mut config = TideConfig::default();
        config.database.driver = "mariadb".to_string();

        let content = create_users_table(&config, &["name:string"]);
        assert!(content.contains("`id` BIGINT AUTO_INCREMENT"), "{content}");
        assert!(
            content.contains("`name` VARCHAR(255) NOT NULL"),
            "{content}"
        );
    }

    #[test]
    fn test_default_primary_key_follows_the_configured_type() {
        let mut config = TideConfig::default();
        config.database.driver = "postgres".to_string();

        config.model.primary_key_type = "uuid".to_string();
        let content = create_users_table(&config, &[]);
        assert!(content.contains("\"id\" UUID,"), "{content}");
        assert!(content.contains("PRIMARY KEY (\"id\")"), "{content}");
        assert!(!content.contains("SERIAL"), "{content}");

        config.model.primary_key_type = "i32".to_string();
        let content = create_users_table(&config, &[]);
        assert!(content.contains("\"id\" SERIAL"), "{content}");
        assert!(!content.contains("BIGSERIAL"), "{content}");

        config.model.primary_key_type = "i64".to_string();
        let content = create_users_table(&config, &[]);
        assert!(content.contains("\"id\" BIGSERIAL"), "{content}");
    }

    #[test]
    fn test_indexed_fields_get_an_index_in_create_migrations() {
        let mut config = TideConfig::default();
        config.database.driver = "postgres".to_string();

        let content = create_users_table(&config, &["email:string:indexed", "name:string"]);
        assert!(
            content.contains(
                "        schema.raw(r#\"CREATE INDEX IF NOT EXISTS \"idx_users_email\" ON \"users\" (\"email\") \"#).await?;"
            ),
            "{content}"
        );
        assert!(content.contains("CREATE TABLE IF NOT EXISTS \"users\""));
        assert!(!content.contains("idx_users_name"), "{content}");
        // The table takes its indexes along.
        assert!(content.contains("schema.raw(r#\"DROP TABLE IF EXISTS \"users\" \"#)"));

        config.database.driver = "mysql".to_string();
        let content = create_users_table(&config, &["email:string:indexed"]);
        assert!(
            content.contains("CREATE INDEX `idx_users_email` ON `users` (`email`)"),
            "{content}"
        );

        // MySQL refuses an index on a TEXT column (error 1170), so the column is
        // a VARCHAR as a unique one is.
        let content = create_users_table(&config, &["slug:text:indexed"]);
        assert!(
            content.contains("`slug` VARCHAR(255) NOT NULL"),
            "{content}"
        );
        assert!(!content.contains("LONGTEXT"), "{content}");
    }

    #[test]
    fn test_indexed_fields_get_an_index_in_alter_migrations() {
        let mut config = TideConfig::default();
        config.database.driver = "sqlite".to_string();

        let fields = vec![FieldDefinition::parse("email:string:nullable:indexed").unwrap()];
        let content = MigrationGenerator::new(&config)
            .generate_alter_table("add_email_to_users", "1", "users", &fields)
            .unwrap();

        let position = |needle: &str| {
            content
                .find(needle)
                .unwrap_or_else(|| panic!("missing {needle}:\n{content}"))
        };
        let create_index = "CREATE INDEX \"idx_users_email\" ON \"users\" (\"email\")";
        let drop_index = "DROP INDEX \"idx_users_email\"";
        assert!(position("ADD COLUMN \"email\"") < position(create_index));
        // SQLite cannot drop a column an index still covers.
        assert!(position(drop_index) < position("DROP COLUMN \"email\""));

        config.database.driver = "mysql".to_string();
        let content = MigrationGenerator::new(&config)
            .generate_alter_table("add_email_to_users", "1", "users", &fields)
            .unwrap();
        assert!(
            content.contains("DROP INDEX `idx_users_email` ON `users`"),
            "{content}"
        );
    }
}
