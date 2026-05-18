use thiserror::Error;

#[derive(Debug, Error)]
pub enum TraydError {
    #[error("tray host is not implemented yet")]
    NotImplemented,
}
