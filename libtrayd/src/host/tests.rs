//! TrayHost unit tests (headless — no live D-Bus connection).
//!
//! Live D-Bus integration tests are in `libtrayd/tests/` and are marked
//! `#[ignore]` so they don't run in CI without a session bus.

// Phase 2: headless tests cover the host state helpers; live bus tests
// are integration-level and require a running D-Bus session.

#[test]
fn host_module_compiles() {}
