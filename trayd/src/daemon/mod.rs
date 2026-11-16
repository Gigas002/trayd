use crate::config::Config;
use crate::error::TraydBinError;
use crate::ipc::server::IpcServer;
use libtrayd::TrayHost;

pub async fn run(config: &Config) -> Result<(), TraydBinError> {
    let socket_path = config.socket_path()?;
    if crate::ipc::ping(&socket_path).await.is_ok() {
        return Err(TraydBinError::AlreadyRunning);
    }

    // Check the socket path before claiming the watcher name, so a bad path
    // fails fast without touching the bus.
    crate::ipc::server::prepare_socket_path(&socket_path)?;
    let host = TrayHost::start().await.map_err(|e| {
        if e.is_watcher_taken() {
            TraydBinError::WatcherTaken
        } else {
            e.into()
        }
    })?;
    let server = IpcServer::new(&socket_path, host);
    tokio::select! {
        result = server.run() => result,
        signal = shutdown_signal() => {
            tracing::info!(signal, "shutting down");
            // Leave no socket behind for clients to connect to in vain.
            let _ = std::fs::remove_file(&socket_path);
            Ok(())
        }
    }
}

/// Resolves on SIGINT or SIGTERM with the signal's name.
async fn shutdown_signal() -> &'static str {
    use tokio::signal::unix::{SignalKind, signal};
    let (Ok(mut int), Ok(mut term)) = (
        signal(SignalKind::interrupt()),
        signal(SignalKind::terminate()),
    ) else {
        tracing::warn!("cannot listen for SIGINT/SIGTERM; the socket won't be removed on exit");
        return std::future::pending().await;
    };
    tokio::select! {
        _ = int.recv() => "SIGINT",
        _ = term.recv() => "SIGTERM",
    }
}

#[cfg(test)]
mod tests;
