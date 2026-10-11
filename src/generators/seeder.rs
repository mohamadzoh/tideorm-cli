//! Seeder generator for TideORM CLI

use crate::config::TideConfig;
use crate::utils::{
    FieldDefinition, crate_module_path, ensure_directory, ensure_writable, escape_ident,
    print_warning, to_snake_case,
};

/// The first line of the registry `tideorm db seed` runs: every seeder the
/// runner knows about is an `.add(..)` in the chain that opens with
/// `Seeder::new()` on the next line.
pub(crate) const SEEDER_REGISTRY: &str = "pub fn all() -> Seeder {";

/// Initial `mod.rs` of a seeders directory: an empty registry.
pub(crate) const SEEDERS_MOD_RS: &str = r#"//! Database seeders

use tideorm::prelude::*;

/// The seeders `tideorm db seed` runs. `tideorm make seeder` registers new ones
/// here, and the runner orders them by their `priority()` and `depends_on()`.
pub fn all() -> Seeder {
    Seeder::new()
}
"#;

/// Seeder generator
pub struct SeederGenerator<'a> {
    config: &'a TideConfig,
    output_dir: Option<String>,
}

impl<'a> SeederGenerator<'a> {
    /// Create a new seeder generator
    pub fn new(config: &'a TideConfig) -> Self {
        Self {
            config,
            output_dir: None,
        }
    }

    /// Override the directory seeders are written to (the `--output` flag).
    ///
    /// `None` keeps the configured `[paths] seeders` directory.
    ///
    /// This only moves the generated file: the model import is still derived from
    /// `[paths] models`, so relocating a seeder does not break it.
    pub fn output_dir(mut self, dir: Option<&str>) -> Self {
        self.output_dir = dir
            .map(str::trim)
            .filter(|dir| !dir.is_empty())
            .map(ToOwned::to_owned);
        self
    }

    /// Directory the generated seeder and its `mod.rs` belong to.
    fn seeders_dir(&self) -> &str {
        self.output_dir
            .as_deref()
            .unwrap_or(&self.config.paths.seeders)
    }

    /// Generate a seeder file
    pub fn generate(
        &self,
        name: &str,
        model: Option<String>,
        count: u32,
    ) -> Result<String, String> {
        self.generate_with_fields(name, model, count, &[])
    }

    /// Generate a seeder whose model rows fill `fields` with a value of each
    /// field's type, different on every row so a unique column accepts them all.
    /// Without fields the rows are left to the model's `Default`.
    pub fn generate_with_fields(
        &self,
        name: &str,
        model: Option<String>,
        count: u32,
        fields: &[FieldDefinition],
    ) -> Result<String, String> {
        ensure_directory(self.seeders_dir())?;

        let seeder_name = if name.ends_with("Seeder") {
            to_pascal_case(name)
        } else {
            format!("{}Seeder", to_pascal_case(name))
        };

        let file_name = format!("{}.rs", to_snake_case(&seeder_name));
        let file_path = format!("{}/{}", self.seeders_dir(), file_name);

        let content = if let Some(model_name) = model {
            self.generate_model_seeder(&seeder_name, &model_name, count, fields)
        } else {
            self.generate_basic_seeder(&seeder_name)
        };

        ensure_writable(&file_path)?;

        std::fs::write(&file_path, content)
            .map_err(|e| format!("Failed to write seeder file: {}", e))?;

        let module = to_snake_case(&seeder_name);
        if !register_seeder(self.seeders_dir(), &module, &seeder_name)? {
            print_warning(&format!(
                "{}/mod.rs has no `{}` registry: add `.add({}::{}::default())` to the Seeder your runner uses, or `tideorm db seed` will not run it",
                self.seeders_dir(),
                SEEDER_REGISTRY,
                module,
                seeder_name
            ));
        }

        Ok(file_path)
    }

    /// Generate a seeder for a specific model
    ///
    /// The model is imported through the configured `[paths] models` directory rather than
    /// a hardcoded `crate::models`, so a project that keeps its models elsewhere still gets
    /// a seeder that compiles.
    fn generate_model_seeder(
        &self,
        seeder_name: &str,
        model_name: &str,
        count: u32,
        fields: &[FieldDefinition],
    ) -> String {
        let model_snake = to_snake_case(model_name);
        let model_pascal = to_pascal_case(model_name);
        let assignments: Vec<(String, bool)> = fields
            .iter()
            .filter_map(|field| {
                let (value, uses_index) = seed_value(field)?;
                Some((
                    format!("{}: {},", escape_ident(&field.name), value),
                    uses_index,
                ))
            })
            .collect();
        // Named `_i` unless a value reads it, so the scaffold compiles without warnings.
        let index = if assignments.iter().any(|(_, uses_index)| *uses_index) {
            "i"
        } else {
            "_i"
        };
        let field_lines = if assignments.is_empty() {
            format!(
                "// TODO: Fill in the model fields\n                \
                 // Example:\n                \
                 // name: format!(\"{model_pascal} {{}}\", {index}),\n                \
                 // email: format!(\"{model_snake}{{}}@example.com\", {index}),"
            )
        } else {
            assignments
                .iter()
                .map(|(line, _)| line.as_str())
                .collect::<Vec<_>>()
                .join("\n                ")
        };
        let model_module = escape_ident(&model_snake);
        let models_path = crate_module_path(&self.config.paths.models);
        let factories_path = crate_module_path(&self.config.paths.factories);

        format!(
            r#"//! {} Seeder
//!
//! Seeds the database with {} records.

use tideorm::prelude::*;
use {models_path}::{model_module}::{model_pascal};

/// {} seeder
#[derive(Default)]
pub struct {seeder_name};

#[async_trait]
impl Seed for {seeder_name} {{
    fn name(&self) -> &str {{
        "{seeder_snake}"
    }}

    async fn run(&self, _db: &Database) -> tideorm::Result<()> {{
        println!("Seeding {model_snake}s...");

        for {index} in 1..={count} {{
            let record = {model_pascal} {{
                {field_lines}
                ..Default::default()
            }};

            record.save().await?;
        }}

        println!("Seeded {count} {model_snake}(s)");
        Ok(())
    }}
}}

impl {seeder_name} {{
    /// Run the seeder with a factory
    pub async fn run_with_factory() -> tideorm::Result<()> {{
        println!("Seeding {model_snake}s with factory...");

        // TODO: Use factory pattern
        // Example:
        // {factories_path}::{model_snake}_factory::{model_pascal}Factory::create_many({count}).await?;

        Self::default().run(db()).await
    }}
}}
"#,
            seeder_name,
            model_name,
            model_name,
            seeder_name = seeder_name,
            seeder_snake = to_snake_case(seeder_name),
            model_pascal = model_pascal,
            model_snake = model_snake,
            model_module = model_module,
            models_path = models_path,
            factories_path = factories_path,
            count = count,
            index = index,
            field_lines = field_lines,
        )
    }

    /// Generate a basic seeder
    fn generate_basic_seeder(&self, seeder_name: &str) -> String {
        format!(
            r#"//! {} Seeder
//!
//! Custom database seeder.

use tideorm::prelude::*;

/// {}
#[derive(Default)]
pub struct {};

#[async_trait]
impl Seed for {} {{
    fn name(&self) -> &str {{
        "{}"
    }}

    async fn run(&self, _db: &Database) -> tideorm::Result<()> {{
        println!("Running {}...");

        // TODO: Add your seeding logic here
        // Example:
        // 
        // let user = User {{
        //     name: "Admin".to_string(),
        //     email: "admin@example.com".to_string(),
        //     ..Default::default()
        // }};
        // user.save().await?;

        println!("{} completed!");
        Ok(())
    }}
}}
"#,
            seeder_name,
            seeder_name,
            seeder_name,
            seeder_name,
            to_snake_case(seeder_name),
            seeder_name,
            seeder_name
        )
    }
}

/// A value for `field` on seeder row `i`, and whether it reads `i`. `None`
/// leaves the field to the model's `Default`: optional and auto-increment
/// fields, arrays, and types the CLI does not model. Every row's value differs
/// from the others', so a unique column takes them all, except for a `bool`,
/// which has two.
fn seed_value(field: &FieldDefinition) -> Option<(String, bool)> {
    if field.nullable || field.auto_increment {
        return None;
    }
    let (value, uses_index) = match field.field_type.to_lowercase().as_str() {
        "string" | "varchar" | "text" => (format!("format!(\"{}-{{}}\", i)", field.name), true),
        "i8" | "tinyint" => ("i as i8".to_string(), true),
        "i16" | "smallint" => ("i as i16".to_string(), true),
        "i32" | "int" | "integer" => ("i as i32".to_string(), true),
        "i64" | "bigint" => ("i as i64".to_string(), true),
        "f32" | "float" => ("i as f32".to_string(), true),
        "f64" | "double" => ("f64::from(i)".to_string(), true),
        "bool" | "boolean" => ("i % 2 == 0".to_string(), true),
        "decimal" => ("Decimal::from(i)".to_string(), true),
        "uuid" => ("Uuid::new_v4()".to_string(), false),
        "datetime" | "timestamp" => (
            "tideorm::chrono::Utc::now() + tideorm::chrono::TimeDelta::seconds(i64::from(i))"
                .to_string(),
            true,
        ),
        "date" => (
            "tideorm::chrono::Utc::now().date_naive() + tideorm::chrono::Days::new(i as u64)"
                .to_string(),
            true,
        ),
        "time" => (
            "tideorm::chrono::Utc::now().time() + tideorm::chrono::TimeDelta::seconds(i64::from(i))"
                .to_string(),
            true,
        ),
        "json" | "jsonb" => ("json!({ \"n\": i })".to_string(), true),
        "bytes" | "blob" | "binary" => ("vec![i as u8]".to_string(), true),
        _ => return None,
    };
    Some((value, uses_index))
}

/// Declare `module` in `<dir>/mod.rs` and add `module::name` to its `all()`
/// registry.
///
/// Returns `false`, leaving the registry alone, when the file has no registry
/// in the shape [`SEEDERS_MOD_RS`] scaffolds: editing a hand-written one could
/// break it.
pub(crate) fn register_seeder(dir: &str, module: &str, name: &str) -> Result<bool, String> {
    let mod_path = format!("{}/mod.rs", dir);
    let source = std::fs::read_to_string(&mod_path).unwrap_or_default();
    let mut lines: Vec<String> = source.lines().map(str::to_string).collect();

    let declaration = format!("pub mod {};", module);
    if !lines.iter().any(|line| line.trim() == declaration) {
        match lines
            .iter()
            .rposition(|line| line.trim_start().starts_with("pub mod "))
        {
            Some(last) => lines.insert(last + 1, declaration),
            None => {
                // After the `//!` header and its blank line, set off by another.
                let header = lines
                    .iter()
                    .take_while(|line| line.starts_with("//!"))
                    .count();
                let at = header + usize::from(lines.get(header).is_some_and(|l| l.is_empty()));
                lines.insert(at, declaration);
                if lines.get(at + 1).is_some_and(|line| !line.is_empty()) {
                    lines.insert(at + 1, String::new());
                }
            }
        }
    }

    let entry = format!(".add({}::{}::default())", module, name);
    let registry = lines
        .iter()
        .position(|line| line.trim() == SEEDER_REGISTRY)
        .filter(|&start| {
            lines
                .get(start + 1)
                .is_some_and(|l| l.trim() == "Seeder::new()")
        });
    if let Some(start) = registry
        && !lines.iter().any(|line| line.trim() == entry)
    {
        let mut at = start + 2;
        while lines
            .get(at)
            .is_some_and(|l| l.trim_start().starts_with(".add("))
        {
            at += 1;
        }
        lines.insert(at, format!("        {}", entry));
    }

    let newline = if source.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    std::fs::write(&mod_path, lines.join(newline) + newline)
        .map_err(|e| format!("Failed to update mod.rs: {}", e))?;

    Ok(registry.is_some())
}

/// Convert string to PascalCase
fn to_pascal_case(s: &str) -> String {
    heck::AsPascalCase(s).to_string()
}

#[cfg(test)]
mod tests {
    use super::{SEEDERS_MOD_RS, SeederGenerator, register_seeder};
    use crate::config::TideConfig;

    #[test]
    fn register_seeder_declares_and_registers_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let seeders = dir.path().to_string_lossy().into_owned();
        std::fs::write(dir.path().join("mod.rs"), SEEDERS_MOD_RS).unwrap();

        assert!(register_seeder(&seeders, "database_seeder", "DatabaseSeeder").unwrap());
        assert!(register_seeder(&seeders, "user_seeder", "UserSeeder").unwrap());
        assert!(register_seeder(&seeders, "user_seeder", "UserSeeder").unwrap());

        let content = std::fs::read_to_string(dir.path().join("mod.rs")).unwrap();
        assert_eq!(
            content,
            "//! Database seeders\n\npub mod database_seeder;\npub mod user_seeder;\n\nuse tideorm::prelude::*;\n\n\
             /// The seeders `tideorm db seed` runs. `tideorm make seeder` registers new ones\n\
             /// here, and the runner orders them by their `priority()` and `depends_on()`.\n\
             pub fn all() -> Seeder {\n    Seeder::new()\n        .add(database_seeder::DatabaseSeeder::default())\n        .add(user_seeder::UserSeeder::default())\n}\n"
        );
    }

    #[test]
    fn register_seeder_leaves_a_hand_written_mod_rs_registry_alone() {
        let dir = tempfile::tempdir().unwrap();
        let seeders = dir.path().to_string_lossy().into_owned();
        std::fs::write(dir.path().join("mod.rs"), "//! Database seeders\n").unwrap();

        assert!(!register_seeder(&seeders, "user_seeder", "UserSeeder").unwrap());

        let content = std::fs::read_to_string(dir.path().join("mod.rs")).unwrap();
        assert_eq!(content, "//! Database seeders\npub mod user_seeder;\n");
    }

    /// Default-valued rows repeat the same value in every unique column, so the
    /// second insert of a generated seeder failed.
    #[test]
    fn model_seeders_fill_each_field_per_row() {
        let config = TideConfig::default();
        let fields: Vec<crate::utils::FieldDefinition> = [
            "sku:string:unique",
            "price:decimal",
            "uid:uuid",
            "meta:json",
            "note:string:nullable",
        ]
        .iter()
        .map(|field| crate::utils::FieldDefinition::parse(field).unwrap())
        .collect();
        let content = SeederGenerator::new(&config).generate_model_seeder(
            "ProductSeeder",
            "Product",
            10,
            &fields,
        );

        assert!(content.contains("for i in 1..=10 {"), "{content}");
        assert!(
            content.contains("sku: format!(\"sku-{}\", i),"),
            "{content}"
        );
        assert!(content.contains("price: Decimal::from(i),"), "{content}");
        assert!(content.contains("uid: Uuid::new_v4(),"), "{content}");
        assert!(content.contains("meta: json!({ \"n\": i }),"), "{content}");
        assert!(
            !content.contains("note:"),
            "an optional field is left to Default: {content}"
        );
    }

    #[test]
    fn seeder_name_matches_the_seeder_not_the_model() {
        // `tideorm db seed --seeder AdminSeeder` looks the seed up as `admin_seeder`.
        let config = TideConfig::default();
        let content =
            SeederGenerator::new(&config).generate_model_seeder("AdminSeeder", "User", 1, &[]);
        assert!(content.contains("\"admin_seeder\""));
    }

    #[test]
    fn model_seeder_uses_global_db_helper_without_double_reference() {
        let config = TideConfig::default();
        let generator = SeederGenerator::new(&config);
        let content = generator.generate_model_seeder("UserSeeder", "User", 10, &[]);

        assert!(content.contains("Self::default().run(db()).await"));
        assert!(!content.contains("run(&db())"));
    }

    #[test]
    fn model_seeder_imports_from_the_configured_models_path() {
        let mut config = TideConfig::default();
        let generator = SeederGenerator::new(&config);
        let content = generator.generate_model_seeder("UserSeeder", "User", 10, &[]);
        assert!(content.contains("use crate::models::user::User;"));

        config.paths.models = "src/domain/models".to_string();
        config.paths.factories = "src/domain/factories".to_string();
        let generator = SeederGenerator::new(&config);
        let content = generator.generate_model_seeder("UserSeeder", "User", 10, &[]);

        assert!(content.contains("use crate::domain::models::user::User;"));
        assert!(!content.contains("use crate::models::"));
        assert!(content.contains("// crate::domain::factories::user_factory::UserFactory"));
    }

    #[test]
    fn seeder_output_dir_overrides_the_configured_path() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("custom").to_string_lossy().into_owned();

        let mut config = TideConfig::default();
        config.paths.seeders = dir.path().join("configured").to_string_lossy().into_owned();

        let path = SeederGenerator::new(&config)
            .output_dir(Some(&output))
            .generate("Demo", None, 1)
            .unwrap();

        assert!(path.starts_with(&output), "{}", path);
    }
}
