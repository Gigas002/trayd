//! In-process tray types — not IPC wire format.
//!
//! The IPC wire types live in `trayd::ipc::protocol`.

// ─── ItemId ─────────────────────────────────────────────────────────────────

/// Stable identifier for a tray item.
///
/// On the wire this is either a plain D-Bus name (`com.example.App`) or a
/// combined `busname/object_path` string.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ItemId(pub String);

impl std::fmt::Display for ItemId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl From<String> for ItemId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for ItemId {
    fn from(s: &str) -> Self {
        Self(s.to_owned())
    }
}

// ─── TrayStatus ──────────────────────────────────────────────────────────────

/// Item visibility and urgency status (maps `org.kde.StatusNotifierItem.Status`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum TrayStatus {
    /// Item is passive — may be hidden.
    #[default]
    Passive,
    /// Item is active — show normally.
    Active,
    /// Item needs attention — show prominently.
    NeedsAttention,
}

impl TrayStatus {
    /// Parse the string value from the `Status` D-Bus property.
    pub fn from_dbus(s: &str) -> Self {
        match s {
            "Active" => Self::Active,
            "NeedsAttention" => Self::NeedsAttention,
            _ => Self::Passive,
        }
    }

    /// Serialize to the string form expected by IPC consumers.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Passive => "Passive",
            Self::Active => "Active",
            Self::NeedsAttention => "NeedsAttention",
        }
    }
}

impl std::fmt::Display for TrayStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ─── Icon types ──────────────────────────────────────────────────────────────

/// A single pixmap surface at a given resolution.
///
/// Bytes are ARGB32 in big-endian byte order (per the SNI specification).
#[derive(Debug, Clone)]
pub struct IconPixmap {
    pub width: i32,
    pub height: i32,
    /// Raw ARGB32 bytes.
    pub data: Vec<u8>,
}

/// Raw pixmap data returned by [`crate::host::TrayHost::get_pixmap`].
///
/// Bytes are ARGB32 in big-endian byte order (per the SNI specification).
#[derive(Debug, Clone)]
pub struct PixmapData {
    /// Actual pixel width of the returned pixmap.
    pub width: u32,
    /// Actual pixel height of the returned pixmap.
    pub height: u32,
    /// Raw ARGB32 bytes (`width × height × 4`).
    pub data: Vec<u8>,
}

/// Icon state for a tray item: either a theme icon name, raw pixmaps, or both.
#[derive(Debug, Clone, Default)]
pub struct IconData {
    /// XDG icon-theme icon name (`""` when only pixmaps are provided).
    pub name: String,
    /// Raw pixmap surfaces ordered by size (may be empty when `name` is set).
    pub pixmaps: Vec<IconPixmap>,
}

impl IconData {
    /// `true` when there is no usable icon data.
    pub fn is_empty(&self) -> bool {
        self.name.is_empty() && self.pixmaps.is_empty()
    }

    /// Best-effort icon handle for the IPC wire: returns the theme name, or
    /// `None` when the item only has pixmap data (callers must use `get_pixmap`).
    pub fn as_handle(&self) -> Option<String> {
        if self.name.is_empty() {
            None
        } else {
            Some(self.name.clone())
        }
    }
}

// ─── MenuNode ────────────────────────────────────────────────────────────────

/// One node in a DBusMenu tree (flattened snapshot; children fetched on demand).
#[derive(Debug, Clone)]
pub struct MenuNode {
    /// DBusMenu item id.
    pub id: i32,
    /// Display label (empty string for separators).
    pub label: String,
    /// Whether the item can be selected.
    pub enabled: bool,
    /// Whether the item should be displayed.
    pub visible: bool,
    /// Icon name if the item carries one.
    pub icon_name: String,
    /// `true` when this node represents a submenu (children not yet fetched).
    pub is_submenu: bool,
    /// Direct children (populated on demand).
    pub children: Vec<MenuNode>,
}

// ─── TrayItem ────────────────────────────────────────────────────────────────

/// In-process snapshot of one registered SNI item.
#[derive(Debug, Clone)]
pub struct TrayItem {
    /// Stable registration id (used as the IPC `app_id`).
    pub id: ItemId,
    /// D-Bus service (bus) name.
    pub bus_name: String,
    /// D-Bus object path for the `org.kde.StatusNotifierItem` interface.
    pub object_path: String,
    /// Human-readable title.
    pub title: String,
    /// Current visibility / urgency status.
    pub status: TrayStatus,
    /// Normal icon data.
    pub icon: IconData,
    /// Attention icon data — shown when `status == NeedsAttention`.
    pub attention_icon: IconData,
    /// D-Bus object path for the associated DBusMenu object (`""` if absent).
    pub menu_path: String,
}

// ─── HostEvent ───────────────────────────────────────────────────────────────

/// Events broadcast from [`crate::host::TrayHost`] to IPC subscribers.
#[derive(Debug, Clone)]
pub enum HostEvent {
    /// A new item was registered and its properties fetched.
    ItemAdded(TrayItem),
    /// An item's title, status, or icon was updated.
    ItemChanged(TrayItem),
    /// An item's bus name disappeared (app exited or unregistered).
    ItemRemoved(ItemId),
}

#[cfg(test)]
mod tests;
