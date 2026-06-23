//! `org.kde.StatusNotifierItem` D-Bus client proxy.
//!
//! Used by [`TrayHost`](crate::host::TrayHost) to read properties from and
//! invoke methods on registered tray apps.

/// Proxy for the `org.kde.StatusNotifierItem` D-Bus interface.
///
/// Build with the destination and path obtained from the watcher's
/// `RegisterStatusNotifierItem` call:
///
/// ```rust,ignore
/// let proxy = StatusNotifierItemProxy::builder(&conn)
///     .destination(bus_name)?
///     .path(object_path)?
///     .build()
///     .await?;
/// ```
#[zbus::proxy(
    interface = "org.kde.StatusNotifierItem",
    default_path = "/StatusNotifierItem"
)]
pub trait StatusNotifierItem {
    // ── Methods ──────────────────────────────────────────────────────────────

    /// Primary activation at screen coordinates.
    async fn activate(&self, x: i32, y: i32) -> zbus::Result<()>;

    /// Secondary (alternate) activation.
    async fn secondary_activate(&self, x: i32, y: i32) -> zbus::Result<()>;

    /// Scroll event on the icon.
    async fn scroll(&self, delta: i32, orientation: &str) -> zbus::Result<()>;

    /// Context menu request at screen coordinates.
    async fn context_menu(&self, x: i32, y: i32) -> zbus::Result<()>;

    // ── Properties ───────────────────────────────────────────────────────────

    /// Unique application identifier.
    #[zbus(property)]
    fn id(&self) -> zbus::Result<String>;

    /// Icon category.
    #[zbus(property)]
    fn category(&self) -> zbus::Result<String>;

    /// Whether the item is a pure menu (no application window — should not activate).
    #[zbus(property)]
    fn item_is_menu(&self) -> zbus::Result<bool>;

    /// Tooltip data: `(icon_name, icon_pixmaps, title, description)`.
    #[zbus(property)]
    fn tool_tip(&self) -> zbus::Result<(String, Vec<(i32, i32, Vec<u8>)>, String, String)>;

    /// Human-readable title.
    #[zbus(property)]
    fn title(&self) -> zbus::Result<String>;

    /// Visibility/urgency status (`"Passive"`, `"Active"`, or `"NeedsAttention"`).
    #[zbus(property)]
    fn status(&self) -> zbus::Result<String>;

    /// XDG icon-theme name for the normal icon.
    #[zbus(property)]
    fn icon_name(&self) -> zbus::Result<String>;

    /// Raw pixmaps for the normal icon: `(width, height, ARGB32_big_endian_bytes)`.
    #[zbus(property)]
    fn icon_pixmap(&self) -> zbus::Result<Vec<(i32, i32, Vec<u8>)>>;

    /// XDG icon-theme name for the overlay icon.
    #[zbus(property)]
    fn overlay_icon_name(&self) -> zbus::Result<String>;

    /// Raw pixmaps for the overlay icon.
    #[zbus(property)]
    fn overlay_icon_pixmap(&self) -> zbus::Result<Vec<(i32, i32, Vec<u8>)>>;

    /// XDG icon-theme name for the attention icon.
    #[zbus(property)]
    fn attention_icon_name(&self) -> zbus::Result<String>;

    /// Raw pixmaps for the attention icon.
    #[zbus(property)]
    fn attention_icon_pixmap(&self) -> zbus::Result<Vec<(i32, i32, Vec<u8>)>>;

    /// D-Bus object path for the associated `com.canonical.dbusmenu` object.
    ///
    /// Returns `"/"` when the item has no menu.
    #[zbus(property, name = "Menu")]
    fn menu_path(&self) -> zbus::Result<zbus::zvariant::OwnedObjectPath>;

    // ── Signals ───────────────────────────────────────────────────────────────

    /// Emitted when the icon or icon theme path changes.
    #[zbus(signal)]
    async fn new_icon(&self) -> zbus::Result<()>;

    /// Emitted when the title changes.
    #[zbus(signal)]
    async fn new_title(&self) -> zbus::Result<()>;

    /// Emitted when the status changes.
    #[zbus(signal)]
    async fn new_status(&self, status: String) -> zbus::Result<()>;

    /// Emitted when the attention icon changes.
    #[zbus(signal)]
    async fn new_attention_icon(&self) -> zbus::Result<()>;

    /// Emitted when the overlay icon changes.
    #[zbus(signal)]
    async fn new_overlay_icon(&self) -> zbus::Result<()>;

    /// Emitted when the associated `Menu` object path changes.
    #[zbus(signal)]
    async fn new_menu(&self) -> zbus::Result<()>;
}
