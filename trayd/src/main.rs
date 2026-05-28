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
        Command::Run => daemon::run().await,
        cmd => {
            let socket_path = config::default_socket_path()?;
            match cmd {
                Command::Ping => ipc::ping(&socket_path).await,
                Command::List => ipc::list(&socket_path).await,
                Command::Activate { id } => ipc::activate(&socket_path, id).await,
                Command::Subscribe => ipc::subscribe(&socket_path).await,
                Command::MenuList { item, node } => ipc::menu_list(&socket_path, item, node).await,
                Command::MenuClick { item, label } => {
                    ipc::menu_click(&socket_path, item, label).await
                }
                Command::Run => unreachable!(),
            }
        }
    }
}
