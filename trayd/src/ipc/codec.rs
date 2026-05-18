//! NDJSON framing: encode/decode and async `FramedReader`/`FramedWriter`.
//!
//! Each frame is a single JSON object followed by `\n`. There is no
//! length prefix — the newline is the sole frame delimiter (same as
//! the NDJSON spec and `socat` debuggability requirement from `docs/IPC.md`).

use serde::{Serialize, de::DeserializeOwned};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use super::IpcError;

// ── Synchronous helpers ────────────────────────────────────────────────────────

/// Serialize `value` into a NDJSON line (`{...}\n`).
pub fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, IpcError> {
    let mut bytes = serde_json::to_vec(value)?;
    bytes.push(b'\n');
    Ok(bytes)
}

/// Deserialize a NDJSON line (trailing `\n` is trimmed automatically).
pub fn decode<T: DeserializeOwned>(line: &str) -> Result<T, IpcError> {
    Ok(serde_json::from_str(line.trim_end_matches('\n'))?)
}

// ── Async reader ───────────────────────────────────────────────────────────────

/// Wraps an async reader and yields one deserialized JSON frame per call.
pub struct FramedReader<R> {
    inner: BufReader<R>,
}

impl<R: tokio::io::AsyncRead + Unpin> FramedReader<R> {
    pub fn new(reader: R) -> Self {
        Self {
            inner: BufReader::new(reader),
        }
    }

    /// Read the next raw NDJSON line. Returns `None` on EOF.
    pub async fn next_line(&mut self) -> Result<Option<String>, IpcError> {
        let mut line = String::new();
        let n = self.inner.read_line(&mut line).await?;
        if n == 0 { Ok(None) } else { Ok(Some(line)) }
    }

    /// Read and deserialize the next NDJSON frame. Returns `None` on EOF.
    pub async fn read_frame<T: DeserializeOwned>(&mut self) -> Result<Option<T>, IpcError> {
        match self.next_line().await? {
            None => Ok(None),
            Some(line) => Ok(Some(decode(&line)?)),
        }
    }
}

// ── Async writer ───────────────────────────────────────────────────────────────

/// Wraps an async writer and emits one NDJSON line per call.
pub struct FramedWriter<W> {
    inner: W,
}

impl<W: tokio::io::AsyncWrite + Unpin> FramedWriter<W> {
    pub fn new(writer: W) -> Self {
        Self { inner: writer }
    }

    /// Serialize `value` and write it as a NDJSON frame.
    pub async fn write_frame<T: Serialize>(&mut self, value: &T) -> Result<(), IpcError> {
        let bytes = encode(value)?;
        self.inner.write_all(&bytes).await?;
        Ok(())
    }
}
