use super::{Cli, Commands, MakeCommands};
use clap::Parser;

fn model_flags(args: &[&str]) -> (Option<bool>, Option<bool>, Option<bool>) {
    let cli = Cli::try_parse_from(args).expect("make model should parse");

    match cli.command {
        Commands::Make(MakeCommands::Model {
            timestamps,
            soft_deletes,
            tokenize,
            ..
        }) => (timestamps, soft_deletes, tokenize),
        _ => panic!("expected a `make model` command"),
    }
}

/// The `[model]` config can only win when an unsupplied flag is distinguishable from
/// one that was passed explicitly, which is what `Option<bool>` buys.
#[test]
fn model_flags_are_absent_until_supplied() {
    assert_eq!(
        model_flags(&["tideorm", "make", "model", "User"]),
        (None, None, None)
    );
}

#[test]
fn model_flags_record_explicit_values() {
    assert_eq!(
        model_flags(&["tideorm", "make", "model", "User", "--timestamps=true"]),
        (Some(true), None, None)
    );
    assert_eq!(
        model_flags(&["tideorm", "make", "model", "User", "--timestamps=false"]),
        (Some(false), None, None)
    );
    assert_eq!(
        model_flags(&["tideorm", "make", "model", "User", "--timestamps"]),
        (Some(true), None, None)
    );
    assert_eq!(
        model_flags(&["tideorm", "make", "model", "User", "--soft-deletes"]),
        (None, Some(true), None)
    );
    assert_eq!(
        model_flags(&["tideorm", "make", "model", "User", "--tokenize"]),
        (None, None, Some(true))
    );
}

#[test]
fn make_model_preserves_an_explicit_default_output_directory() {
    let cli = Cli::try_parse_from(["tideorm", "make", "model", "User", "--output=src/models"])
        .expect("make model should parse");

    match cli.command {
        Commands::Make(MakeCommands::Model { output, .. }) => {
            assert_eq!(output.as_deref(), Some("src/models"));
        }
        _ => panic!("expected a `make model` command"),
    }
}

#[test]
fn migration_step_cannot_be_combined_with_a_specific_migration() {
    assert!(
        Cli::try_parse_from([
            "tideorm",
            "migrate",
            "up",
            "--migration",
            "0001_create_users",
            "--step",
            "1",
        ])
        .is_err()
    );
    assert!(
        Cli::try_parse_from([
            "tideorm",
            "migrate",
            "down",
            "--migration",
            "0001_create_users",
            "--step",
            "1",
        ])
        .is_err()
    );
}

#[test]
fn a_specific_fresh_seeder_requires_seeding() {
    assert!(
        Cli::try_parse_from(["tideorm", "migrate", "fresh", "--seeder", "UserSeeder",]).is_err()
    );
}

#[test]
fn a_step_refresh_cannot_seed() {
    assert!(
        Cli::try_parse_from(["tideorm", "migrate", "refresh", "--step", "1", "--seed"]).is_err()
    );
}

#[test]
fn migration_step_counts_must_be_positive() {
    for args in [
        vec!["tideorm", "migrate", "run", "--step", "0"],
        vec!["tideorm", "migrate", "up", "--step", "0"],
        vec!["tideorm", "migrate", "down", "--step", "0"],
        vec!["tideorm", "migrate", "redo", "--step", "0"],
        vec!["tideorm", "migrate", "refresh", "--step", "0"],
    ] {
        assert!(Cli::try_parse_from(args).is_err());
    }
}
