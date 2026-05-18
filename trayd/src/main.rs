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

fn main() -> ExitCode {
    logger::init();
    let cli = Cli::parse();

    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            tracing::error!(%err, "trayd failed");
            ExitCode::from(1)
        }
    }
}

fn run(cli: Cli) -> Result<(), TraydBinError> {
    match cli.command.unwrap_or(Command::Run) {
        Command::Run => daemon::run(),
        Command::Ping => ipc::stub_ping(),
        Command::List => ipc::stub_list(),
    }
}
