use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};

use crate::error::TraydBinError;
use crate::ipc::protocol::{IpcRequest, IpcResponse};

/// Longest request line the daemon accepts, newline included.  Requests are
/// a few hundred bytes; the limit keeps a client that never sends a newline
/// from making the daemon buffer without bound.
pub const MAX_REQUEST_LEN: usize = 64 * 1024;

/// Read one request line.
///
/// # Errors
///
/// [`TraydBinError::RequestTooLong`] past [`MAX_REQUEST_LEN`] bytes,
/// [`TraydBinError::Json`] for a line that is not a valid request, and
/// [`TraydBinError::Io`] on socket errors.
pub async fn read_request<R>(reader: &mut R) -> Result<Option<IpcRequest>, TraydBinError>
where
    R: AsyncBufReadExt + Unpin,
{
    let mut line = Vec::new();
    let limit = MAX_REQUEST_LEN as u64 + 1;
    let n = (&mut *reader)
        .take(limit)
        .read_until(b'\n', &mut line)
        .await?;
    if n == 0 {
        return Ok(None);
    }
    if n > MAX_REQUEST_LEN {
        return Err(TraydBinError::RequestTooLong);
    }
    let req = serde_json::from_slice(line.trim_ascii())?;
    Ok(Some(req))
}

pub async fn write_response<W>(writer: &mut W, resp: &IpcResponse) -> Result<(), TraydBinError>
where
    W: AsyncWriteExt + Unpin,
{
    let mut json = serde_json::to_string(resp)?;
    json.push('\n');
    writer.write_all(json.as_bytes()).await?;
    Ok(())
}

pub async fn write_request<W>(writer: &mut W, req: &IpcRequest) -> Result<(), TraydBinError>
where
    W: AsyncWriteExt + Unpin,
{
    let mut json = serde_json::to_string(req)?;
    json.push('\n');
    writer.write_all(json.as_bytes()).await?;
    Ok(())
}

pub async fn read_response<R>(reader: &mut R) -> Result<Option<IpcResponse>, TraydBinError>
where
    R: AsyncBufReadExt + Unpin,
{
    let mut line = String::new();
    let n = reader.read_line(&mut line).await?;
    if n == 0 {
        return Ok(None);
    }
    let resp = serde_json::from_str(line.trim())?;
    Ok(Some(resp))
}
