//! `com.canonical.dbusmenu` D-Bus client proxy — Phase 2 stub.
//!
//! Full menu tree traversal (`GetLayout`) is implemented in Phase 3.
//! Only the `Event` method is needed here to support menu item activation.

use zbus::zvariant;

/// Proxy for the `com.canonical.dbusmenu` D-Bus interface.
///
/// Phase 2: only `event` (menu item activation) is exercised.
/// Phase 3 will add `get_layout` for full tree traversal.
#[zbus::proxy(interface = "com.canonical.dbusmenu", default_path = "/")]
pub trait DBusMenu {
    /// Fire an event on a menu item.
    ///
    /// Common `event_id` values: `"clicked"`, `"hovered"`, `"opened"`, `"closed"`.
    async fn event(
        &self,
        id: i32,
        event_id: &str,
        data: zvariant::Value<'_>,
        timestamp: u32,
    ) -> zbus::Result<()>;

    /// Emitted when the menu layout changes.
    #[zbus(signal)]
    async fn layout_updated(&self, revision: u32, parent: i32) -> zbus::Result<()>;
}
