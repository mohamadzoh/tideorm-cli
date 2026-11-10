use super::{parse_migrations, pick_migration};

const STATUS: &str = "applied\t0001\tcreate_users_table\n\
                          pending\t0002\tcreate_posts_table\n\
                          pending\t0003\tadd_title_to_posts\n";

#[test]
fn the_runner_lines_are_read_and_other_output_is_skipped() {
    let history =
        parse_migrations("connecting..\napplied\t0001\tcreate_users_table\t2026-11-05 10:00:00\n");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].file_name(), "0001_create_users_table");
    assert_eq!(
        history[0].applied_at.as_deref(),
        Some("2026-11-05 10:00:00")
    );

    let status = parse_migrations(STATUS);
    assert_eq!(status.len(), 3);
    assert_eq!(status[1].state, "pending");
    assert_eq!(status[1].applied_at, None);
}

#[test]
fn a_migration_is_picked_exactly_else_by_one_unique_match() {
    let status = || parse_migrations(STATUS);
    let pick = |name: &str| pick_migration(status(), name).map(|migration| migration.version);
    assert_eq!(pick("0002").unwrap(), "0002");
    assert_eq!(pick("create_users_table").unwrap(), "0001");
    assert_eq!(pick("0003_add_title_to_posts").unwrap(), "0003");
    assert_eq!(pick("title").unwrap(), "0003");

    let ambiguous = pick("posts").unwrap_err();
    assert!(ambiguous.contains("matches 2 migrations"), "{ambiguous}");
    assert!(pick("comments").unwrap_err().contains("not found"));

    let error = pick("  ").expect_err("a blank selector must not match every migration");
    assert!(error.contains("Migration name must not be empty"));
}
