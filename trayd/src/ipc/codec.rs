use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

use crate::error::TraydBinError;
use crate::ipc::protocol::{IpcRequest, IpcResponse};

pub async fn read_request<R>(reader: &mut R) -> Result<Option<IpcRequest>, TraydBinError>
where
    R: AsyncBufReadExt + Unpin,
{
    let mut line = String::new();
    let n = reader.read_line(&mut line).await?;
    if n == 0 {
        return Ok(None);
    }
    let req = serde_json::from_str(line.trim())?;
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
