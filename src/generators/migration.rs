//! Migration generator for TideORM CLI

use crate::config::TideConfig;
use crate::utils::{ensure_directory, migration_timestamp, to_snake_case, FieldDefinition};

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
        let parsed_fields: Vec<FieldDefinition> = fields
            .as_ref()
            .map(|f| {
                f.split(',')
                    .filter_map(|field| FieldDefinition::parse(field.trim()).ok())
                    .collect()
            })
            .unwrap_or_default();

        // Generate content
        let content = if let Some(table) = create_table {
            self.generate_create_table(&migration_name, &table, &parsed_fields)
        } else if let Some(table) = alter_table {
            self.generate_alter_table(&migration_name, &table, &parsed_fields)
        } else {
            self.generate_empty(&migration_name)
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
        table: &str,
        fields: &[FieldDefinition],
    ) -> String {
        let struct_name = to_pascal_case(name);
        let driver = &self.config.database.driver;

        // Generate columns SQL
        let mut columns = vec![
            format!(
                "            {} {} PRIMARY KEY{}",
                self.config.model.primary_key,
                self.get_pk_type(driver),
                self.get_auto_increment(driver)
            ),
        ];

        for field in fields {
            let mut col_def = format!(
                "            {} {}",
                field.name,
                field.sql_type(driver)
            );

            if !field.nullable {
                col_def.push_str(" NOT NULL");
            }

            if field.unique {
                col_def.push_str(" UNIQUE");
            }

            if let Some(default) = &field.default {
                col_def.push_str(&format!(" DEFAULT {}", default));
            }

            columns.push(col_def);
        }

        // Add timestamps
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

        let columns_sql = columns.join(",\n");

        let mut content = String::new();
        content.push_str(&format!("//! Migration: {}\n", name));
        content.push_str("//!\n");
        content.push_str(&format!("//! Creates the {} table.\n\n", table));
        content.push_str("use tideorm::migration::{Migration, MigrationContext};\n\n");
        content.push_str(&format!("/// Migration: {}\n", name));
        content.push_str(&format!("pub struct {};\n\n", struct_name));
        content.push_str("#[async_trait::async_trait]\n");
        content.push_str(&format!("impl Migration for {} {{\n", struct_name));
        content.push_str("    fn name(&self) -> &'static str {\n");
        content.push_str(&format!("        \"{}\"\n", name));
        content.push_str("    }\n\n");
        content.push_str("    async fn up(&self, ctx: &MigrationContext) -> tideorm::Result<()> {\n");
        content.push_str("        ctx.execute(r#\"\n");
        content.push_str(&format!("        CREATE TABLE IF NOT EXISTS {} (\n", table));
        content.push_str(&columns_sql);
        content.push_str("\n        )\n");
        content.push_str("        \"#).await?;\n");
        content.push_str("        \n");
        content.push_str("        Ok(())\n");
        content.push_str("    }\n\n");
        content.push_str("    async fn down(&self, ctx: &MigrationContext) -> tideorm::Result<()> {\n");
        content.push_str(&format!("        ctx.execute(r#\"DROP TABLE IF EXISTS {}\"#).await?;\n", table));
        content.push_str("        \n");
        content.push_str("        Ok(())\n");
        content.push_str("    }\n");
        content.push_str("}\n");

        content
    }

    /// Generate an alter table migration
    fn generate_alter_table(
        &self,
        name: &str,
        table: &str,
        fields: &[FieldDefinition],
    ) -> String {
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
                "        ctx.execute(r#\"ALTER TABLE {} ADD COLUMN {}\"#).await?;",
                table, col_def
            ));

            down_statements.push(format!(
                "        ctx.execute(r#\"ALTER TABLE {} DROP COLUMN {}\"#).await?;",
                table, field.name
            ));
        }

        let up_sql = up_statements.join("\n");
        let down_sql = down_statements.join("\n");

        let mut content = String::new();
        content.push_str(&format!("//! Migration: {}\n", name));
        content.push_str("//!\n");
        content.push_str(&format!("//! Alters the {} table.\n\n", table));
        content.push_str("use tideorm::migration::{Migration, MigrationContext};\n\n");
        content.push_str(&format!("/// Migration: {}\n", name));
        content.push_str(&format!("pub struct {};\n\n", struct_name));
        content.push_str("#[async_trait::async_trait]\n");
        content.push_str(&format!("impl Migration for {} {{\n", struct_name));
        content.push_str("    fn name(&self) -> &'static str {\n");
        content.push_str(&format!("        \"{}\"\n", name));
        content.push_str("    }\n\n");
        content.push_str("    async fn up(&self, ctx: &MigrationContext) -> tideorm::Result<()> {\n");
        content.push_str(&up_sql);
        content.push_str("\n        Ok(())\n");
        content.push_str("    }\n\n");
        content.push_str("    async fn down(&self, ctx: &MigrationContext) -> tideorm::Result<()> {\n");
        content.push_str(&down_sql);
        content.push_str("\n        Ok(())\n");
        content.push_str("    }\n");
        content.push_str("}\n");

        content
    }

    /// Generate an empty migration
    fn generate_empty(&self, name: &str) -> String {
        let struct_name = to_pascal_case(name);

        let mut content = String::new();
        content.push_str(&format!("//! Migration: {}\n", name));
        content.push_str("//!\n");
        content.push_str("//! TODO: Describe what this migration does.\n\n");
        content.push_str("use tideorm::migration::{Migration, MigrationContext};\n\n");
        content.push_str(&format!("/// Migration: {}\n", name));
        content.push_str(&format!("pub struct {};\n\n", struct_name));
        content.push_str("#[async_trait::async_trait]\n");
        content.push_str(&format!("impl Migration for {} {{\n", struct_name));
        content.push_str("    fn name(&self) -> &'static str {\n");
        content.push_str(&format!("        \"{}\"\n", name));
        content.push_str("    }\n\n");
        content.push_str("    async fn up(&self, ctx: &MigrationContext) -> tideorm::Result<()> {\n");
        content.push_str("        // TODO: Implement the forward migration\n");
        content.push_str("        // Example:\n");
        content.push_str("        // ctx.execute(r#\"\n");
        content.push_str("        //     CREATE TABLE example (\n");
        content.push_str("        //         id BIGSERIAL PRIMARY KEY,\n");
        content.push_str("        //         name VARCHAR(255) NOT NULL\n");
        content.push_str("        //     )\n");
        content.push_str("        // \"#).await?;\n");
        content.push_str("        \n");
        content.push_str("        Ok(())\n");
        content.push_str("    }\n\n");
        content.push_str("    async fn down(&self, ctx: &MigrationContext) -> tideorm::Result<()> {\n");
        content.push_str("        // TODO: Implement the reverse migration\n");
        content.push_str("        // Example:\n");
        content.push_str("        // ctx.execute(r#\"DROP TABLE IF EXISTS example\"#).await?;\n");
        content.push_str("        \n");
        content.push_str("        Ok(())\n");
        content.push_str("    }\n");
        content.push_str("}\n");

        content
    }

    /// Get primary key type for driver
    fn get_pk_type(&self, driver: &str) -> &'static str {
        match driver {
            "postgres" => "BIGSERIAL",
            "mysql" => "BIGINT AUTO_INCREMENT",
            "sqlite" => "INTEGER",
            _ => "BIGINT",
        }
    }

    /// Get auto increment syntax
    fn get_auto_increment(&self, driver: &str) -> &'static str {
        match driver {
            "postgres" => "", // SERIAL types handle this
            "mysql" => "",    // Already in type
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
        let module_name = file_name.trim_end_matches(".rs");

        let existing = std::fs::read_to_string(&mod_path).unwrap_or_default();

        let module_decl = format!("pub mod {};", module_name);
        if existing.contains(&module_decl) {
            return Ok(());
        }

        let struct_name = to_pascal_case(
            module_name
                .split('_')
                .skip(1) // Skip timestamp
                .collect::<Vec<_>>()
                .join("_")
                .as_str(),
        );

        let new_content = format!(
            "{}{}\npub use {}::{};\n",
            existing, module_decl, module_name, struct_name
        );

        std::fs::write(&mod_path, new_content)
            .map_err(|e| format!("Failed to update mod.rs: {}", e))?;

        Ok(())
    }
}

/// Convert string to PascalCase
fn to_pascal_case(s: &str) -> String {
    heck::AsPascalCase(s).to_string()
}
