//! Migration generator for TideORM CLI

use crate::config::{TideConfig, canonical_driver};
use crate::generators::Generated;
use crate::generators::registry::{MIGRATIONS, register};
use crate::utils::{
    FieldDefinition, auto_increments, database_type, ensure_writable, escape_ident,
    migration_timestamp, module_declaration, print_warning, render_template, split_definitions,
    to_pascal_case, to_snake_case, validate_rust_type_name, validate_sql_object_name,
};
use serde::Serialize;
use tideorm::config::DatabaseType;
use tideorm::migration::{ColumnType, StringLen, column_type_sql};
use versions::{ensure_no_conflicting_migration, next_migration_number};

mod versions;

/// Initial `mod.rs` of a migrations directory: an empty registry.
pub(crate) const MIGRATIONS_MOD_RS: &str = r#"//! Database migrations

use tideorm::prelude::*;

/// The migrations `tideorm migrate` runs, in version order. `tideorm make
/// migration` registers new ones here.
pub fn all() -> Migrator {
    Migrator::new()
}
"#;

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
        self.render(
            name,
            create_table,
            alter_table,
            fields,
            include_timestamps,
            include_soft_deletes,
        )?
        .write()
    }

    /// Check and render a migration without writing anything.
    pub fn render(
        &self,
        name: &str,
        create_table: Option<String>,
        alter_table: Option<String>,
        fields: Option<String>,
        include_timestamps: bool,
        include_soft_deletes: bool,
    ) -> Result<Generated<'_>, String> {
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

        // Migrations run in version order. Without a timestamp the version is
        // the next number in the directory, so they still run in the order
        // they were made rather than by name.
        let version = if self.config.migration.timestamps {
            migration_timestamp()
        } else {
            next_migration_number(directory)?
        };
        let file_name = format!("{}_{}.rs", version, migration_name);

        let file_path = format!("{}/{}", directory, file_name);

        ensure_writable(&file_path)?;
        ensure_no_conflicting_migration(directory, &version, &migration_name)?;

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

        Ok(Generated::new(file_path, content, move || {
            self.register_migration(&file_name, &struct_name)
        }))
    }

    fn is_mysql(&self) -> bool {
        database_type(&self.config.database.driver) == DatabaseType::MySQL
    }

    /// Generate a create table migration
    ///
    /// The table is declared through TideORM's `Schema::create_table`, which
    /// renders it for the driver the migration runs on, and names its indexes
    /// as TideORM names a model's `#[index(..)]`, so schema sync finds them.
    fn generate_create_table(
        &self,
        name: &str,
        version: &str,
        table: &str,
        fields: &[FieldDefinition],
        include_timestamps: bool,
        include_soft_deletes: bool,
    ) -> Result<String, String> {
        validate_sql_object_name("table", table)?;
        let driver = &self.config.database.driver;

        let mut columns = Vec::new();
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
            columns.push(self.create_column(&key, driver)?);
        }
        for field in fields {
            columns.push(self.create_column(field, driver)?);
        }
        if include_timestamps {
            columns.push("t.timestamps();".to_string());
        }
        if include_soft_deletes {
            columns.push("t.soft_deletes();".to_string());
        }
        for field in fields.iter().filter(|field| field.indexed) {
            columns.push(format!("t.index(&[{:?}]);", field.name));
        }

        let context = MigrationTemplateContext {
            name: name.to_string(),
            version: version.to_string(),
            struct_name: to_pascal_case(name),
            description: format!("Creates the {} table.", table),
            up_statements: vec![schema_call("create_table_if_not_exists", table, &columns)],
            down_statements: vec![format!(
                "        schema.drop_table_if_exists({table:?}).await?;"
            )],
        };

        self.render_migration_template(&context)
    }

    /// Generate an alter table migration
    ///
    /// Its `down()` drops the columns its `up()` adds, and each backend drops
    /// a dropped column's indexes with it: TideORM's `Schema::alter_table`
    /// does so on SQLite, which refuses to drop a column an index uses.
    fn generate_alter_table(
        &self,
        name: &str,
        version: &str,
        table: &str,
        fields: &[FieldDefinition],
    ) -> Result<String, String> {
        validate_sql_object_name("table", table)?;
        let driver = &self.config.database.driver;

        let mut added = Vec::new();
        let mut dropped = Vec::new();
        for field in fields {
            let column = self.column_call(
                "add_column",
                &field.name,
                &field.migration_column_type(driver),
            );
            let nullability = if field.nullable {
                ".nullable()"
            } else {
                ".not_null()"
            };
            added.push(with_modifiers(column + nullability, field));
            dropped.push(format!("t.drop_column({:?});", field.name));
        }
        for field in fields.iter().filter(|field| field.indexed) {
            added.push(format!("t.add_index(&[{:?}]);", field.name));
        }

        let context = MigrationTemplateContext {
            name: name.to_string(),
            version: version.to_string(),
            struct_name: to_pascal_case(name),
            description: format!("Alters the {} table.", table),
            up_statements: vec![schema_call("alter_table", table, &added)],
            down_statements: vec![schema_call("alter_table", table, &dropped)],
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
            up_statements: vec![
                "        // TODO: Implement the forward migration.".to_string(),
                "        // Create, alter or drop tables through `schema`.".to_string(),
                "        let _ = schema; // Remove once implemented.".to_string(),
            ],
            down_statements: vec![
                "        // TODO: Implement the reverse migration.".to_string(),
                "        // Undo everything that up() applies.".to_string(),
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
            for field in split_definitions(fields_str, ',') {
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

    /// The line declaring `field` on the table a migration creates.
    fn create_column(&self, field: &FieldDefinition, driver: &str) -> Result<String, String> {
        // A field carrying the configured primary key name is the table's primary key even
        // without the modifier - the model generator treats it the same way.
        let is_primary_key = field.primary_key || field.name == self.config.model.primary_key;
        let name = &field.name;

        if field.auto_increment && !is_primary_key {
            return Err(format!(
                "Field '{}' is auto_increment but not the primary key; only a primary key column is numbered by the database",
                name
            ));
        }
        let column_type = field.migration_column_type(driver);
        if is_primary_key && field.auto_increment {
            // A numbered key keeps the width the model reads it as: a wider one is a
            // column the driver will not decode into the field.
            return match column_type {
                ColumnType::Integer => Ok(format!("t.increments({name:?});")),
                ColumnType::BigInteger => Ok(format!("t.big_increments({name:?});")),
                // SQLite numbers only an `INTEGER PRIMARY KEY`, which holds any width.
                ColumnType::SmallInteger if canonical_driver(driver) == Some("sqlite") => {
                    Ok(format!("t.increments({name:?});"))
                }
                ColumnType::SmallInteger => {
                    let serial = if self.is_mysql() {
                        "SMALLINT AUTO_INCREMENT"
                    } else {
                        "SMALLSERIAL"
                    };
                    Ok(format!(
                        "t.column({name:?}, ColumnType::custom({serial:?})).primary_key();"
                    ))
                }
                _ => Err(format!(
                    "Field '{}' is auto_increment, which takes an integer key (i16, i32 or i64)",
                    name
                )),
            };
        }

        let column = self.column_call("column", name, &column_type);
        let constraint = if is_primary_key {
            ".primary_key()"
        } else if field.nullable {
            ".nullable()"
        } else {
            ".not_null()"
        };
        Ok(with_modifiers(column + constraint, field))
    }

    /// The builder call adding the column `name` of `column_type`: the
    /// builder's method for that type, or `add` (`column` or `add_column`)
    /// with the type's SQL for the configured driver.
    fn column_call(&self, add: &str, name: &str, column_type: &ColumnType) -> String {
        let method = match column_type {
            ColumnType::String(StringLen::None) => "string",
            ColumnType::Text => "text",
            ColumnType::SmallInteger => "small_integer",
            ColumnType::Integer => "integer",
            ColumnType::BigInteger => "big_integer",
            ColumnType::Float => "float",
            ColumnType::Double => "double",
            ColumnType::Boolean => "boolean",
            ColumnType::TimestampWithTimeZone => "timestamptz",
            ColumnType::Date => "date",
            ColumnType::Time => "time",
            ColumnType::Uuid => "uuid",
            ColumnType::Json => "json",
            ColumnType::JsonBinary => "jsonb",
            ColumnType::Blob => "binary",
            ColumnType::Decimal(Some((precision, scale))) => {
                return format!("t.decimal_with({name:?}, {precision}, {scale})");
            }
            ColumnType::Array(item) if **item == ColumnType::Integer => "integer_array",
            ColumnType::Array(item) if **item == ColumnType::Text => "text_array",
            // The field's own width where the backend has it (`TINYINT`), which
            // refuses a value the model cannot read; `SMALLINT` on PostgreSQL.
            ColumnType::TinyInteger => {
                return format!("t.{add}({name:?}, ColumnType::TinyInteger)");
            }
            other => {
                let sql = column_type_sql(other, database_type(&self.config.database.driver));
                return format!("t.{add}({name:?}, ColumnType::custom({sql:?}))");
            }
        };
        format!("t.{method}({name:?})")
    }

    /// Declare the migration in `file_name` in its directory's `mod.rs` and
    /// add it to the `migrations::all()` registry there, which the project's
    /// migration runner runs.
    fn register_migration(&self, file_name: &str, struct_name: &str) -> Result<(), String> {
        let file_stem = file_name.trim_end_matches(".rs");
        let module = migration_module_name(file_stem);
        let declaration = module_declaration(&module, file_stem);
        let entry = format!(".add({module}::{struct_name})");
        if !register(
            self.migrations_dir(),
            &module,
            &declaration,
            &MIGRATIONS,
            &entry,
        )? {
            print_warning(&format!(
                "{}/mod.rs has no `{}` registry: add `{}` to the Migrator your runner uses, or `tideorm migrate` will not run it",
                self.migrations_dir(),
                MIGRATIONS.signature,
                entry
            ));
        }
        Ok(())
    }
}

/// `column`, a builder call, with the modifiers `field` asks for, as a
/// statement.
fn with_modifiers(mut column: String, field: &FieldDefinition) -> String {
    if field.unique {
        column.push_str(".unique()");
    }
    if let Some(default) = &field.default {
        column.push_str(&format!(".default(DefaultValue::Raw({default:?}.into()))"));
    }
    column.push(';');
    column
}

/// The statement calling the `Schema` method `method` on `table`, with a
/// closure running `lines` on the table's builder `t`.
fn schema_call(method: &str, table: &str, lines: &[String]) -> String {
    let builder = if lines.is_empty() { "_t" } else { "t" };
    let body: String = lines
        .iter()
        .map(|line| format!("                {line}\n"))
        .collect();
    format!(
        "        schema\n            .{method}({table:?}, |{builder}| {{\n{body}            }})\n            .await?;"
    )
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
{% for statement in up_statements %}{{ statement }}
{% endfor %}
        Ok(())
    }

    async fn down(&self, schema: &mut Schema) -> tideorm::Result<()> {
{% for statement in down_statements %}{{ statement }}
{% endfor %}
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
    up_statements: Vec<String>,
    down_statements: Vec<String>,
}

/// Refuse a driver the generators write no migration DDL for.
pub(crate) fn ensure_ddl_driver(driver: &str) -> Result<(), String> {
    match canonical_driver(driver) {
        Some(_) => Ok(()),
        None => Err(format!(
            "Unsupported database driver for migration DDL: {driver}"
        )),
    }
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
#[path = "../../tests/unit/generators_migration_tests.rs"]
mod tests;
