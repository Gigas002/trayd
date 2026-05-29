use tracing_subscriber::EnvFilter;

/// Initialise the global tracing subscriber.
///
/// `RUST_LOG` takes precedence over `config_filter`.
pub fn init(config_filter: &str) {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(config_filter));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}
