use clap::Parser;

use super::{Cli, Command};

#[test]
fn default_command_is_run() {
    let cli = Cli::try_parse_from(["trayd"]).unwrap();
    assert!(cli.command.is_none());
}

#[test]
fn ping_subcommand_parses() {
    let cli = Cli::try_parse_from(["trayd", "ping"]).unwrap();
    assert!(matches!(cli.command, Some(Command::Ping)));
}

#[test]
fn list_subcommand_parses() {
    let cli = Cli::try_parse_from(["trayd", "list"]).unwrap();
    assert!(matches!(cli.command, Some(Command::List)));
}

#[test]
fn activate_subcommand_parses() {
    let cli = Cli::try_parse_from(["trayd", "activate", "org.kde.plasma.nm"]).unwrap();
    assert!(matches!(
        cli.command,
        Some(Command::Activate { ref id }) if id == "org.kde.plasma.nm"
    ));
}

#[test]
fn run_subcommand_parses_explicitly() {
    let cli = Cli::try_parse_from(["trayd", "run"]).unwrap();
    assert!(matches!(cli.command, Some(Command::Run)));
}
