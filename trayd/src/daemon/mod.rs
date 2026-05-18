//! Daemon run loop: binds the IPC server, starts the D-Bus SNI host, and
//! enforces single-instance policy via the socket file.

use std::path::PathBuf;
use std::sync::Arc;

use tracing::{info, warn};

use crate::error::TraydBinError;
use crate::ipc::server::{Server, StubHandler, TrayHostHandler};

/// Entry point called from `main`.
pub async fn run() -> Result<(), TraydBinError> {
    let socket_path = crate::config::default_socket_path()?;
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        tracing::info!("received Ctrl+C, shutting down");
        let _ = shutdown_tx.send(true);
    });

    run_with_socket(socket_path, shutdown_rx).await
}

/// Bind `socket_path` and serve until the `shutdown` watch fires.
///
/// Tries to start the real D-Bus SNI host first; if the session bus is
/// unavailable (headless / CI) it falls back to the stub handler and logs a
/// warning.
pub(crate) async fn run_with_socket(
    socket_path: PathBuf,
    shutdown: tokio::sync::watch::Receiver<bool>,
) -> Result<(), TraydBinError> {
    check_not_already_running(&socket_path).await?;

    let server = Server::bind(&socket_path)?;

    match libtrayd::TrayHost::start().await {
        Ok(host) => {
            info!(
                socket = %socket_path.display(),
                version = libtrayd::VERSION,
                "trayd starting (D-Bus SNI host active)"
            );
            let handler = Arc::new(TrayHostHandler::new(host));
            server.run(handler, shutdown).await?;
        }
        Err(e) => {
            warn!(
                error = %e,
                "D-Bus unavailable — running in stub mode (no real tray items)"
            );
            info!(
                socket = %socket_path.display(),
                version = libtrayd::VERSION,
                "trayd starting (stub mode — no D-Bus)"
            );
            let handler = Arc::new(StubHandler::new());
            server.run(handler, shutdown).await?;
        }
    }

    info!("trayd stopped");
    Ok(())
}

/// Return an error if another daemon instance is already answering on the socket.
async fn check_not_already_running(socket_path: &std::path::Path) -> Result<(), TraydBinError> {
    use crate::ipc::client::Client;
    use crate::ipc::protocol::Method;

    if !socket_path.exists() {
        return Ok(());
    }

    // If a ping succeeds, another instance is running.
    if let Ok(mut c) = Client::connect(socket_path).await
        && c.send(Method::Ping).await.is_ok()
    {
        return Err(TraydBinError::AlreadyRunning {
            socket: socket_path.display().to_string(),
        });
    }

    // Socket exists but no daemon responds — stale file; Server::bind will remove it.
    Ok(())
}

#[cfg(test)]
mod tests;
