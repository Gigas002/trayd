use super::stub_ping;
use crate::error::TraydBinError;

#[test]
fn stub_ping_is_not_ready() {
    assert!(matches!(stub_ping(), Err(TraydBinError::IpcNotReady)));
}
