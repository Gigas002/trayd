//! Unix-socket NDJSON IPC (see `docs/IPC.md`).

pub mod client;
pub mod codec;
pub mod protocol;
pub mod server;

use crate::error::TraydBinError;

pub fn stub_ping() -> Result<(), TraydBinError> {
    Err(TraydBinError::IpcNotReady)
}

pub fn stub_list() -> Result<(), TraydBinError> {
    Err(TraydBinError::IpcNotReady)
}

#[cfg(test)]
mod tests;
