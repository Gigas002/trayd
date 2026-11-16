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

impl TraydError {
    /// `true` when [`TrayHost::start`](crate::TrayHost::start) failed because
    /// another process already owns `org.kde.StatusNotifierWatcher`.
    pub fn is_watcher_taken(&self) -> bool {
        matches!(self, Self::DBus(zbus::Error::NameTaken))
    }
}
