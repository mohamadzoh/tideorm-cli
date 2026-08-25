//! Migration generator for TideORM CLI

use crate::config::TideConfig;
use crate::utils::{
    FieldDefinition, ensure_directory, ensure_writable, escape_ident, migration_timestamp,
    quote_sql_identifier, render_template, to_snake_case,
};
use serde::Serialize;

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
        let directory = self.migrations_dir();
        ensure_directory(directory)?;

        let migration_name = to_snake_case(name);
        let timestamp = if self.config.migration.timestamps {
            migration_timestamp()
        } else {
            String::new()
        };

        let file_name = if timestamp.is_empty() {
            format!("{}.rs", migration_name)
        } else {
            format!("{}_{}.rs", timestamp, migration_name)
        };

        let file_path = format!("{}/{}", directory, file_name);

        ensure_writable(&file_path)?;
        Self::ensure_no_conflicting_migration(directory, &timestamp, &migration_name)?;

        // Parse fields
        let parsed_fields = Self::parse_fields(fields.as_deref())?;

        let version = if timestamp.is_empty() {
            migration_name.clone()
        } else {
            timestamp.clone()
        };

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
    /// Both `version` and `name` are UNIQUE in the migrations table, so two files sharing
    /// either value can be written but only one of them can ever run. Timestamps carry
    /// milliseconds, which makes a version clash unlikely, but reusing a name is easy to
    /// do by accident and is worth reporting at generation time.
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

            if !timestamp.is_empty() && stem.starts_with(&format!("{}_", timestamp)) {
                return Err(format!(
                    "Migration version '{}' is already used by '{}.rs'. Re-run to get a fresh timestamp.",
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

        // Generate columns SQL
        let mut columns = Vec::new();

        if !fields
            .iter()
            .any(|field| field.primary_key || field.name == self.config.model.primary_key)
        {
            columns.push(self.default_primary_key_sql(driver)?);
        }

        for field in fields {
            columns.push(self.build_column_sql(field, driver)?);
        }

        // Add timestamps
        if include_timestamps {
            columns.push(format!(
                "            {} {} NOT NULL DEFAULT {}",
                self.quote("column", "created_at")?,
                self.get_timestamp_type(driver),
                self.get_now_function(driver)
            ));
            columns.push(format!(
                "            {} {} NOT NULL DEFAULT {}",
                self.quote("column", "updated_at")?,
                self.get_timestamp_type(driver),
                self.get_now_function(driver)
            ));
        }

        if include_soft_deletes {
            columns.push(format!(
                "            {} {} NULL",
                self.quote("column", "deleted_at")?,
                self.get_timestamp_type(driver)
            ));
        }

        let raw_sql = format!(
            "        CREATE TABLE IF NOT EXISTS {} (\n{}\n        )",
            quoted_table,
            columns.join(",\n")
        );

        let context = MigrationTemplateContext {
            name: name.to_string(),
            version: version.to_string(),
            struct_name,
            description: format!("Creates the {} table.", table),
            up_mode: "raw_sql".to_string(),
            down_mode: "raw_sql".to_string(),
            up_raw_sql: Some(raw_sql),
            down_raw_sql: Some(format!("DROP TABLE IF EXISTS {}", quoted_table)),
            up_statements: Vec::new(),
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
        let quoted_table = self.quote("table", table)?;

        // Generate add column statements
        let mut up_statements = Vec::new();
        let mut down_statements = Vec::new();

        for field in fields {
            let quoted_column = self.quote("column", &field.name)?;
            let mut col_def = format!("{} {}", quoted_column, field.sql_type(driver));

            if !field.nullable {
                col_def.push_str(" NOT NULL");
            }

            if field.unique {
                col_def.push_str(" UNIQUE");
            }

            if let Some(default) = &field.default {
                col_def.push_str(&format!(" DEFAULT {}", default));
            }

            up_statements.push(format!(
                "        schema.raw(r#\"ALTER TABLE {} ADD COLUMN {}\"#).await?;",
                quoted_table, col_def
            ));

            // The trailing space keeps the quoted column from sitting directly against the
            // `"#` that closes the raw string literal.
            down_statements.push(format!(
                "        schema.raw(r#\"ALTER TABLE {} DROP COLUMN {} \"#).await?;",
                quoted_table, quoted_column
            ));
        }

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

    fn parse_fields(fields: Option<&str>) -> Result<Vec<FieldDefinition>, String> {
        let mut parsed_fields = Vec::new();

        if let Some(fields_str) = fields {
            for field in fields_str.split(',') {
                let field = field.trim();
                if field.is_empty() {
                    continue;
                }

                parsed_fields.push(FieldDefinition::parse(field)?);
            }
        }

        Ok(parsed_fields)
    }

    fn build_column_sql(&self, field: &FieldDefinition, driver: &str) -> Result<String, String> {
        // A field carrying the configured primary key name is the table's primary key even
        // without the modifier - the model generator treats it the same way.
        let is_primary_key = field.primary_key || field.name == self.config.model.primary_key;

        if is_primary_key && field.auto_increment {
            return self.auto_increment_primary_key_sql(&field.name, driver);
        }

        let mut col_def = format!(
            "            {} {}",
            self.quote("column", &field.name)?,
            field.sql_type(driver)
        );

        if is_primary_key {
            col_def.push_str(" PRIMARY KEY");
        }

        if field.auto_increment {
            col_def.push_str(self.get_auto_increment(driver));
        }

        if !field.nullable && !is_primary_key {
            col_def.push_str(" NOT NULL");
        }

        if field.unique {
            col_def.push_str(" UNIQUE");
        }

        if let Some(default) = &field.default {
            col_def.push_str(&format!(" DEFAULT {}", default));
        }

        Ok(col_def)
    }

    fn default_primary_key_sql(&self, driver: &str) -> Result<String, String> {
        self.auto_increment_primary_key_sql(&self.config.model.primary_key, driver)
    }

    fn auto_increment_primary_key_sql(
        &self,
        field_name: &str,
        driver: &str,
    ) -> Result<String, String> {
        let column = self.quote("column", field_name)?;

        Ok(match driver {
            "postgres" => format!("            {} BIGSERIAL PRIMARY KEY", column),
            "mysql" => format!("            {} BIGINT PRIMARY KEY AUTO_INCREMENT", column),
            "sqlite" => format!("            {} INTEGER PRIMARY KEY AUTOINCREMENT", column),
            _ => format!("            {} BIGINT PRIMARY KEY", column),
        })
    }

    /// Get auto increment syntax
    fn get_auto_increment(&self, driver: &str) -> &'static str {
        match driver {
            "postgres" => "", // SERIAL types handle this
            "mysql" => " AUTO_INCREMENT",
            "sqlite" => " AUTOINCREMENT",
            _ => "",
        }
    }

    /// Get timestamp type for driver
    fn get_timestamp_type(&self, driver: &str) -> &'static str {
        match driver {
            "postgres" => "TIMESTAMPTZ",
            "mysql" => "DATETIME",
            "sqlite" => "TEXT",
            _ => "TIMESTAMP",
        }
    }

    /// Get NOW() function for driver
    fn get_now_function(&self, driver: &str) -> &'static str {
        match driver {
            "postgres" => "NOW()",
            "mysql" => "NOW()",
            "sqlite" => "CURRENT_TIMESTAMP",
            _ => "NOW()",
        }
    }

    /// Update mod.rs with new migration
    fn update_mod_file(&self, file_name: &str) -> Result<(), String> {
        let mod_path = format!("{}/mod.rs", self.migrations_dir());
        let file_stem = file_name.trim_end_matches(".rs");
        let module_name = migration_module_name(file_stem);

        let existing = std::fs::read_to_string(&mod_path).unwrap_or_default();

        let module_decl = if module_name == file_stem {
            format!("pub mod {};", module_name)
        } else {
            format!("#[path = \"{}\"]\npub mod {};", file_name, module_name)
        };

        if existing.contains(&format!("pub mod {};", module_name)) {
            return Ok(());
        }

        let new_content = format!("{}{}\n", existing, module_decl);

        std::fs::write(&mod_path, new_content)
            .map_err(|e| format!("Failed to update mod.rs: {}", e))?;

        Ok(())
    }
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

        assert!(content.contains("`id` BIGINT PRIMARY KEY AUTO_INCREMENT"));
        assert!(!content.contains("AUTO_INCREMENT PRIMARY KEY AUTO_INCREMENT"));
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

        assert!(content.contains("\"custom_id\" INTEGER PRIMARY KEY AUTOINCREMENT"));
        assert!(!content.contains("\"custom_id\" BIGINT"));
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

        assert!(content.contains("\"id\" INTEGER PRIMARY KEY"));
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
        assert!(content.contains("\"deleted_at\" TIMESTAMPTZ NULL"));
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

        assert!(content.contains("ALTER TABLE `user` ADD COLUMN `order` INTEGER"));
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
}
