use super::{SeederGenerator, validate_unique_seed_fields};
use crate::config::TideConfig;

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
        "payload:bytes:unique",
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

    assert!(content.contains("for i in 1u32..=10u32 {"), "{content}");
    assert!(
        content.contains("sku: format!(\"sku-{}\", i),"),
        "{content}"
    );
    assert!(content.contains("price: Decimal::from(i),"), "{content}");
    assert!(content.contains("uid: Uuid::new_v4(),"), "{content}");
    assert!(content.contains("meta: json!({ \"n\": i }),"), "{content}");
    assert!(
        content.contains("payload: (i as u32).to_be_bytes().to_vec(),"),
        "{content}"
    );
    assert!(
        !content.contains("note:"),
        "an optional field is left to Default: {content}"
    );
}

#[test]
fn unique_boolean_seeders_reject_more_rows_than_distinct_values() {
    let config = TideConfig::default();
    let fields = vec![crate::utils::FieldDefinition::parse("active:bool:unique").unwrap()];

    let error = SeederGenerator::new(&config)
        .render("UserSeeder", Some("User".to_string()), 3, &fields)
        .err()
        .unwrap();

    assert!(error.contains("unique values for boolean field 'active'"));
}

#[test]
fn unique_small_integer_seeders_reject_cast_wraparound() {
    let int8 = vec![crate::utils::FieldDefinition::parse("code:i8:unique").unwrap()];
    let int16 = vec![crate::utils::FieldDefinition::parse("code:i16:unique").unwrap()];
    let float = vec![crate::utils::FieldDefinition::parse("code:f32:unique").unwrap()];
    let time = vec![crate::utils::FieldDefinition::parse("at:time:unique").unwrap()];

    assert!(validate_unique_seed_fields(256, &int8).is_ok());
    assert!(validate_unique_seed_fields(257, &int8).is_err());
    assert!(validate_unique_seed_fields(65_536, &int16).is_ok());
    assert!(validate_unique_seed_fields(65_537, &int16).is_err());
    assert!(validate_unique_seed_fields(16_777_216, &float).is_ok());
    assert!(validate_unique_seed_fields(16_777_217, &float).is_err());
    assert!(validate_unique_seed_fields(86_400, &time).is_ok());
    assert!(validate_unique_seed_fields(86_401, &time).is_err());
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
fn model_seeder_has_no_factory_placeholder() {
    // It had a `run_with_factory()` that announced a factory, held a TODO
    // and ran the plain seeder.
    let config = TideConfig::default();
    let generator = SeederGenerator::new(&config);
    let content = generator.generate_model_seeder("UserSeeder", "User", 10, &[]);

    assert!(!content.contains("run_with_factory"));
    assert!(!content.to_lowercase().contains("factory"), "{content}");
}

#[test]
fn model_seeder_imports_from_the_configured_models_path() {
    let mut config = TideConfig::default();
    let generator = SeederGenerator::new(&config);
    let content = generator.generate_model_seeder("UserSeeder", "User", 10, &[]);
    assert!(content.contains("use crate::models::user::User;"));

    config.paths.models = "src/domain/models".to_string();
    let generator = SeederGenerator::new(&config);
    let content = generator.generate_model_seeder("UserSeeder", "User", 10, &[]);

    assert!(content.contains("use crate::domain::models::user::User;"));
    assert!(!content.contains("use crate::models::"));
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
