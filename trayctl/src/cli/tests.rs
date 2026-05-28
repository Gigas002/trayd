use clap::Parser;

use super::Cli;

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
    assert!(cli.run_stub().is_ok());
}
