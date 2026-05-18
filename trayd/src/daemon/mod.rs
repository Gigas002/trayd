use libtrayd::TrayHost;
use tracing::info;

use crate::error::TraydBinError;

pub fn run() -> Result<(), TraydBinError> {
    let _host = TrayHost::new();
    info!(
        version = libtrayd::VERSION,
        "trayd daemon stub (Phase 2 wires D-Bus + IPC server)"
    );
    Ok(())
}

#[cfg(test)]
mod tests;
