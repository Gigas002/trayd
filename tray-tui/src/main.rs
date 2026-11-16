#![forbid(unsafe_code)]

mod app;
mod config;
mod error;
mod ipc;
mod logger;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;

use crate::app::App;
use crate::config::Config;
use crate::error::TuiError;

#[derive(Debug, Parser)]
#[command(
    name = "tray-tui",
    version,
    about = "Terminal tray UI (IPC socket client)"
)]
struct Cli {
    /// Unix socket path (default: `$XDG_RUNTIME_DIR/trayd.sock`).
    #[arg(long)]
    socket: Option<PathBuf>,

    /// Config file path (default: `$XDG_CONFIG_HOME/tray-tui/config.toml`).
    #[arg(long)]
    config: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> ExitCode {
    logger::init();

    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            tracing::error!(%err, "tray-tui failed");
            ExitCode::from(1)
        }
    }
}

async fn run() -> Result<(), TuiError> {
    let cli = Cli::parse();
    let config = Config::load(cli.config.as_deref())?;
    let socket_path = config.resolve_socket(cli.socket)?;
    App::run(socket_path).await
}
