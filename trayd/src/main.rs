mod cli;
mod config;
mod daemon;
mod error;
mod ipc;
mod logger;

use std::process::ExitCode;

use clap::Parser;

use crate::cli::{Cli, Command};
use crate::error::TraydBinError;

#[tokio::main]
async fn main() -> ExitCode {
    logger::init();
    let cli = Cli::parse();

    match run(cli).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            tracing::error!(%err, "trayd failed");
            ExitCode::from(1)
        }
    }
}

async fn run(cli: Cli) -> Result<(), TraydBinError> {
    match cli.command.unwrap_or(Command::Run) {
        // The daemon resolves the socket path itself (and installs Ctrl+C).
        Command::Run => daemon::run().await,
        // CLI subcommands need the socket path to connect to the daemon.
        cmd => {
            let socket_path = config::default_socket_path()?;
            match cmd {
                Command::Ping => ipc::ping(&socket_path).await,
                Command::List => ipc::list(&socket_path).await,
                Command::Activate { id } => ipc::activate(&socket_path, id).await,
                Command::Run => unreachable!(),
            }
        }
    }
}
