use super::TrayHost;
use crate::TraydError;

#[test]
fn tray_host_stub_returns_not_implemented() {
    let host = TrayHost::new();
    assert!(matches!(host.stub(), Err(TraydError::NotImplemented)));
}
