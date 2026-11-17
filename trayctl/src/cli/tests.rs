use clap::Parser;

use super::{Cli, Command};

#[test]
fn menu_cli_parses() {
    let cli = Cli::try_parse_from([
        "trayctl",
        "menu",
        "--app-id",
        "test-app",
        "--dmenu-cmd",
        "tofi --mode dmenu",
    ])
    .expect("parse");
    assert!(matches!(
        cli.command,
        Command::Menu { ref app_id, ref dmenu_cmd }
        if app_id == "test-app" && dmenu_cmd == "tofi --mode dmenu"
    ));
    assert!(cli.socket.is_none());
}

#[test]
fn items_cli_parses() {
    let cli = Cli::try_parse_from(["trayctl", "items"]).expect("parse");
    assert!(matches!(cli.command, Command::Items));
    assert!(cli.socket.is_none());
}

#[test]
fn socket_flag_before_subcommand() {
    let cli =
        Cli::try_parse_from(["trayctl", "--socket", "/tmp/test.sock", "items"]).expect("parse");
    assert!(matches!(cli.command, Command::Items));
    assert_eq!(cli.socket.unwrap().to_str().unwrap(), "/tmp/test.sock");
}

#[test]
fn socket_flag_after_subcommand() {
    // global flags can appear after the subcommand
    let cli =
        Cli::try_parse_from(["trayctl", "items", "--socket", "/run/trayd.sock"]).expect("parse");
    assert!(matches!(cli.command, Command::Items));
    assert!(cli.socket.is_some());
}

#[test]
fn menu_default_dmenu_cmd() {
    let cli =
        Cli::try_parse_from(["trayctl", "menu", "--app-id", "org.example.App"]).expect("parse");
    assert!(matches!(
        cli.command,
        Command::Menu { ref dmenu_cmd, .. } if dmenu_cmd == "tofi --mode dmenu"
    ));
}

#[test]
fn subscribe_cli_parses() {
    let cli = Cli::try_parse_from(["trayctl", "subscribe"]).expect("parse");
    assert!(matches!(cli.command, Command::Subscribe));
    assert!(cli.socket.is_none());
}

#[test]
fn item_method_commands_take_the_app_id_last() {
    // abar's `feed_id` appends the app id to the configured command.
    let cli = Cli::try_parse_from([
        "trayctl",
        "scroll",
        "--delta",
        "-120",
        "--app-id",
        "org.example.App",
    ])
    .expect("parse");
    assert!(matches!(
        cli.command,
        Command::Scroll { delta: -120, horizontal: false, ref app_id } if app_id == "org.example.App"
    ));

    let cli =
        Cli::try_parse_from(["trayctl", "secondary-activate", "--app-id", "a"]).expect("parse");
    assert!(matches!(
        cli.command,
        Command::SecondaryActivate { x: 0, y: 0, ref app_id } if app_id == "a"
    ));

    let cli = Cli::try_parse_from([
        "trayctl",
        "context-menu",
        "--x",
        "-5",
        "--y",
        "7",
        "--app-id",
        "a",
    ])
    .expect("parse");
    assert!(matches!(
        cli.command,
        Command::ContextMenu { x: -5, y: 7, .. }
    ));

    let cli = Cli::try_parse_from(["trayctl", "activate", "--app-id", "a"]).expect("parse");
    assert!(matches!(cli.command, Command::Activate { item_id: 0, .. }));
}
