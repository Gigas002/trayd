//! Socket client + wire types per `docs/IPC.md` (not shared with the trayd crate).

use crate::error::TuiError;

#[cfg_attr(not(test), allow(dead_code))]
pub fn stub_connect() -> Result<(), TuiError> {
    Err(TuiError::IpcNotReady)
}

#[cfg(test)]
mod tests;
