//! In-process IPC client used by `trayd` CLI subcommands.
//!
//! Connects to the running daemon's Unix socket and exchanges single
//! request/response pairs. A streaming subscribe path will be added
//! in Phase 2 alongside the real D-Bus host.

use std::path::Path;

use tokio::net::UnixStream;

use super::IpcError;
use super::codec::{FramedReader, FramedWriter};
use super::protocol::{Method, RequestEnvelope, Response};

/// Connected IPC client.
pub struct Client {
    reader: FramedReader<tokio::net::unix::OwnedReadHalf>,
    writer: FramedWriter<tokio::net::unix::OwnedWriteHalf>,
}

impl Client {
    /// Connect to the daemon socket at `path`.
    ///
    /// Returns [`IpcError::NotRunning`] when the daemon is not listening.
    pub async fn connect(path: &Path) -> Result<Self, IpcError> {
        let stream = UnixStream::connect(path).await.map_err(|e| {
            if matches!(
                e.kind(),
                std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound
            ) {
                IpcError::NotRunning {
                    path: path.display().to_string(),
                }
            } else {
                IpcError::Io(e)
            }
        })?;
        let (r, w) = stream.into_split();
        Ok(Self {
            reader: FramedReader::new(r),
            writer: FramedWriter::new(w),
        })
    }

    /// Send a single request and await the response.
    pub async fn send(&mut self, method: Method) -> Result<Response, IpcError> {
        let envelope = RequestEnvelope::new(method);
        self.writer.write_frame(&envelope).await?;
        match self.reader.read_frame::<Response>().await? {
            Some(resp) => Ok(resp),
            None => Err(IpcError::UnexpectedEof),
        }
    }
}
