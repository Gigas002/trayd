use super::App;

#[test]
fn run_stub_succeeds() {
    assert!(App::run_stub().is_ok());
}
