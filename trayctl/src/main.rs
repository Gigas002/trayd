#![forbid(unsafe_code)]

mod cli;
mod dmenu;
mod error;
mod ipc;
mod logger;

use std::process::ExitCode;

use clap::Parser;
use cli::Cli;

#[tokio::main]
async fn main() -> ExitCode {
    logger::init();

    match Cli::parse().run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            tracing::error!(%err, "trayctl failed");
            ExitCode::from(1)
        }
    }
}
