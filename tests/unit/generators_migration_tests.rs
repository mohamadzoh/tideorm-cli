use super::*;
use tempfile::tempdir;

fn config(driver: &str) -> TideConfig {
    let mut config = TideConfig::default();
    config.database.driver = driver.to_string();
    config.migration.timestamps = false;
    config
}

fn fields(fields: &[&str]) -> Vec<FieldDefinition> {
    fields
        .iter()
        .map(|field| FieldDefinition::parse(field).unwrap())
        .collect()
}

fn create_users_table(config: &TideConfig, field_list: &[&str]) -> String {
    MigrationGenerator::new(config)
        .generate_create_table(
            "create_users_table",
            "1",
            "users",
            &fields(field_list),
            false,
            false,
        )
        .unwrap()
}

fn alter_users_table(config: &TideConfig, field_list: &[&str]) -> String {
    MigrationGenerator::new(config)
        .generate_alter_table("add_to_users", "1", "users", &fields(field_list))
        .unwrap()
}

/// The table is declared through the builder, which TideORM renders for the
/// database the migration runs on.
#[test]
fn test_a_create_migration_declares_its_table_through_the_builder() {
    let content = create_users_table(&config("postgres"), &["name:string"]);

    assert!(
        content.contains(
            "    async fn up(&self, schema: &mut Schema) -> tideorm::Result<()> {\n        schema\n            .create_table_if_not_exists(\"users\", |t| {\n                t.big_increments(\"id\");\n                t.string(\"name\").not_null();\n            })\n            .await?;\n\n        Ok(())\n    }"
        ),
        "{content}"
    );
    assert!(
        content.contains(
            "    async fn down(&self, schema: &mut Schema) -> tideorm::Result<()> {\n        schema.drop_table_if_exists(\"users\").await?;\n\n        Ok(())\n    }"
        ),
        "{content}"
    );
    assert!(!content.contains("schema.raw"), "{content}");
}

#[test]
fn test_default_mysql_primary_key_is_numbered_once() {
    let content = create_users_table(&config("mysql"), &[]);

    assert!(content.contains("t.big_increments(\"id\");"), "{content}");
    assert_eq!(content.matches("increments").count(), 1, "{content}");
}

#[test]
fn test_an_explicit_auto_increment_primary_key_replaces_the_default_one() {
    let content = create_users_table(
        &config("sqlite"),
        &["custom_id:i64:primary_key:auto_increment"],
    );

    assert!(
        content.contains("t.big_increments(\"custom_id\");"),
        "{content}"
    );
    assert!(!content.contains("\"id\""), "{content}");
}

#[test]
fn test_auto_increment_is_refused_on_a_column_that_is_not_the_key() {
    let error = MigrationGenerator::new(&config("sqlite"))
        .generate_create_table(
            "create_counters_table",
            "1",
            "counters",
            &fields(&["counter:i64:auto_increment"]),
            false,
            false,
        )
        .unwrap_err();

    assert!(error.contains("only a primary key column"), "{error}");
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
    let content = create_users_table(&config("postgres"), &["id:i32"]);

    assert!(
        content.contains("t.integer(\"id\").primary_key();"),
        "{content}"
    );
    assert!(!content.contains("increments"), "{content}");
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

    let mut config = config("postgres");
    config.migration.template = Some(template_path.to_string_lossy().into_owned());

    assert_eq!(
        create_users_table(&config, &[]),
        "// custom migration create_users_table Creates the users table."
    );
}

#[test]
fn test_reserved_words_are_left_to_the_builder_to_quote() {
    let content = MigrationGenerator::new(&config("postgres"))
        .generate_create_table(
            "create_orders_table",
            "1",
            "order",
            &fields(&["order:i32", "user:string"]),
            true,
            true,
        )
        .unwrap();

    assert!(
        content.contains(".create_table_if_not_exists(\"order\", |t| {"),
        "{content}"
    );
    assert!(content.contains("t.integer(\"order\").not_null();"));
    assert!(content.contains("t.string(\"user\").not_null();"));
    assert!(content.contains("t.timestamps();"));
    assert!(content.contains("t.soft_deletes();"));
    assert!(content.contains("schema.drop_table_if_exists(\"order\").await?;"));
}

#[test]
fn test_an_alter_migration_adds_and_drops_its_columns() {
    let content = MigrationGenerator::new(&config("mysql"))
        .generate_alter_table(
            "add_order_to_users",
            "1",
            "user",
            &fields(&["order:i32:nullable"]),
        )
        .unwrap();

    let up = &content[..content.find("async fn down").unwrap()];
    let down = &content[content.find("async fn down").unwrap()..];
    assert!(up.contains(".alter_table(\"user\", |t| {"), "{content}");
    assert!(up.contains("t.integer(\"order\").nullable();"), "{content}");
    assert!(down.contains(".alter_table(\"user\", |t| {"), "{content}");
    assert!(down.contains("t.drop_column(\"order\");"), "{content}");
}

/// Defaults and types without a builder method keep their SQL, written as
/// Rust string literals.
#[test]
fn test_defaults_and_custom_types_are_rust_literals() {
    let content = create_users_table(
        &config("postgres"),
        &[
            "embedding:vector(1536)",
            "price:decimal:default=0",
            "tags:bool_array:nullable",
            "label:string:default='draft'",
        ],
    );

    assert!(
        content
            .contains("t.column(\"embedding\", ColumnType::custom(\"VECTOR(1536)\")).not_null();"),
        "{content}"
    );
    assert!(
        content.contains(
            "t.decimal_with(\"price\", 19, 4).not_null().default(DefaultValue::Raw(\"0\".into()));"
        ),
        "{content}"
    );
    assert!(
        content.contains("t.column(\"tags\", ColumnType::custom(\"BOOLEAN[]\")).nullable();"),
        "{content}"
    );
    assert!(
        content.contains(
            "t.string(\"label\").not_null().default(DefaultValue::Raw(\"'draft'\".into()));"
        ),
        "{content}"
    );
}

#[test]
fn test_table_names_are_validated_before_they_are_written() {
    let error = MigrationGenerator::new(&TideConfig::default())
        .generate_create_table(
            "create_evil_table",
            "1",
            "t\"#; fn evil() {}",
            &[],
            false,
            false,
        )
        .unwrap_err();
    assert!(error.contains("Invalid table name"), "{error}");
}

#[test]
fn test_an_empty_migration_is_a_stub() {
    let content = MigrationGenerator::new(&TideConfig::default())
        .generate_empty("do_something", "1")
        .unwrap();

    assert!(!content.contains("schema.raw"), "{content}");
    assert!(content.contains("// TODO: Implement the forward migration."));
    assert!(content.contains("let _ = schema;"));
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

    let error = MigrationGenerator::new(&config)
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
fn unsupported_drivers_do_not_generate_migrations() {
    let dir = tempdir().unwrap();
    let output = dir.path().join("migrations");

    let error = MigrationGenerator::new(&config("oracle"))
        .output_dir(Some(output.to_str().unwrap()))
        .generate(
            "create_users_table",
            Some("users".to_string()),
            None,
            None,
            false,
            false,
        )
        .unwrap_err();

    assert!(error.contains("Unsupported database driver"));
    assert!(!output.exists());
}

#[test]
fn conflicting_migration_targets_are_rejected_before_creating_output() {
    let dir = tempdir().unwrap();
    let output = dir.path().join("migrations");
    let config = TideConfig::default();

    let error = MigrationGenerator::new(&config)
        .output_dir(Some(output.to_str().unwrap()))
        .generate(
            "change_users",
            Some("users".to_string()),
            Some("users".to_string()),
            None,
            false,
            false,
        )
        .unwrap_err();

    assert!(error.contains("cannot create and alter"));
    assert!(!output.exists());
}

#[test]
fn fields_without_a_migration_target_are_rejected_before_creating_output() {
    let dir = tempdir().unwrap();
    let output = dir.path().join("migrations");
    let config = TideConfig::default();

    let error = MigrationGenerator::new(&config)
        .output_dir(Some(output.to_str().unwrap()))
        .generate(
            "change_users",
            None,
            None,
            Some("name:string".to_string()),
            false,
            false,
        )
        .unwrap_err();

    assert!(error.contains("fields require --create or --table"));
    assert!(!output.exists());
}

#[test]
fn invalid_migration_type_names_are_rejected_before_creating_output() {
    let dir = tempdir().unwrap();
    let output = dir.path().join("migrations");
    let config = TideConfig::default();

    for name in ["", "123", "self"] {
        let error = MigrationGenerator::new(&config)
            .output_dir(Some(output.to_str().unwrap()))
            .generate(name, None, None, None, false, false)
            .unwrap_err();

        assert!(error.contains("migration"), "{error}");
        assert!(!output.exists());
    }
}

/// Without timestamps, a migration made after another runs after it,
/// whatever their names: `add_age_to_users` sorts before
/// `create_users_table`. Each is registered in `migrations::all()` in the
/// order it was made.
#[test]
fn test_migrations_without_timestamps_keep_the_order_they_were_made_in() {
    let dir = tempdir().unwrap();
    std::fs::write(dir.path().join("mod.rs"), MIGRATIONS_MOD_RS).unwrap();
    let mut config = config("sqlite");
    config.paths.migrations = dir.path().to_string_lossy().into_owned();
    let generator = MigrationGenerator::new(&config);

    let created = generator
        .generate(
            "create_users_table",
            Some("users".to_string()),
            None,
            None,
            false,
            false,
        )
        .unwrap();
    let altered = generator
        .generate(
            "add_age_to_users",
            None,
            Some("users".to_string()),
            Some("age:i32".to_string()),
            false,
            false,
        )
        .unwrap();

    assert!(
        created.ends_with("/0001_create_users_table.rs"),
        "{created}"
    );
    assert!(altered.ends_with("/0002_add_age_to_users.rs"), "{altered}");
    let altered = std::fs::read_to_string(altered).unwrap();
    assert!(altered.contains("\"0002\""), "{altered}");

    let registry = std::fs::read_to_string(dir.path().join("mod.rs")).unwrap();
    assert!(
        registry.contains(
            "#[path = \"0001_create_users_table.rs\"]\npub mod m_0001_create_users_table;\n#[path = \"0002_add_age_to_users.rs\"]\npub mod m_0002_add_age_to_users;\n"
        ),
        "{registry}"
    );
    assert!(
        registry.contains(
            "    Migrator::new()\n        .add(m_0001_create_users_table::CreateUsersTable)\n        .add(m_0002_add_age_to_users::AddAgeToUsers)\n}"
        ),
        "{registry}"
    );
}

/// A table in another schema (a PostgreSQL schema, a MySQL database) is
/// named alike by every statement; the builder splits the name.
#[test]
fn test_schema_qualified_tables_are_named_alike_everywhere() {
    let config = config("postgres");
    let generator = MigrationGenerator::new(&config);
    let fields = fields(&["email:string:indexed"]);

    let created = generator
        .generate_create_table(
            "create_users_table",
            "1",
            "app.users",
            &fields,
            false,
            false,
        )
        .unwrap();
    assert!(
        created.contains(".create_table_if_not_exists(\"app.users\", |t| {"),
        "{created}"
    );
    assert!(
        created.contains("schema.drop_table_if_exists(\"app.users\")"),
        "{created}"
    );

    let altered = generator
        .generate_alter_table("add_email_to_users", "2", "app.users", &fields)
        .unwrap();
    assert_eq!(
        altered.matches(".alter_table(\"app.users\", |t| {").count(),
        2,
        "{altered}"
    );
}

/// SQLite refuses `ADD COLUMN .. UNIQUE`: the builder adds the column and a
/// unique index there, and drops the index with the column.
#[test]
fn test_a_unique_column_is_added_as_unique_on_every_driver() {
    for driver in ["sqlite", "postgres", "mysql"] {
        let content = alter_users_table(&config(driver), &["email:string:unique"]);

        assert!(
            content.contains("t.string(\"email\").not_null().unique();"),
            "{driver}: {content}"
        );
        assert!(
            content.contains("t.drop_column(\"email\");"),
            "{driver}: {content}"
        );
        assert!(!content.contains("index"), "{driver}: {content}");
    }
}

/// An `i8` field was a `SMALLINT` on MySQL, which holds values the model
/// cannot read, and was refused on PostgreSQL; TideORM now writes it per backend.
#[test]
fn test_an_i8_field_is_a_tiny_integer_on_every_backend() {
    for driver in ["postgres", "mysql", "sqlite"] {
        let content = create_users_table(&config(driver), &["code:i8"]);
        assert!(
            content.contains("t.column(\"code\", ColumnType::TinyInteger).not_null();"),
            "{driver}: {content}"
        );
    }
}

/// An `i16` key numbered by the database got a `BIGSERIAL`/`BIGINT` column,
/// which the driver refuses to read into the model's `i16`.
#[test]
fn test_a_numbered_key_keeps_its_width() {
    let key = ["id:i16:primary_key:auto_increment"];
    for (driver, column) in [
        (
            "postgres",
            "t.column(\"id\", ColumnType::custom(\"SMALLSERIAL\")).primary_key();",
        ),
        (
            "mysql",
            "t.column(\"id\", ColumnType::custom(\"SMALLINT AUTO_INCREMENT\")).primary_key();",
        ),
        ("sqlite", "t.increments(\"id\");"),
    ] {
        let content = create_users_table(&config(driver), &key);
        assert!(content.contains(column), "{driver}: {content}");
        assert!(!content.contains("big"), "{driver}: {content}");
    }

    let error = MigrationGenerator::new(&config("sqlite"))
        .generate_create_table(
            "create_users_table",
            "1",
            "users",
            &fields(&["id:uuid:primary_key:auto_increment"]),
            false,
            false,
        )
        .unwrap_err();
    assert!(error.contains("integer key"), "{error}");
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

/// MySQL refuses an index on a TEXT column (error 1170), so an indexed text
/// field is a VARCHAR there, on a `mariadb` driver too.
#[test]
fn test_mariadb_projects_get_mysql_columns() {
    let indexed_text = ["slug:text:indexed"];
    for driver in ["mysql", "mariadb"] {
        let content = create_users_table(&config(driver), &indexed_text);
        assert!(
            content.contains("t.string(\"slug\").not_null();"),
            "{driver}: {content}"
        );
    }
    let content = create_users_table(&config("postgres"), &indexed_text);
    assert!(
        content.contains("t.text(\"slug\").not_null();"),
        "{content}"
    );
}

#[test]
fn test_default_primary_key_follows_the_configured_type() {
    let mut config = config("postgres");

    config.model.primary_key_type = "uuid".to_string();
    let content = create_users_table(&config, &[]);
    assert!(
        content.contains("t.uuid(\"id\").primary_key();"),
        "{content}"
    );
    assert!(!content.contains("increments"), "{content}");

    config.model.primary_key_type = "i32".to_string();
    let content = create_users_table(&config, &[]);
    assert!(content.contains("t.increments(\"id\");"), "{content}");

    config.model.primary_key_type = "i64".to_string();
    let content = create_users_table(&config, &[]);
    assert!(content.contains("t.big_increments(\"id\");"), "{content}");
}

#[test]
fn test_indexed_fields_get_an_index_in_create_migrations() {
    let content = create_users_table(
        &config("postgres"),
        &["email:string:indexed", "name:string"],
    );

    assert!(content.contains("t.index(&[\"email\"]);"), "{content}");
    assert!(!content.contains("&[\"name\"]"), "{content}");
    assert!(
        content.find("t.string(\"name\")").unwrap() < content.find("t.index(").unwrap(),
        "{content}"
    );
}

/// The index is added after its column, and dropped with it: the builder
/// drops a dropped column's indexes on SQLite, which cannot drop a column an
/// index still covers.
#[test]
fn test_indexed_fields_get_an_index_in_alter_migrations() {
    let content = alter_users_table(&config("sqlite"), &["email:string:nullable:indexed"]);

    let position = |needle: &str| {
        content
            .find(needle)
            .unwrap_or_else(|| panic!("missing {needle}:\n{content}"))
    };
    assert!(position("t.string(\"email\").nullable();") < position("t.add_index(&[\"email\"]);"));
    assert!(content.contains("t.drop_column(\"email\");"), "{content}");
    assert!(!content.contains("drop_index"), "{content}");
}
