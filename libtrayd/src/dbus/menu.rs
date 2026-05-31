//! `com.canonical.dbusmenu` client proxy.
//!
//! `get_layout` is used for full menu tree traversal.

use std::collections::HashMap;

use zbus::zvariant::{self, OwnedValue};

/// Proxy for the `com.canonical.dbusmenu` D-Bus interface.
///
/// Build with the destination and path obtained from the SNI `Menu` property:
///
/// ```rust,ignore
/// let proxy = DBusMenuProxy::builder(&conn)
///     .destination(bus_name)?
///     .path(menu_path)?
///     .build()
///     .await?;
/// ```
#[zbus::proxy(interface = "com.canonical.dbusmenu", default_path = "/")]
pub trait DBusMenu {
    /// Fire an event on a menu item.
    ///
    /// Common `event_id` values: `"clicked"`, `"hovered"`, `"opened"`, `"closed"`.
    /// Pass `zvariant::Value::from(0i32)` for `data` when unused.
    async fn event(
        &self,
        id: i32,
        event_id: &str,
        data: zvariant::Value<'_>,
        timestamp: u32,
    ) -> zbus::Result<()>;

    /// Fetch the menu layout tree starting at `parent_id`.
    ///
    /// - `parent_id = 0` → root menu.
    /// - `recursion_depth = 1` → direct children only (use `-1` for the full tree).
    /// - `property_names = &[]` → return all properties.
    ///
    /// Returns `(revision, (id, properties, children))` where `children` is an
    /// `av` — each element is a `(i32 id, a{sv} properties, av children)` variant.
    async fn get_layout(
        &self,
        parent_id: i32,
        recursion_depth: i32,
        property_names: &[&str],
    ) -> zbus::Result<(u32, (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>))>;

    /// Emitted when the menu layout changes.
    #[zbus(signal)]
    async fn layout_updated(&self, revision: u32, parent: i32) -> zbus::Result<()>;
}
