use super::stub_connect;
use crate::error::TuiError;

#[test]
fn stub_connect_is_not_ready() {
    assert!(matches!(stub_connect(), Err(TuiError::IpcNotReady)));
}
