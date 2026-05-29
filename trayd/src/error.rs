use thiserror::Error;

#[derive(Debug, Error)]
pub enum TraydBinError {
    #[error(transparent)]
    Host(#[from] libtrayd::TraydError),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("config error: {0}")]
    Config(String),

    #[error("daemon already running")]
    AlreadyRunning,

    #[error("daemon not reachable at {0}")]
    DaemonUnreachable(String),

    #[error("unexpected IPC response")]
    UnexpectedResponse,
}
