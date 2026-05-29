//! D-Bus layer compile-time tests.
//!
//! Live bus tests are `#[ignore]`d and intended for manual runs:
//!   cargo test --package libtrayd -- dbus::tests:: --ignored

use super::watcher::parse_service;

#[test]
fn parse_service_plain_bus_name() {
    let (bus, path) = parse_service(":1.100", "com.example.App");
    assert_eq!(bus, "com.example.App");
    assert_eq!(path, "/StatusNotifierItem");
}

#[test]
fn parse_service_object_path_uses_sender() {
    let (bus, path) = parse_service(":1.100", "/org/example/Tray");
    assert_eq!(bus, ":1.100");
    assert_eq!(path, "/org/example/Tray");
}

#[test]
fn parse_service_combined_form() {
    let (bus, path) = parse_service(":1.100", "com.example.App/StatusNotifierItem");
    assert_eq!(bus, "com.example.App");
    assert_eq!(path, "/StatusNotifierItem");
}

#[test]
fn parse_service_unique_name_with_path() {
    let (bus, path) = parse_service(":1.100", ":1.100/StatusNotifierItem");
    assert_eq!(bus, ":1.100");
    assert_eq!(path, "/StatusNotifierItem");
}

#[test]
fn dbus_module_compiles() {}
