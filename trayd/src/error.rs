use thiserror::Error;

#[derive(Debug, Error)]
pub enum TraydBinError {
    #[error(transparent)]
    Host(#[from] libtrayd::TraydError),

    #[error(transparent)]
    Config(#[from] crate::config::ConfigError),

    #[error(transparent)]
    Ipc(#[from] crate::ipc::IpcError),

    #[error("another trayd instance is already running at {socket}")]
    AlreadyRunning { socket: String },
}
