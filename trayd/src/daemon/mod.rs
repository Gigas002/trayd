use crate::config::Config;
use crate::error::TraydBinError;
use crate::ipc::server::IpcServer;
use libtrayd::TrayHost;

pub async fn run(config: &Config) -> Result<(), TraydBinError> {
    if crate::ipc::ping(&config.socket_path).await.is_ok() {
        return Err(TraydBinError::AlreadyRunning);
    }

    let host = TrayHost::start().await?;
    let server = IpcServer::new(&config.socket_path, host);
    server.run().await
}

#[cfg(test)]
mod tests;
