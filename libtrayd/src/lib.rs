//! Tray host library: D-Bus StatusNotifier + DBusMenu (no IPC, no CLI).

pub mod dbus;
pub mod error;
pub mod host;
pub mod model;

pub use error::TraydError;
pub use host::TrayHost;
pub use model::{HostEvent, Item, ItemId, ItemStatus, Pixmap, PixmapFormat, ScrollDirection};

/// Library version (matches workspace package version at release time).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
