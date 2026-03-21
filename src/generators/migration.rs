//! Migration generator for TideORM CLI

use crate::config::TideConfig;
use crate::utils::{
    ensure_directory, migration_timestamp, render_template, to_snake_case, FieldDefinition,
};
use serde::Serialize;

/// Migration generator
pub struct MigrationGenerator<'a> {
    config: &'a TideConfig,
}

impl<'a> MigrationGenerator<'a> {
    /// Create a new migration generator
    pub fn new(config: &'a TideConfig) -> Self {
        Self { config }
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
        ensure_directory(&self.config.paths.migrations)?;

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

        let file_path = format!("{}/{}", self.config.paths.migrations, file_name);

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

        // Generate columns SQL
        let mut columns = Vec::new();

        if !fields.iter().any(|field| field.primary_key || field.name == self.config.model.primary_key) {
            columns.push(self.default_primary_key_sql(driver));
        }

        for field in fields {
            columns.push(self.build_column_sql(field, driver));
        }

        // Add timestamps
        if include_timestamps {
            columns.push(format!(
                "            created_at {} NOT NULL DEFAULT {}",
                self.get_timestamp_type(driver),
                self.get_now_function(driver)
            ));
            columns.push(format!(
                "            updated_at {} NOT NULL DEFAULT {}",
                self.get_timestamp_type(driver),
                self.get_now_function(driver)
            ));
        }

        if include_soft_deletes {
            columns.push(format!(
                "            deleted_at {} NULL",
                self.get_timestamp_type(driver)
            ));
        }

        let raw_sql = format!(
            "        CREATE TABLE IF NOT EXISTS {} (\n{}\n        )",
            table,
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
            down_raw_sql: Some(format!("DROP TABLE IF EXISTS {}", table)),
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

        // Generate add column statements
        let mut up_statements = Vec::new();
        let mut down_statements = Vec::new();

        for field in fields {
            let mut col_def = format!("{} {}", field.name, field.sql_type(driver));

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
                table, col_def
            ));

            down_statements.push(format!(
                "        schema.raw(r#\"ALTER TABLE {} DROP COLUMN {}\"#).await?;",
                table, field.name
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
            up_statements: vec![
                "        // TODO: Implement the forward migration".to_string(),
                "        // Example:".to_string(),
                "        // schema.raw(r#\"".to_string(),
                "        //     CREATE TABLE example (".to_string(),
                "        //         id BIGSERIAL PRIMARY KEY,".to_string(),
                "        //         name VARCHAR(255) NOT NULL".to_string(),
                "        //     )".to_string(),
                "        // \"#).await?;".to_string(),
            ],
            down_statements: vec![
                "        // TODO: Implement the reverse migration".to_string(),
                "        // Example:".to_string(),
                "        // schema.raw(r#\"DROP TABLE IF EXISTS example\"#).await?;".to_string(),
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

    fn build_column_sql(&self, field: &FieldDefinition, driver: &str) -> String {
        if field.primary_key && field.auto_increment {
            return self.auto_increment_primary_key_sql(&field.name, driver);
        }

        let mut col_def = format!("            {} {}", field.name, field.sql_type(driver));

        if field.primary_key {
            col_def.push_str(" PRIMARY KEY");
        }

        if field.auto_increment {
            col_def.push_str(self.get_auto_increment(driver));
        }

        if !field.nullable && !field.primary_key {
            col_def.push_str(" NOT NULL");
        }

        if field.unique {
            col_def.push_str(" UNIQUE");
        }

        if let Some(default) = &field.default {
            col_def.push_str(&format!(" DEFAULT {}", default));
        }

        col_def
    }

    fn default_primary_key_sql(&self, driver: &str) -> String {
        self.auto_increment_primary_key_sql(&self.config.model.primary_key, driver)
    }

    fn auto_increment_primary_key_sql(&self, field_name: &str, driver: &str) -> String {
        match driver {
            "postgres" => format!("            {} BIGSERIAL PRIMARY KEY", field_name),
            "mysql" => format!("            {} BIGINT PRIMARY KEY AUTO_INCREMENT", field_name),
            "sqlite" => format!("            {} INTEGER PRIMARY KEY AUTOINCREMENT", field_name),
            _ => format!("            {} BIGINT PRIMARY KEY", field_name),
        }
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
        let mod_path = format!("{}/mod.rs", self.config.paths.migrations);
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
{% if down_mode == "raw_sql" %}        schema.raw(r#"{{ down_raw_sql }}"#).await?;
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
        file_stem.to_string()
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
        let content = generator.generate_create_table(
            "create_users_table",
            "20260316_001",
            "users",
            &[],
            false,
            false,
        ).unwrap();

        assert!(content.contains("id BIGINT PRIMARY KEY AUTO_INCREMENT"));
        assert!(!content.contains("AUTO_INCREMENT PRIMARY KEY AUTO_INCREMENT"));
    }

    #[test]
    fn test_sqlite_explicit_auto_increment_primary_key_uses_integer() {
        let mut config = TideConfig::default();
        config.database.driver = "sqlite".to_string();
        config.migration.timestamps = false;

        let generator = MigrationGenerator::new(&config);
        let fields = vec![FieldDefinition::parse("custom_id:i64:primary_key:auto_increment").unwrap()];
        let content = generator.generate_create_table(
            "create_users_table",
            "20260316_001",
            "users",
            &fields,
            false,
            false,
        ).unwrap();

        assert!(content.contains("custom_id INTEGER PRIMARY KEY AUTOINCREMENT"));
        assert!(!content.contains("custom_id BIGINT"));
    }

    #[test]
    fn test_timestamped_migration_module_name_is_sanitized() {
        assert_eq!(
            migration_module_name("20260316203329_create_posts_table"),
            "m_20260316203329_create_posts_table"
        );
        assert_eq!(migration_module_name("create_posts_table"), "create_posts_table");
    }

    #[test]
    fn test_migration_template_override_is_used() {
        let dir = tempdir().unwrap();
        let template_path = dir.path().join("migration.rs.j2");
        std::fs::write(&template_path, "// custom migration {{ name }} {{ description }}\n").unwrap();

        let mut config = TideConfig::default();
        config.migration.template = Some(template_path.to_string_lossy().into_owned());
        config.migration.timestamps = false;

        let generator = MigrationGenerator::new(&config);
        let content = generator
            .generate_create_table("create_users_table", "20260316_001", "users", &[], false, false)
            .unwrap();

        assert_eq!(content, "// custom migration create_users_table Creates the users table.");
    }
}
