use thiserror::Error;

/// Errors from the `libtrayd` tray host.
#[derive(Debug, Error)]
pub enum TraydError {
    /// D-Bus transport or protocol error.
    #[error("D-Bus error: {0}")]
    DBus(#[from] zbus::Error),

    /// Item not found in the host cache.
    #[error("item not found: {0}")]
    NotFound(String),

    /// D-Bus activation failed.
    #[error("activation failed for {app_id}: {reason}")]
    ActivationFailed { app_id: String, reason: String },

    /// Operation not yet implemented.
    #[error("not implemented yet")]
    NotImplemented,
}
