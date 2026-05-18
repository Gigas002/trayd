//! Daemon run loop: binds the IPC server and handles tray events.
//!
//! Phase 1: uses [`StubHandler`] (no D-Bus). Phase 2 will replace the stub
//! with a real [`libtrayd::TrayHost`] + D-Bus connection.

use std::path::PathBuf;
use std::sync::Arc;

use tracing::info;

use crate::error::TraydBinError;
use crate::ipc::server::{Server, StubHandler};

/// Entry point called from `main`. Resolves the socket path from config,
/// installs a Ctrl+C handler, and runs the IPC server indefinitely.
pub async fn run() -> Result<(), TraydBinError> {
    let socket_path = crate::config::default_socket_path()?;
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    // Graceful shutdown on Ctrl+C.
    tokio::spawn(async move {
        // Ignore the result — Ctrl+C might not be available in all envs.
        let _ = tokio::signal::ctrl_c().await;
        tracing::info!("received Ctrl+C, shutting down");
        let _ = shutdown_tx.send(true);
    });

    run_with_socket(socket_path, shutdown_rx).await
}

/// Bind `socket_path` and serve until the `shutdown` watch fires.
///
/// Exposed separately so integration tests can supply their own path and
/// shutdown channel without hitting `XDG_RUNTIME_DIR` or signal handlers.
pub(crate) async fn run_with_socket(
    socket_path: PathBuf,
    shutdown: tokio::sync::watch::Receiver<bool>,
) -> Result<(), TraydBinError> {
    info!(socket = %socket_path.display(), version = libtrayd::VERSION, "trayd starting (Phase 1 mock — no D-Bus)");

    let server = Server::bind(&socket_path)?;
    let handler = Arc::new(StubHandler::new());

    server.run(handler, shutdown).await?;

    info!("trayd stopped");
    Ok(())
}

#[cfg(test)]
mod tests;
