pub mod codec;
pub mod protocol;
pub mod server;

#[cfg(test)]
mod tests;

use std::path::Path;

use tokio::io::BufReader;
use tokio::net::UnixStream;

use self::codec as ipc_codec;
use self::protocol::{Cmd, IpcRequest, IpcResponse, OkPayload};
use crate::error::TraydBinError;

pub fn default_socket_path() -> std::path::PathBuf {
    let dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into());
    std::path::PathBuf::from(dir).join("trayd.sock")
}

pub async fn ping(socket_path: &Path) -> Result<(), TraydBinError> {
    let stream = UnixStream::connect(socket_path)
        .await
        .map_err(|_| TraydBinError::DaemonUnreachable(socket_path.display().to_string()))?;

    let (read, mut write) = stream.into_split();
    let mut reader = BufReader::new(read);

    ipc_codec::write_request(&mut write, &IpcRequest::new(Cmd::Ping)).await?;

    match ipc_codec::read_response(&mut reader).await? {
        Some(IpcResponse::Ok(r)) if r.payload == OkPayload::Pong => {
            tracing::info!("pong");
            Ok(())
        }
        _ => Err(TraydBinError::UnexpectedResponse),
    }
}
