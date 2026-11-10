use super::{MIGRATIONS, register};
use crate::generators::migration::MIGRATIONS_MOD_RS;
use crate::generators::seeder::{SEEDERS_MOD_RS, register_seeder};

fn seeder(dir: &str, module: &str, name: &str) -> bool {
    register_seeder(dir, module, name).unwrap()
}

#[test]
fn a_seeder_is_declared_and_registered_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let seeders = dir.path().to_string_lossy().into_owned();
    std::fs::write(dir.path().join("mod.rs"), SEEDERS_MOD_RS).unwrap();

    assert!(seeder(&seeders, "database_seeder", "DatabaseSeeder"));
    assert!(seeder(&seeders, "user_seeder", "UserSeeder"));
    assert!(seeder(&seeders, "user_seeder", "UserSeeder"));

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
fn a_hand_written_registry_is_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    let seeders = dir.path().to_string_lossy().into_owned();
    std::fs::write(dir.path().join("mod.rs"), "//! Database seeders\n").unwrap();

    assert!(!seeder(&seeders, "user_seeder", "UserSeeder"));

    let content = std::fs::read_to_string(dir.path().join("mod.rs")).unwrap();
    assert_eq!(content, "//! Database seeders\npub mod user_seeder;\n");
}

/// A migration module is declared by its file's path, over two lines.
#[test]
fn a_migration_is_declared_by_path_and_registered_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let migrations = dir.path().to_string_lossy().into_owned();
    std::fs::write(dir.path().join("mod.rs"), MIGRATIONS_MOD_RS).unwrap();
    for (stem, name) in [
        ("0001_create_users", "CreateUsers"),
        ("0002_add_age", "AddAge"),
    ] {
        let module = format!("m_{stem}");
        let declaration = format!("#[path = \"{stem}.rs\"]\npub mod {module};");
        let entry = format!(".add({module}::{name})");
        assert!(register(&migrations, &module, &declaration, &MIGRATIONS, &entry).unwrap());
    }

    let content = std::fs::read_to_string(dir.path().join("mod.rs")).unwrap();
    assert!(
        content.starts_with(
            "//! Database migrations\n\n#[path = \"0001_create_users.rs\"]\npub mod m_0001_create_users;\n\
                 #[path = \"0002_add_age.rs\"]\npub mod m_0002_add_age;\n\nuse tideorm::prelude::*;\n"
        ),
        "{content}"
    );
    assert!(
        content.ends_with(
            "pub fn all() -> Migrator {\n    Migrator::new()\n        .add(m_0001_create_users::CreateUsers)\n        .add(m_0002_add_age::AddAge)\n}\n"
        ),
        "{content}"
    );
}

/// A declaration goes after the file's inner attributes, which must come first, and the
/// file is replaced whole, leaving no temporary file behind.
#[test]
fn a_declaration_follows_the_inner_attributes() {
    let dir = tempfile::tempdir().unwrap();
    let seeders = dir.path().to_string_lossy().into_owned();
    std::fs::write(
        dir.path().join("mod.rs"),
        "//! Database seeders\n#![allow(unused_imports)]\n\nuse tideorm::prelude::*;\n",
    )
    .unwrap();

    assert!(!seeder(&seeders, "user_seeder", "UserSeeder"));

    let content = std::fs::read_to_string(dir.path().join("mod.rs")).unwrap();
    assert_eq!(
        content,
        "//! Database seeders\n#![allow(unused_imports)]\n\npub mod user_seeder;\n\nuse tideorm::prelude::*;\n"
    );
    let files: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(files, ["mod.rs"]);
}
