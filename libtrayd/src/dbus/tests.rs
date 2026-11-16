//! D-Bus layer compile-time tests.
//!
//! Live bus behaviour is covered by `libtrayd/tests/live.rs` (private bus).

use super::watcher::{parse_service, service_belongs_to};

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
fn service_belongs_to_matches_whole_bus_names_only() {
    assert!(service_belongs_to("org.example.App", "org.example.App"));
    assert!(service_belongs_to(":1.2/StatusNotifierItem", ":1.2"));
    assert!(!service_belongs_to(":1.20/StatusNotifierItem", ":1.2"));
    assert!(!service_belongs_to("org.example.App2", "org.example.App"));
    assert!(!service_belongs_to("org.example.App", "org.example.App2"));
}
