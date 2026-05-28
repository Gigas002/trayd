use thiserror::Error;

#[derive(Debug, Error)]
#[cfg_attr(not(test), allow(dead_code))]
pub enum CtlError {
    #[error("IPC client is not implemented yet (see docs/IPC.md Phase 1)")]
    IpcNotReady,
    #[error("dmenu orchestration is not implemented yet (see docs/PLAN.md Phase 3)")]
    DmenuNotReady,
}
