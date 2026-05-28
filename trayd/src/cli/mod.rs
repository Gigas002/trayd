use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "trayd", version, about = "System tray daemon and CLI")]
pub struct Cli {
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
