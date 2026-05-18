use super::stub_connect;
use crate::error::ClientError;

#[test]
fn stub_connect_is_not_ready() {
    assert!(matches!(stub_connect(), Err(ClientError::IpcNotReady)));
}
