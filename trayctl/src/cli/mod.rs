use clap::{Parser, Subcommand};

use crate::error::CtlError;

#[derive(Debug, Parser)]
#[command(
    name = "trayctl",
    version,
    about = "Tray menu orchestrator (IPC + dmenu)"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Clone, Subcommand)]
pub enum Command {
    /// Run the dmenu submenu loop for one tray app (Phase 3).
    Menu {
        /// StatusNotifier app id (wire `app_id`).
        #[arg(long)]
        app_id: String,
        /// dmenu-compatible command, e.g. `tofi --mode dmenu`.
        #[arg(long, default_value = "tofi --mode dmenu")]
        dmenu_cmd: String,
    },
    /// Print tray items (`get_items`) for scripts (Phase 3).
    Items,
}

impl Cli {
    pub fn run_stub(self) -> Result<(), CtlError> {
        match self.command {
            Command::Menu { app_id, dmenu_cmd } => {
                tracing::info!(%app_id, %dmenu_cmd, "trayctl menu stub (Phase 3)");
                let _ = crate::dmenu::stub_run();
            }
            Command::Items => {
                tracing::info!("trayctl items stub (Phase 3)");
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
