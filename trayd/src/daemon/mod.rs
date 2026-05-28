use crate::error::TraydBinError;
use crate::ipc::server::IpcServer;

pub async fn run() -> Result<(), TraydBinError> {
    let socket_path = crate::ipc::default_socket_path();
    let server = IpcServer::new(socket_path);
    server.run().await
}

#[cfg(test)]
mod tests;
