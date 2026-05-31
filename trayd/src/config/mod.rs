//! XDG `trayd.toml` config loader.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::TraydBinError;

#[derive(Debug, Deserialize)]
pub struct Config {
    /// Unix socket path for the IPC server.
    #[serde(default = "default_socket_path")]
    pub socket_path: PathBuf,

    /// Tracing filter string (overridden by `RUST_LOG`).
    #[serde(default = "default_log_filter")]
    pub log_filter: String,
}

fn default_socket_path() -> PathBuf {
    crate::ipc::default_socket_path()
}

fn default_log_filter() -> String {
    "warn".into()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            socket_path: default_socket_path(),
            log_filter: default_log_filter(),
        }
    }
}

impl Config {
    /// Load from an explicit path, or discover via XDG, or return defaults.
    ///
    /// - Explicit path given → error if missing or unparsable.
    /// - XDG path exists → parse it; error if unparsable.
    /// - Nothing found → silent defaults.
    pub fn load(explicit: Option<&Path>) -> Result<Self, TraydBinError> {
        if let Some(path) = explicit {
            let text = std::fs::read_to_string(path)
                .map_err(|e| TraydBinError::Config(format!("{}: {e}", path.display())))?;
            return toml::from_str(&text)
                .map_err(|e| TraydBinError::Config(format!("{}: {e}", path.display())));
        }

        let xdg = Self::xdg_path();
        if xdg.exists() {
            let text = std::fs::read_to_string(&xdg)
                .map_err(|e| TraydBinError::Config(format!("{}: {e}", xdg.display())))?;
            return toml::from_str(&text)
                .map_err(|e| TraydBinError::Config(format!("{}: {e}", xdg.display())));
        }

        Ok(Self::default())
    }

    /// `$XDG_CONFIG_HOME/trayd/trayd.toml` (falls back to `~/.config/…`).
    pub fn xdg_path() -> PathBuf {
        let base = std::env::var("XDG_CONFIG_HOME").unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
            format!("{home}/.config")
        });
        PathBuf::from(base).join("trayd").join("trayd.toml")
    }
}

#[cfg(test)]
mod tests;
