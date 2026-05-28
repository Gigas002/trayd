mod cli;
mod dmenu;
mod error;
mod ipc;
mod logger;

use std::process::ExitCode;

use clap::Parser;
use cli::Cli;

fn main() -> ExitCode {
    logger::init();

    match Cli::parse().run_stub() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            tracing::error!(%err, "trayctl failed");
            ExitCode::from(1)
        }
    }
}
