use thiserror::Error;

#[derive(Debug, Error)]
pub enum TraydBinError {
    #[error(transparent)]
    Host(#[from] libtrayd::TraydError),

    #[error("IPC is not implemented yet (see docs/IPC.md Phase 1)")]
    IpcNotReady,
}
