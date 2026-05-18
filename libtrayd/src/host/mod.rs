//! [`TrayHost`]: merges D-Bus state for the trayd daemon.

use crate::TraydError;

/// Tray state machine backing the trayd daemon (stub until Phase 2).
#[derive(Debug, Default)]
pub struct TrayHost;

impl TrayHost {
    pub fn new() -> Self {
        Self
    }

    pub fn stub(&self) -> Result<(), TraydError> {
        Err(TraydError::NotImplemented)
    }
}

#[cfg(test)]
mod tests;
