//! Session D-Bus: StatusNotifierWatcher service, item and menu proxies.

pub mod item;
pub mod menu;
pub mod watcher;

pub use item::StatusNotifierItemProxy;
pub use menu::DBusMenuProxy;
pub(crate) use watcher::run_watcher_cmd_loop;
pub use watcher::{StatusNotifierWatcher, WatcherCmd, WatcherMsg, parse_service};

#[cfg(test)]
mod tests;
