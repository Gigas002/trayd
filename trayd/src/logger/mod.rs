use std::io::IsTerminal;

use tracing_subscriber::EnvFilter;

/// Initialise the global tracing subscriber.
///
/// `RUST_LOG` takes precedence over `config_filter`.  Logs go to stderr, as
/// usual for a daemon, and are coloured only when stderr is a terminal so
/// journald and log files don't fill with escape codes.
pub fn init(config_filter: &str) {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(config_filter));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .init();
}
