use super::{ModelInfo, public_models, scan_models};

fn parse_model_file(content: &str) -> Result<Vec<ModelInfo>, syn::Error> {
    Ok(public_models(&syn::parse_file(content)?))
}

#[test]
fn test_parse_tideorm_model_attribute() {
    let content = r#"
#[tideorm::model(table = "posts")]
#[index("user_id")]
pub struct Post {
    pub id: i64,
    pub user_id: i64,
    pub title: String,
}
"#;

    let models = parse_model_file(content).expect("model should parse");
    assert_eq!(models.len(), 1);
    assert_eq!(models[0].name, "Post");
    assert_eq!(models[0].table, "posts");
    assert_eq!(models[0].fields.len(), 3);
}

#[test]
fn test_parse_reads_every_model_of_a_file_with_its_own_options() {
    let content = r#"
#[tideorm::model(table = "posts")]
pub struct Post {
    pub id: i64,
    pub title: String,
    // pub commented_out: String,
}

#[tideorm::model(table = "helpers", soft_delete, translatable = "name", tokenize)]
pub struct Helper {
    pub created_at: String,
    pub updated_at: String,
    pub deleted_at: String,
    pub other: String,
}

impl Post {
    pub fn tokenize(&self) {}
}
"#;

    let models = parse_model_file(content).expect("models should parse");
    assert_eq!(models.len(), 2);
    let post = &models[0];
    assert_eq!(post.name, "Post");
    assert_eq!(post.table, "posts");
    assert_eq!(post.fields, ["id", "title"]);
    assert!(!post.has_timestamps);
    assert!(!post.has_soft_deletes);
    assert!(!post.has_tokenize);
    let helper = &models[1];
    assert_eq!(helper.name, "Helper");
    assert_eq!(helper.table, "helpers");
    assert!(helper.has_timestamps);
    assert!(helper.has_soft_deletes);
    assert!(helper.has_tokenize);
    assert!(helper.has_translatable);
}

#[test]
fn test_attachment_attribute_is_not_reported_as_a_relation() {
    let content = r#"
#[tideorm::model(table = "posts", has_many_files = "photos", translatable = "title", tokenize)]
pub struct Post {
    pub id: i64,
    #[tideorm(belongs_to = "User", foreign_key = "user_id")]
    pub user: BelongsTo<User>,
}
"#;

    let models = parse_model_file(content).expect("model should parse");
    assert_eq!(models[0].relations, ["user"]);
    assert!(models[0].has_translatable);
    assert!(models[0].has_tokenize);
}

#[test]
fn test_every_relation_wrapper_is_reported() {
    let content = r#"
#[tideorm::model(table = "users")]
pub struct User {
    pub id: i64,
    #[tideorm(has_many_through = "Role", pivot = "user_roles", foreign_key = "user_id", related_key = "role_id")]
    pub roles: HasManyThrough<Role, UserRole>,
    pub image: MorphOne<Image>,
    pub parent: SelfRef<User>,
    pub cached: Option<tideorm::relations::HasOne<Profile>>,
    pub tags: Vec<String>,
}
"#;

    let models = parse_model_file(content).expect("model should parse");
    assert_eq!(models[0].relations, ["roles", "image", "parent", "cached"]);
}

#[test]
fn test_parse_derived_model() {
    let content = r#"
#[derive(Clone, Model)]
pub struct User {
    pub id: i64,
}
"#;

    let models = parse_model_file(content).expect("derived model should parse");
    assert_eq!(models[0].name, "User");
    assert_eq!(models[0].table, "users");
    assert_eq!(models[0].fields, ["id"]);
}

#[test]
fn test_parse_prelude_model_attribute_and_inline_modules() {
    let content = r#"
use tideorm::prelude::*;

pub mod admin {
    #[model(table = "admins")]
    pub struct Admin {
        pub id: i64,
    }
}
"#;

    let models = parse_model_file(content).expect("model should parse");
    assert_eq!(models.len(), 1);
    assert_eq!(models[0].name, "Admin");
    assert_eq!(models[0].table, "admins");
}

#[test]
fn test_parse_reports_invalid_rust() {
    assert!(parse_model_file("#[tideorm::model] pub struct Broken {").is_err());
}

#[test]
fn test_scan_reads_mod_rs_and_nested_directories_and_skips_invalid_files() {
    let dir = tempfile::tempdir().unwrap();
    let models = dir.path().join("models");
    std::fs::create_dir_all(models.join("billing")).unwrap();
    std::fs::write(
        models.join("mod.rs"),
        "pub mod billing;\n#[tideorm::model]\npub struct User { pub id: i64 }\n",
    )
    .unwrap();
    std::fs::write(
        models.join("billing").join("invoice.rs"),
        "#[tideorm::model]\npub struct Invoice { pub id: i64 }\n",
    )
    .unwrap();
    std::fs::write(models.join("broken.rs"), "pub struct {").unwrap();

    let found = scan_models(models.to_str().unwrap()).expect("scan should succeed");
    let names: Vec<&str> = found.iter().map(|model| model.name.as_str()).collect();
    assert_eq!(names, ["Invoice", "User"]);
}
