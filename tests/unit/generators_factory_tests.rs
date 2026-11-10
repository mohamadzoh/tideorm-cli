use super::FactoryGenerator;
use crate::config::TideConfig;

#[test]
fn factory_imports_from_the_configured_models_path() {
    let mut config = TideConfig::default();
    let generator = FactoryGenerator::new(&config);
    let content = generator.generate_factory("UserFactory", "User");
    assert!(content.contains("use crate::models::user::User;"));

    config.paths.models = "src/domain/models".to_string();
    let generator = FactoryGenerator::new(&config);
    let content = generator.generate_factory("UserFactory", "User");

    assert!(content.contains("use crate::domain::models::user::User;"));
    assert!(!content.contains("use crate::models::"));
}

/// The record was bound to the model's snake-cased name, which for `Match` is
/// `let mut match`, and the factory did not compile.
#[test]
fn factory_for_a_keyword_named_model_compiles() {
    let config = TideConfig::default();
    let content = FactoryGenerator::new(&config).generate_factory("MatchFactory", "Match");

    assert!(content.contains("use crate::models::r#match::Match;"));
    assert!(content.contains("let mut record = Self::definition();"));
    assert!(!content.contains("let mut match"), "{content}");
}

#[test]
fn factory_output_dir_overrides_the_configured_path() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("custom").to_string_lossy().into_owned();

    let mut config = TideConfig::default();
    config.paths.factories = dir.path().join("configured").to_string_lossy().into_owned();

    let path = FactoryGenerator::new(&config)
        .output_dir(Some(&output))
        .generate("Demo", None)
        .unwrap();

    assert!(path.starts_with(&output), "{}", path);
}
