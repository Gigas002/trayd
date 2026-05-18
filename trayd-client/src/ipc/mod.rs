//! Socket client + wire types per `docs/IPC.md` (not shared with the trayd crate).

use crate::error::ClientError;

#[cfg_attr(not(test), allow(dead_code))]
pub fn stub_connect() -> Result<(), ClientError> {
    Err(ClientError::IpcNotReady)
}

#[cfg(test)]
mod tests;
