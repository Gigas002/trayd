mod app;
mod config;
mod error;
mod ipc;
mod logger;

use std::process::ExitCode;

use crate::app::App;

fn main() -> ExitCode {
    logger::init();

    match App::run_stub() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            tracing::error!(%err, "tray-tui failed");
            ExitCode::from(1)
        }
    }
}
