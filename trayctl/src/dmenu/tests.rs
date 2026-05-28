use super::stub_run;
use crate::error::CtlError;

#[test]
fn stub_run_is_not_ready() {
    assert!(matches!(stub_run(), Err(CtlError::DmenuNotReady)));
}
