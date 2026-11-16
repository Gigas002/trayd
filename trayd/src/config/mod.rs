//! XDG `trayd.toml` config loader.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::TraydBinError;

#[derive(Debug, Deserialize)]
pub struct Config {
    /// Unix socket path for the IPC server; see [`Config::socket_path`].
    #[serde(default)]
    pub socket_path: Option<PathBuf>,

    /// Tracing filter string (overridden by `RUST_LOG`).
    #[serde(default = "default_log_filter")]
    pub log_filter: String,
}

fn default_log_filter() -> String {
    "warn".into()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            socket_path: None,
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

    /// Replace `socket_path` when `socket` (from `--socket`) is given.
    pub fn with_socket_override(mut self, socket: Option<PathBuf>) -> Self {
        if socket.is_some() {
            self.socket_path = socket;
        }
        self
    }

    /// The IPC socket: `socket_path` from the config, else
    /// `$XDG_RUNTIME_DIR/trayd.sock`.
    ///
    /// # Errors
    ///
    /// When neither is set. There is deliberately no `/tmp` fallback: any
    /// local user could create that path first, and the clients would not
    /// look there anyway.
    pub fn socket_path(&self) -> Result<PathBuf, TraydBinError> {
        resolve_socket_path(
            self.socket_path.as_deref(),
            std::env::var_os("XDG_RUNTIME_DIR"),
        )
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

/// [`Config::socket_path`] without reading the environment.
pub fn resolve_socket_path(
    configured: Option<&Path>,
    runtime_dir: Option<std::ffi::OsString>,
) -> Result<PathBuf, TraydBinError> {
    if let Some(path) = configured {
        return Ok(path.to_owned());
    }
    match runtime_dir {
        Some(dir) if !dir.is_empty() => Ok(PathBuf::from(dir).join("trayd.sock")),
        _ => Err(TraydBinError::Config(
            "XDG_RUNTIME_DIR is not set; set it or socket_path in trayd.toml".into(),
        )),
    }
}

#[cfg(test)]
mod tests;
