//! Submenu loop: IPC `get_menu` → stdin lines → dmenu stdout → `activate` (Phase 3).

use crate::error::CtlError;

#[cfg_attr(not(test), allow(dead_code))]
pub fn stub_run() -> Result<(), CtlError> {
    Err(CtlError::DmenuNotReady)
}

#[cfg(test)]
mod tests;
