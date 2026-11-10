use super::next_migration_number;
use tempfile::tempdir;

#[test]
fn the_next_number_follows_the_highest_one() {
    let dir = tempdir().unwrap();
    for name in ["0002_b.rs", "0010_c.rs", "mod.rs", "notes.txt"] {
        std::fs::write(dir.path().join(name), "").unwrap();
    }

    assert_eq!(
        next_migration_number(dir.path().to_str().unwrap()),
        Ok("0011".to_string())
    );
}

#[test]
fn a_missing_directory_starts_at_one() {
    let dir = tempdir().unwrap();
    let missing = dir.path().join("migrations");

    assert_eq!(
        next_migration_number(missing.to_str().unwrap()),
        Ok("0001".to_string())
    );
}

/// A number with no successor, or one too large to read, is an error rather than a
/// wrapped or skipped version that would run before the migrations already there.
#[test]
fn numbers_that_cannot_be_followed_are_errors() {
    for name in [
        format!("{}_last.rs", u128::MAX),
        format!("{}0_huge.rs", u128::MAX),
    ] {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join(&name), "").unwrap();

        let error = next_migration_number(dir.path().to_str().unwrap()).unwrap_err();
        assert!(error.contains("follow"), "{name}: {error}");
    }
}

#[test]
fn an_unreadable_directory_is_an_error() {
    let dir = tempdir().unwrap();
    let file = dir.path().join("migrations");
    std::fs::write(&file, "").unwrap();

    assert!(next_migration_number(file.to_str().unwrap()).is_err());
}

/// After `9999` a fifth digit would sort `10000` before `9999` as text, so the
/// next number is refused; numbers already written wider keep their width.
#[test]
fn the_next_number_keeps_the_width_of_those_before_it() {
    let dir = tempdir().unwrap();
    std::fs::write(dir.path().join("9999_last.rs"), "").unwrap();
    let error = next_migration_number(dir.path().to_str().unwrap()).unwrap_err();
    assert!(error.contains("timestamps = true"), "{error}");

    let dir = tempdir().unwrap();
    std::fs::write(dir.path().join("00007_wide.rs"), "").unwrap();
    assert_eq!(
        next_migration_number(dir.path().to_str().unwrap()),
        Ok("00008".to_string())
    );
}
