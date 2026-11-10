//! Seeder generator for TideORM CLI

use crate::config::TideConfig;
use crate::generators::Generated;
use crate::generators::registry::{SEEDERS, register};
use crate::utils::{
    FieldDefinition, crate_module_path, ensure_writable, escape_ident, print_warning,
    to_pascal_case, to_snake_case, validate_rust_type_name,
};

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
        self.render(name, model, count, &[])?.write()
    }

    /// Check and render a seeder without writing anything; its model rows fill `fields`
    /// with a value of each field's type, different on every row so a unique column
    /// accepts them all. Without fields the rows are left to the model's `Default`.
    pub fn render(
        &self,
        name: &str,
        model: Option<String>,
        count: u32,
        fields: &[FieldDefinition],
    ) -> Result<Generated<'_>, String> {
        if name.trim().is_empty() {
            return Err("Seeder name is required".to_string());
        }
        validate_unique_seed_fields(count, fields)?;

        let seeder_name = if name.ends_with("Seeder") {
            to_pascal_case(name)
        } else {
            format!("{}Seeder", to_pascal_case(name))
        };
        validate_rust_type_name("seeder", &seeder_name)?;
        if let Some(model_name) = &model {
            validate_rust_type_name("model", &to_pascal_case(model_name))?;
        }

        let file_name = format!("{}.rs", to_snake_case(&seeder_name));
        let file_path = format!("{}/{}", self.seeders_dir(), file_name);

        let content = if let Some(model_name) = model {
            self.generate_model_seeder(&seeder_name, &model_name, count, fields)
        } else {
            self.generate_basic_seeder(&seeder_name)
        };

        ensure_writable(&file_path)?;

        Ok(Generated::new(file_path, content, move || {
            let module = to_snake_case(&seeder_name);
            if !register_seeder(self.seeders_dir(), &module, &seeder_name)? {
                print_warning(&format!(
                    "{}/mod.rs has no `{}` registry: add `.add({}::{}::default())` to the Seeder your runner uses, or `tideorm db seed` will not run it",
                    self.seeders_dir(),
                    SEEDERS.signature,
                    module,
                    seeder_name
                ));
            }
            Ok(())
        }))
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

        for {index} in 1u32..={count}u32 {{
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
/// from the others' (an `i8` or `i16` wraps, still distinct) while the type has
/// a value per row; [`validate_unique_seed_fields`] refuses a unique field of a
/// type with fewer, such as a `bool` past two rows or an `i8` past 256.
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
        "bytes" | "blob" | "binary" => ("(i as u32).to_be_bytes().to_vec()".to_string(), true),
        _ => return None,
    };
    Some((value, uses_index))
}

fn validate_unique_seed_fields(count: u32, fields: &[FieldDefinition]) -> Result<(), String> {
    if count <= 1 {
        return Ok(());
    }

    for field in fields {
        if !field.unique || field.nullable || field.auto_increment {
            continue;
        }

        match field.field_type.to_lowercase().as_str() {
            "bool" | "boolean" if count > 2 => {
                return Err(format!(
                    "Cannot generate {count} unique values for boolean field '{}'; use a non-boolean type or reduce the seeder count to 2",
                    field.name
                ));
            }
            "bool" | "boolean" => {}
            "i8" | "tinyint" if count > 256 => {
                return Err(format!(
                    "Cannot generate {count} unique values for field '{}' of type '{}'; the type has only 256 distinct values",
                    field.name, field.field_type
                ));
            }
            "i16" | "smallint" if count > 65_536 => {
                return Err(format!(
                    "Cannot generate {count} unique values for field '{}' of type '{}'; the type has only 65536 distinct values",
                    field.name, field.field_type
                ));
            }
            "f32" | "float" if count > 16_777_216 => {
                return Err(format!(
                    "Cannot generate {count} unique values for field '{}' of type '{}'; consecutive integers are exact only through 16777216",
                    field.name, field.field_type
                ));
            }
            "time" if count > 86_400 => {
                return Err(format!(
                    "Cannot generate {count} unique values for time field '{}'; the generated values have only 86400 distinct seconds in a day",
                    field.name
                ));
            }
            _ if seed_value(field).is_none() => {
                return Err(format!(
                    "Cannot generate distinct values for unique field '{}' of type '{}'; provide a supported field type or reduce the seeder count to 1",
                    field.name, field.field_type
                ));
            }
            _ => {}
        }
    }

    Ok(())
}

/// Declare the seeder module `module` in `<dir>/mod.rs` and add its seeder
/// `name` to the `seeders::all()` registry there; see [`register`].
pub(crate) fn register_seeder(dir: &str, module: &str, name: &str) -> Result<bool, String> {
    let entry = format!(".add({module}::{name}::default())");
    register(dir, module, &format!("pub mod {module};"), &SEEDERS, &entry)
}

#[cfg(test)]
#[path = "../../tests/unit/generators_seeder_tests.rs"]
mod tests;
