use clap::Parser;

use super::{Cli, Command};

#[test]
fn default_command_is_none() {
    let cli = Cli::try_parse_from(["trayd"]).unwrap();
    assert!(cli.command.is_none());
}

#[test]
fn ping_subcommand_parses() {
    let cli = Cli::try_parse_from(["trayd", "ping"]).unwrap();
    assert!(matches!(cli.command, Some(Command::Ping)));
}

#[test]
fn run_subcommand_parses() {
    let cli = Cli::try_parse_from(["trayd", "run"]).unwrap();
    assert!(matches!(cli.command, Some(Command::Run)));
}

#[test]
fn socket_flag_parses_before_or_after_the_subcommand() {
    for args in [
        &["trayd", "--socket", "/run/x.sock", "ping"][..],
        &["trayd", "ping", "--socket", "/run/x.sock"][..],
    ] {
        let cli = Cli::try_parse_from(args).unwrap();
        assert_eq!(
            cli.socket.as_deref(),
            Some(std::path::Path::new("/run/x.sock"))
        );
    }
}
