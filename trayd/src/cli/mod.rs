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
        /// Stable item id (D-Bus service name + object path).
        id: String,
    },
    /// Stream tray events to stdout (one JSON object per line).
    Subscribe,
    /// Print menu items for a tray item, one label per line.
    ///
    /// Separators print as `---`; submenus print as `Label >`.
    /// Use `--node <id>` (printed by `menu-click` on exit 2) to list a submenu level.
    MenuList {
        /// Stable item id.
        #[arg(long)]
        item: String,
        /// Submenu parent node id (from a prior `menu-click` exit 2).
        #[arg(long)]
        node: Option<i32>,
    },
    /// Click a menu item by label.
    ///
    /// If `--label` is omitted, reads the label from stdin (pipe-friendly).
    MenuClick {
        /// Stable item id.
        #[arg(long)]
        item: String,
        /// Label to click (as printed by `menu-list`).
        #[arg(long)]
        label: Option<String>,
    },
}

#[cfg(test)]
mod tests;
