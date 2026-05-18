//! In-process tray domain types (not IPC wire format).

#[cfg(test)]
mod tests;

// ── Identity ──────────────────────────────────────────────────────────────────

/// Stable item identity: D-Bus service name + object path, e.g.
/// `org.kde.plasma.nm/StatusNotifierItem`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ItemId(pub String);

impl std::fmt::Display for ItemId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

// ── Item ──────────────────────────────────────────────────────────────────────

/// A registered StatusNotifier item with its last-known properties.
#[derive(Debug, Clone)]
pub struct Item {
    pub id: ItemId,
    pub title: Option<String>,
    pub status: ItemStatus,
    pub has_attention_icon: bool,
    pub tooltip: Option<String>,
    pub category: Option<String>,
    pub icon_name: Option<String>,
}

/// `StatusNotifierItem.Status` values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ItemStatus {
    Passive,
    #[default]
    Active,
    NeedsAttention,
}

impl ItemStatus {
    pub fn from_sni_str(s: &str) -> Self {
        match s {
            "Passive" => Self::Passive,
            "NeedsAttention" => Self::NeedsAttention,
            _ => Self::Active,
        }
    }
}

// ── Pixmap ────────────────────────────────────────────────────────────────────

/// Raw pixmap returned by the tray host.
#[derive(Debug, Clone)]
pub struct Pixmap {
    pub format: PixmapFormat,
    pub width: u32,
    pub height: u32,
    /// Raw pixel bytes in the declared format.
    pub data: Vec<u8>,
}

/// Pixel encoding for [`Pixmap`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixmapFormat {
    /// 32-bit ARGB, big-endian, row-major (native SNI wire format).
    Argb32,
}

// ── Events ────────────────────────────────────────────────────────────────────

/// Events broadcast by the tray host to all subscribers.
#[derive(Debug, Clone)]
pub enum HostEvent {
    ItemAdded(Item),
    ItemRemoved(ItemId),
    ItemUpdated(Item),
}

// ── Scroll direction ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollDirection {
    Up,
    Down,
    Left,
    Right,
}
