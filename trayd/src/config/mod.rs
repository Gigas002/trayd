//! XDG config and default socket path.
//!
//! A full `trayd.toml` config file (socket path, log level, …) is planned
//! for Phase 2+. Phase 1 only exposes the default socket path helper.

use std::path::PathBuf;

use thiserror::Error;

/// Errors from the config layer.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// `$XDG_RUNTIME_DIR` is not set in the environment.
    #[error(
        "XDG_RUNTIME_DIR is not set; cannot determine the default socket path \
         (set the variable or use --socket)"
    )]
    NoXdgRuntimeDir,
}

/// Default IPC socket path: `$XDG_RUNTIME_DIR/trayd.sock`.
pub fn default_socket_path() -> Result<PathBuf, ConfigError> {
    let dir = std::env::var("XDG_RUNTIME_DIR").map_err(|_| ConfigError::NoXdgRuntimeDir)?;
    Ok(PathBuf::from(dir).join("trayd.sock"))
}

#[cfg(test)]
mod tests;
