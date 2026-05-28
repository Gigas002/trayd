use super::stub_connect;
use crate::error::CtlError;

#[test]
fn stub_connect_is_not_ready() {
    assert!(matches!(stub_connect(), Err(CtlError::IpcNotReady)));
}
