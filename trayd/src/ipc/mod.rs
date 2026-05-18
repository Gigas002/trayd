//! Unix-socket NDJSON IPC (see `docs/IPC.md`).
//!
//! **Module layout**
//! - [`protocol`] — wire request/response/event types
//! - [`codec`]    — NDJSON framing (encode/decode + async reader/writer)
//! - [`server`]   — [`server::Server`] + [`server::Handler`] trait + [`server::StubHandler`]
//! - [`client`]   — [`client::Client`] for CLI subcommands
//!
//! **CLI entry points** (`ping`, `list`, `activate`) live here; they are thin
//! wrappers that open a [`client::Client`], send a request, and format output.

pub mod client;
pub mod codec;
pub mod protocol;
pub mod server;

use std::path::Path;

use thiserror::Error;

use crate::error::TraydBinError;
use protocol::{ErrorCode, Method, ResultPayload};

// ── IPC error ─────────────────────────────────────────────────────────────────

/// Errors that can occur in the IPC layer (both server and client side).
#[derive(Debug, Error)]
pub enum IpcError {
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON: {0}")]
    Json(#[from] serde_json::Error),

    #[error("trayd is not running at {path}; start it with `trayd run`")]
    NotRunning { path: String },

    #[error("server closed connection without sending a response")]
    UnexpectedEof,

    #[error("server error [{code}]: {message}")]
    Remote { code: ErrorCode, message: String },
}

// ── CLI helpers ───────────────────────────────────────────────────────────────

/// `trayd ping` — health-check the running daemon.
pub async fn ping(socket_path: &Path) -> Result<(), TraydBinError> {
    let mut c = client::Client::connect(socket_path).await?;
    let resp = c.send(Method::Ping).await?;
    match resp.into_result() {
        Ok(ResultPayload::Pong { version }) => {
            println!("pong: trayd {version}");
            Ok(())
        }
        Ok(other) => {
            tracing::warn!(?other, "unexpected result payload for ping");
            Ok(())
        }
        Err(err) => Err(IpcError::Remote {
            code: err.code,
            message: err.message,
        }
        .into()),
    }
}

/// `trayd list` — print all registered tray items.
pub async fn list(socket_path: &Path) -> Result<(), TraydBinError> {
    let mut c = client::Client::connect(socket_path).await?;
    let resp = c.send(Method::List).await?;
    match resp.into_result() {
        Ok(ResultPayload::List { items }) => {
            if items.is_empty() {
                println!("(no tray items registered)");
            } else {
                for item in &items {
                    let title = item.title.as_deref().unwrap_or("<untitled>");
                    println!("{}: {} [{:?}]", item.id, title, item.status);
                }
            }
            Ok(())
        }
        Ok(other) => {
            tracing::warn!(?other, "unexpected result payload for list");
            Ok(())
        }
        Err(err) => Err(IpcError::Remote {
            code: err.code,
            message: err.message,
        }
        .into()),
    }
}

/// `trayd activate <id>` — send a primary-click activation to an item.
pub async fn activate(socket_path: &Path, item_id: String) -> Result<(), TraydBinError> {
    let mut c = client::Client::connect(socket_path).await?;
    let resp = c.send(Method::Activate { item_id }).await?;
    match resp.into_result() {
        Ok(ResultPayload::Ok) => Ok(()),
        Ok(other) => {
            tracing::warn!(?other, "unexpected result payload for activate");
            Ok(())
        }
        Err(err) => Err(IpcError::Remote {
            code: err.code,
            message: err.message,
        }
        .into()),
    }
}

#[cfg(test)]
mod tests;
