use thiserror::Error;

#[derive(Debug, Error)]
pub enum TraydError {
    #[error("D-Bus error: {0}")]
    DBus(#[from] zbus::Error),

    #[error("D-Bus FDO error: {0}")]
    Fdo(#[from] zbus::fdo::Error),

    #[error("item not found: {0}")]
    NotFound(String),

    #[error("no pixmap available for item {0}")]
    NoPixmap(String),

    #[error("item has no menu: {0}")]
    NoMenu(String),

    #[error("tray host error: {0}")]
    Internal(String),
}
