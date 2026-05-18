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
