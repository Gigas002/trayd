use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::error::CtlError;
use crate::ipc::{IpcClient, TrayEvent};

#[derive(Debug, Parser)]
#[command(
    name = "trayctl",
    version,
    about = "Tray menu orchestrator (IPC + dmenu)"
)]
pub struct Cli {
    /// Unix socket path (default: `$XDG_RUNTIME_DIR/trayd.sock`).
    #[arg(long, global = true)]
    pub socket: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Clone, Subcommand)]
pub enum Command {
    /// Run the dmenu submenu loop for one tray app.
    Menu {
        /// StatusNotifier app id (wire `app_id`).
        #[arg(long)]
        app_id: String,
        /// dmenu-compatible command, run with `sh -c`, e.g. `tofi --mode dmenu` or
        /// `rofi -dmenu -p 'Tray menu'`.
        #[arg(long, default_value = "tofi --mode dmenu")]
        dmenu_cmd: String,
    },
    /// Print tray items (`get_items`) as JSON — useful for scripts.
    Items,
    /// Activate an app (usually its window), or with `--item-id` a menu row.
    Activate {
        /// DBusMenu row id; `0` (the default) activates the app itself.
        #[arg(long, default_value_t = 0)]
        item_id: u32,
        /// StatusNotifier app id (wire `app_id`).
        #[arg(long)]
        app_id: String,
    },
    /// Secondary activation, what most apps do on a middle click.
    SecondaryActivate {
        /// Screen position hint for the app.
        #[arg(long, default_value_t = 0, allow_negative_numbers = true)]
        x: i32,
        #[arg(long, default_value_t = 0, allow_negative_numbers = true)]
        y: i32,
        /// StatusNotifier app id (wire `app_id`).
        #[arg(long)]
        app_id: String,
    },
    /// Scroll over an app's icon (e.g. volume up/down).
    Scroll {
        /// Amount; the sign is the direction (KDE sends 120 per wheel notch).
        #[arg(long, allow_negative_numbers = true)]
        delta: i32,
        /// Scroll horizontally instead of vertically.
        #[arg(long)]
        horizontal: bool,
        /// StatusNotifier app id (wire `app_id`).
        #[arg(long)]
        app_id: String,
    },
    /// Ask an app without a DBusMenu to show its own menu.
    ContextMenu {
        /// Screen position for the menu.
        #[arg(long, default_value_t = 0, allow_negative_numbers = true)]
        x: i32,
        #[arg(long, default_value_t = 0, allow_negative_numbers = true)]
        y: i32,
        /// StatusNotifier app id (wire `app_id`).
        #[arg(long)]
        app_id: String,
    },
    /// Stream tray-state change events as NDJSON until the daemon closes the connection.
    Subscribe,
}

impl Cli {
    pub async fn run(self) -> Result<(), CtlError> {
        let socket_path = self.socket.map_or_else(default_socket_path, Ok)?;

        let mut client = IpcClient::connect(&socket_path).await?;

        match self.command {
            Command::Menu { app_id, dmenu_cmd } => {
                tracing::debug!(%app_id, %dmenu_cmd, "trayctl menu");
                crate::dmenu::run_submenu_loop(&mut client, &app_id, &dmenu_cmd).await
            }
            Command::Items => {
                tracing::debug!("trayctl items");
                let items = client.get_items().await?;
                let json = serde_json::to_string_pretty(&items).map_err(CtlError::Json)?;
                println!("{json}");
                Ok(())
            }
            Command::Activate { item_id, app_id } => client.activate(&app_id, item_id).await,
            Command::SecondaryActivate { x, y, app_id } => {
                client.secondary_activate(&app_id, x, y).await
            }
            Command::Scroll {
                delta,
                horizontal,
                app_id,
            } => client.scroll(&app_id, delta, horizontal).await,
            Command::ContextMenu { x, y, app_id } => client.context_menu(&app_id, x, y).await,
            Command::Subscribe => {
                tracing::debug!("trayctl subscribe");
                client.subscribe().await?;
                loop {
                    match client.recv_event().await {
                        Ok(TrayEvent::Update(items)) => {
                            let json = serde_json::to_string(&items).map_err(CtlError::Json)?;
                            println!("{json}");
                        }
                        Err(CtlError::DaemonUnreachable(_)) => return Ok(()),
                        Err(e) => return Err(e),
                    }
                }
            }
        }
    }
}

/// Resolve the default socket path from `$XDG_RUNTIME_DIR/trayd.sock`.
fn default_socket_path() -> Result<PathBuf, CtlError> {
    let dir = std::env::var("XDG_RUNTIME_DIR")
        .map_err(|_| CtlError::DaemonUnreachable("XDG_RUNTIME_DIR not set".to_owned()))?;
    Ok(PathBuf::from(dir).join("trayd.sock"))
}

#[cfg(test)]
mod tests;
