use super::*;
use tideorm::migration::column_type_sql;

#[test]
fn appending_generated_source_separates_declarations_and_preserves_line_endings() {
    assert_eq!(
        append_source_block("pub mod old;", "pub mod new;"),
        "pub mod old;\npub mod new;\n"
    );
    assert_eq!(
        append_source_block("pub mod old;\r\n", "#[path = \"new.rs\"]\npub mod new;"),
        "pub mod old;\r\n#[path = \"new.rs\"]\r\npub mod new;\r\n"
    );
}

#[test]
fn module_declaration_detection_ignores_comments_but_accepts_inline_comments() {
    assert!(!has_module_declaration("// pub mod user;", "user"));
    assert!(has_module_declaration("pub mod user; // models", "user"));
}

#[test]
fn rust_type_names_reject_invalid_and_reserved_identifiers() {
    assert!(validate_rust_type_name("model", "User").is_ok());
    assert!(validate_rust_type_name("model", "123User").is_err());
    assert!(validate_rust_type_name("model", "Self").is_err());
    assert!(validate_rust_type_name("model", "_").is_err());
}

#[test]
fn reading_unusable_source_does_not_treat_it_as_a_new_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mod.rs");
    std::fs::write(&path, [0xff]).unwrap();

    assert!(read_source_or_empty(path.to_str().unwrap()).is_err());
}

#[test]
fn test_pluralize() {
    assert_eq!(pluralize("user"), "users");
    assert_eq!(pluralize("company"), "companies");
    assert_eq!(pluralize("person"), "people");
    assert_eq!(pluralize("box"), "boxes");
    assert_eq!(pluralize("leaf"), "leaves");
}

#[test]
fn test_field_definition_parse() {
    let field = FieldDefinition::parse("name:string").unwrap();
    assert_eq!(field.name, "name");
    assert_eq!(field.field_type, "string");
    assert!(!field.nullable);

    let field = FieldDefinition::parse("age:i32:nullable").unwrap();
    assert_eq!(field.name, "age");
    assert_eq!(field.field_type, "i32");
    assert!(field.nullable);

    let field = FieldDefinition::parse("email:string:unique:indexed").unwrap();
    assert!(field.unique);
    assert!(field.indexed);

    let field = FieldDefinition::parse("id:i64:primary_key:auto_increment").unwrap();
    assert!(field.primary_key);
    assert!(field.auto_increment);
}

#[test]
fn test_default_modifier_preserves_value_case() {
    let field = FieldDefinition::parse("status:string:default='Active'").unwrap();
    assert_eq!(field.default.as_deref(), Some("'Active'"));

    let field = FieldDefinition::parse("created_at:timestamp:DEFAULT=NOW()").unwrap();
    assert_eq!(field.default.as_deref(), Some("NOW()"));
}

/// Commas and colons inside a type's parentheses or a quoted default belong to the field.
#[test]
fn test_field_lists_split_outside_parentheses_and_quotes() {
    assert_eq!(
        split_definitions(
            "price:DECIMAL(10, 2):default='1,5',name:string,tags:text_array",
            ','
        ),
        [
            "price:DECIMAL(10, 2):default='1,5'",
            "name:string",
            "tags:text_array"
        ]
    );

    let field = FieldDefinition::parse("opens:time:default='09:00:00':nullable").unwrap();
    assert_eq!(field.field_type, "time");
    assert_eq!(field.default.as_deref(), Some("'09:00:00'"));
    assert!(field.nullable);

    let field = FieldDefinition::parse("site:string:default='https://example.com'").unwrap();
    assert_eq!(field.default.as_deref(), Some("'https://example.com'"));

    let field = FieldDefinition::parse("price:DECIMAL(10, 2)").unwrap();
    assert_eq!(field.field_type, "DECIMAL(10, 2)");
}

#[test]
fn test_nullable_generic_types_stay_balanced() {
    let field = FieldDefinition::parse("published_at:datetime:nullable").unwrap();
    assert_eq!(
        field.rust_type_for(true),
        "Option<chrono::DateTime<chrono::Utc>>"
    );
    assert_eq!(field.base_rust_type(), "chrono::DateTime<chrono::Utc>");
}

#[test]
fn test_prelude_types_are_emitted_unqualified() {
    assert_eq!(
        FieldDefinition::parse("token:uuid")
            .unwrap()
            .rust_type_for(false),
        "Uuid"
    );
    assert_eq!(
        FieldDefinition::parse("price:decimal")
            .unwrap()
            .rust_type_for(false),
        "Decimal"
    );
}

#[test]
fn test_unknown_field_types_pass_through_but_stay_emittable() {
    // Custom types are a documented escape hatch and must survive untouched...
    let field = FieldDefinition::parse("embedding:vector(1536)").unwrap();
    assert_eq!(field.rust_type_for(false), "vector(1536)");
    assert_eq!(
        column_type_sql(
            &field.migration_column_type("postgres"),
            DatabaseType::Postgres
        ),
        "VECTOR(1536)"
    );

    // ...but a "type" that could terminate a generated string literal is refused.
    let error = FieldDefinition::parse("evil:foo\"# BAD").unwrap_err();
    assert!(error.contains("Invalid field type"));

    let error = FieldDefinition::parse("evil:9lives").unwrap_err();
    assert!(error.contains("must start with a letter"));
}

#[test]
fn test_array_fields_are_postgres_only() {
    let field = FieldDefinition::parse("tags:text_array:nullable").unwrap();
    assert!(field.check_driver_support("postgres").is_ok());
    assert_eq!(
        column_type_sql(
            &field.migration_column_type("postgres"),
            DatabaseType::Postgres
        ),
        "TEXT[]"
    );

    for driver in ["sqlite", "mysql"] {
        let error = field.check_driver_support(driver).unwrap_err();
        assert!(error.contains("only PostgreSQL"), "{}", error);
        assert!(error.contains("`json`"), "{}", error);
    }

    assert!(
        FieldDefinition::parse("meta:json")
            .unwrap()
            .check_driver_support("sqlite")
            .is_ok()
    );
}

#[test]
fn test_column_names_and_defaults_cannot_break_generated_code() {
    let error = FieldDefinition::parse("na\"me:string").unwrap_err();
    assert!(error.contains("Invalid column name"));

    let error = FieldDefinition::parse("title:string:default=\"x\"").unwrap_err();
    assert!(error.contains("Invalid default value"));

    // Ordinary SQL defaults keep working.
    assert_eq!(
        FieldDefinition::parse("status:string:default='Active'")
            .unwrap()
            .default
            .as_deref(),
        Some("'Active'")
    );
}

#[test]
fn test_migration_timestamp_has_sub_second_resolution() {
    let timestamp = migration_timestamp();
    assert_eq!(timestamp.len(), 17, "expected milliseconds: {}", timestamp);
    assert!(
        timestamp
            .chars()
            .all(|character| character.is_ascii_digit())
    );
}

#[test]
fn test_crate_module_path_follows_configured_directories() {
    assert_eq!(crate_module_path("src/models"), "crate::models");
    assert_eq!(
        crate_module_path("./src/domain/models"),
        "crate::domain::models"
    );
    assert_eq!(crate_module_path("src\\models"), "crate::models");
    assert_eq!(crate_module_path("app/models"), "crate::app::models");
    assert_eq!(crate_module_path("src/type"), "crate::r#type");
    assert_eq!(crate_module_path("src"), "crate");
}

/// An unknown driver used to fall through to SQLite, so a `mariadb://` project
/// generated SQLite DDL.
#[test]
fn test_mariadb_and_postgresql_map_to_their_database_types() {
    assert_eq!(database_type("mariadb"), DatabaseType::MySQL);
    assert_eq!(database_type("MySQL"), DatabaseType::MySQL);
    assert_eq!(database_type("postgresql"), DatabaseType::Postgres);
    assert_eq!(database_type("sqlite"), DatabaseType::SQLite);
}

#[test]
fn test_only_integer_keys_auto_increment() {
    for field_type in ["i64", "bigint", "i32", "integer"] {
        assert!(auto_increments(field_type), "{field_type}");
    }
    for field_type in ["uuid", "string", "i16"] {
        assert!(!auto_increments(field_type), "{field_type}");
    }
}

#[test]
fn test_escape_ident_handles_keywords() {
    assert_eq!(escape_ident("email"), "email");
    assert_eq!(escape_ident("type"), "r#type");
    assert_eq!(escape_ident("match"), "r#match");
    assert_eq!(escape_ident("self"), "self_");
    assert_eq!(escape_ident("crate"), "crate_");
}

#[test]
fn test_destructive_confirmation_fails_closed_without_a_terminal() {
    // A prompt that cannot be shown must not be reported as a refusal: the
    // caller would print "Operation cancelled" and exit 0 having done nothing.
    let error = confirm_destructive_with("wipe all tables?", false, || true)
        .expect_err("a missing terminal must be an error");
    assert!(error.contains("wipe all tables?"));
    assert!(error.contains("--force"));

    assert_eq!(
        confirm_destructive_with("wipe all tables?", true, || false),
        Ok(false)
    );
    assert_eq!(
        confirm_destructive_with("wipe all tables?", true, || true),
        Ok(true)
    );
}

#[test]
fn test_relation_definition_parse() {
    let rel = RelationDefinition::parse("posts:has_many:Post").unwrap();
    assert_eq!(rel.name, "posts");
    assert_eq!(rel.relation_type, RelationType::HasMany);
    assert_eq!(rel.related_model, "Post");

    let rel = RelationDefinition::parse("user:belongs_to:User:user_id").unwrap();
    assert_eq!(rel.name, "user");
    assert_eq!(rel.relation_type, RelationType::BelongsTo);
    assert_eq!(rel.foreign_key, Some("user_id".to_string()));

    let lowercase = RelationDefinition::parse("posts:has_many:post").unwrap();
    assert_eq!(lowercase.related_model, "Post");
    assert!(RelationDefinition::parse("posts\"#]:has_many:Post").is_err());
    assert!(RelationDefinition::parse("posts:belongs_to:Post:bad-name").is_err());
}

/// A password holding `?`, `#` or `/` is masked whole; TideORM 0.15.0 cut the URL
/// at the `?` and printed the rest of the password.
#[test]
fn urls_mask_every_spelling_of_a_password() {
    use tideorm::internal::mask_url_credentials;

    for (url, shown) in [
        (
            "postgres://app:p?ss#w/rd@db.internal:5432/app?sslmode=require",
            "postgres://***@db.internal:5432/app?sslmode=require",
        ),
        ("postgres://app:secret@db/app", "postgres://***@db/app"),
        (
            "postgres://db/app?user=app&password=hunter2",
            "postgres://db/app?user=app&password=***",
        ),
        ("sqlite://app.db", "sqlite://app.db"),
    ] {
        assert_eq!(mask_url_credentials(url), shown);
    }

    let url = "mysql://root:p?ss@db.internal/app";
    let error = masked_connection_error(format!("could not reach {url}"), url);
    assert!(!error.contains("ss@"), "{error}");
    assert!(error.contains("mysql://***@db.internal/app"), "{error}");
}
