use thiserror::Error;

#[derive(Debug, Error)]
#[cfg_attr(not(test), allow(dead_code))]
pub enum TuiError {
    #[error("IPC client is not implemented yet (see docs/IPC.md Phase 1)")]
    IpcNotReady,
}
