use std::io::IsTerminal;

use tracing_subscriber::EnvFilter;

/// Log to stderr (stdout carries `items`/`subscribe` output), coloured only
/// on a terminal.
pub fn init() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .init();
}
