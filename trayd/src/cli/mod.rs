use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "trayd", version, about = "System tray daemon and CLI")]
pub struct Cli {
    /// Path to config file (default: $XDG_CONFIG_HOME/trayd/trayd.toml).
    #[arg(long, value_name = "FILE")]
    pub config: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Clone, Subcommand)]
pub enum Command {
    /// Run the tray daemon (default).
    Run,
    /// Check daemon reachability over IPC.
    Ping,
}

#[cfg(test)]
mod tests;
