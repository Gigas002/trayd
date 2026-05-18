use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "trayd", version, about = "System tray daemon and CLI")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Clone, Subcommand)]
pub enum Command {
    /// Run the tray daemon (default when no subcommand is given).
    Run,
    /// Check that the daemon is reachable and print its version.
    Ping,
    /// List all registered tray items.
    List,
    /// Send a primary-click activation to a tray item.
    Activate {
        /// Stable item id (D-Bus service name).
        id: String,
    },
}

#[cfg(test)]
mod tests;
