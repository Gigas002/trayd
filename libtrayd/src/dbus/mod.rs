//! Session D-Bus: StatusNotifierWatcher service, item and menu proxies.

pub mod item;
pub mod menu;
pub mod watcher;

pub use item::StatusNotifierItemProxy;
pub use menu::DBusMenuProxy;
pub use watcher::{StatusNotifierWatcher, WatcherMsg, parse_service};

#[cfg(test)]
mod tests;
