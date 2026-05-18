//! Unix-socket NDJSON IPC (see `docs/IPC.md`).
//!
//! **Module layout**
//! - [`protocol`] — wire request/response/event types
//! - [`codec`]    — NDJSON framing (encode/decode + async reader/writer)
//! - [`server`]   — [`server::Server`] + [`server::Handler`] trait + [`server::StubHandler`]
//! - [`client`]   — [`client::Client`] for CLI subcommands
//!
//! **CLI entry points** (`ping`, `list`, `activate`, `subscribe`) live here;
//! they open a [`client::Client`] (or raw framed socket for `subscribe`),
//! send a request, and format output.

pub mod client;
pub mod codec;
pub mod protocol;
pub mod server;

use std::path::Path;

use thiserror::Error;

use crate::error::TraydBinError;
use protocol::{ErrorCode, Method, ResultPayload};

// ── IPC error ─────────────────────────────────────────────────────────────────

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

/// `trayd subscribe` — stream tray events to stdout until Ctrl+C.
pub async fn subscribe(socket_path: &Path) -> Result<(), TraydBinError> {
    use codec::{FramedReader, FramedWriter};
    use protocol::{EventEnvelope, RequestEnvelope, Response};
    use tokio::net::UnixStream;

    let stream = UnixStream::connect(socket_path).await.map_err(|e| {
        if matches!(
            e.kind(),
            std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound
        ) {
            IpcError::NotRunning {
                path: socket_path.display().to_string(),
            }
        } else {
            IpcError::Io(e)
        }
    })?;

    let (r, w) = stream.into_split();
    let mut reader = FramedReader::new(r);
    let mut writer = FramedWriter::new(w);

    writer
        .write_frame(&RequestEnvelope::new(Method::Subscribe))
        .await?;

    // Read and verify the subscribed ack.
    match reader.read_frame::<Response>().await? {
        None => return Err(IpcError::UnexpectedEof.into()),
        Some(resp) => match resp.into_result() {
            Ok(ResultPayload::Subscribed) => {}
            Ok(other) => tracing::warn!(?other, "unexpected subscribe ack payload"),
            Err(err) => {
                return Err(IpcError::Remote {
                    code: err.code,
                    message: err.message,
                }
                .into());
            }
        },
    }

    eprintln!("subscribed — streaming events (Ctrl+C to stop)");

    loop {
        tokio::select! {
            biased;
            _ = tokio::signal::ctrl_c() => { break; }
            frame = reader.read_frame::<EventEnvelope>() => {
                match frame? {
                    None => break,
                    Some(env) => {
                        println!(
                            "{}",
                            serde_json::to_string(&env.event)
                                .map_err(IpcError::Json)?
                        );
                    }
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests;
