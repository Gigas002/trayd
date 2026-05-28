//! Socket client + wire types per `docs/IPC.md` (not shared with the trayd crate).

use crate::error::CtlError;

#[cfg_attr(not(test), allow(dead_code))]
pub fn stub_connect() -> Result<(), CtlError> {
    Err(CtlError::IpcNotReady)
}

#[cfg(test)]
mod tests;
