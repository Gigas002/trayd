use tracing::info;

/// TUI entry (stub until Phase 4).
pub struct App;

impl App {
    pub fn run_stub() -> Result<(), crate::error::TuiError> {
        info!("tray-tui stub (Phase 4 adds ratatui UI over IPC)");
        Ok(())
    }
}

#[cfg(test)]
mod tests;
