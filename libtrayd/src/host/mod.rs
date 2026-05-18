//! [`TrayHost`]: D-Bus SNI watcher + in-memory item state.
//!
//! The host is the single entry point for the `trayd` daemon.  It registers
//! `org.kde.StatusNotifierWatcher` on the session bus, tracks item arrivals and
//! departures, and exposes a synchronous list + async action API.

use std::sync::Arc;

use tokio::sync::{Mutex, broadcast};
use tracing::info;

use crate::dbus::{
    WatcherIface, WatcherState, item_proxy, pick_pixmap, scroll_delta, scroll_orientation,
    spawn_name_monitor,
};
use crate::error::TraydError;
use crate::model::{HostEvent, Item, ItemId, Pixmap, ScrollDirection};

#[cfg(test)]
mod tests;

const SNI_WATCHER_NAME: &str = "org.kde.StatusNotifierWatcher";
const SNI_WATCHER_PATH: &str = "/StatusNotifierWatcher";
const SNI_HOST_NAME: &str = "org.kde.StatusNotifierHost.trayd";

// ── Public API ─────────────────────────────────────────────────────────────────

/// Running SNI tray host.
///
/// Created via [`TrayHost::start`].  All D-Bus I/O happens on a dedicated
/// tokio task managed by the `zbus` connection.
pub struct TrayHost {
    conn: zbus::Connection,
    state: Arc<Mutex<WatcherState>>,
    event_tx: broadcast::Sender<HostEvent>,
}

impl TrayHost {
    /// Connect to the session bus, register the SNI watcher + host names, and
    /// start background monitoring.
    pub async fn start() -> Result<Self, TraydError> {
        let conn = zbus::Connection::session().await?;
        let state = Arc::new(Mutex::new(WatcherState::default()));
        let (event_tx, _) = broadcast::channel(256);

        let watcher = WatcherIface::new(state.clone(), conn.clone(), event_tx.clone());
        conn.object_server().at(SNI_WATCHER_PATH, watcher).await?;

        conn.request_name(SNI_WATCHER_NAME).await?;
        // Register ourselves as a host too so items know a host is present.
        conn.request_name(SNI_HOST_NAME).await?;

        spawn_name_monitor(conn.clone(), state.clone(), event_tx.clone());

        info!(watcher = SNI_WATCHER_NAME, "SNI watcher registered");

        Ok(Self {
            conn,
            state,
            event_tx,
        })
    }

    // ── Queries ───────────────────────────────────────────────────────────────

    /// Snapshot of all currently registered items (cheap in-memory read).
    pub async fn list(&self) -> Vec<Item> {
        self.state
            .lock()
            .await
            .items
            .values()
            .map(|t| t.item.clone())
            .collect()
    }

    // ── Actions ───────────────────────────────────────────────────────────────

    /// Send primary activation (`Activate(0, 0)`) to the item.
    pub async fn activate(&self, item_id: &str) -> Result<(), TraydError> {
        let (service, path) = self.resolve(item_id)?;
        let proxy = item_proxy(&self.conn, &service, &path).await?;
        proxy.activate(0, 0).await?;
        Ok(())
    }

    /// Send secondary activation (`SecondaryActivate(0, 0)`) to the item.
    pub async fn secondary_activate(&self, item_id: &str) -> Result<(), TraydError> {
        let (service, path) = self.resolve(item_id)?;
        let proxy = item_proxy(&self.conn, &service, &path).await?;
        proxy.secondary_activate(0, 0).await?;
        Ok(())
    }

    /// Send a scroll event to the item.
    pub async fn scroll(
        &self,
        item_id: &str,
        direction: ScrollDirection,
        delta: i32,
    ) -> Result<(), TraydError> {
        let (service, path) = self.resolve(item_id)?;
        let proxy = item_proxy(&self.conn, &service, &path).await?;
        let orientation = scroll_orientation(direction);
        let signed_delta = scroll_delta(direction, delta);
        proxy.scroll(signed_delta, orientation).await?;
        Ok(())
    }

    /// Fetch the best-fit pixmap for `item_id` at approximately `size` pixels.
    pub async fn get_pixmap(&self, item_id: &str, size: u32) -> Result<Pixmap, TraydError> {
        let (service, path) = self.resolve(item_id)?;
        let proxy = item_proxy(&self.conn, &service, &path).await?;

        let frames = proxy.icon_pixmap().await.unwrap_or_default();

        pick_pixmap(frames, size).ok_or_else(|| TraydError::NoPixmap(item_id.to_owned()))
    }

    // ── Subscribe ─────────────────────────────────────────────────────────────

    /// Subscribe to a stream of [`HostEvent`]s.  Each call creates an
    /// independent receiver; up to 256 events are buffered before lagging.
    pub fn subscribe(&self) -> broadcast::Receiver<HostEvent> {
        self.event_tx.subscribe()
    }

    // ── Internals ─────────────────────────────────────────────────────────────

    fn resolve(&self, item_id: &str) -> Result<(String, String), TraydError> {
        // Fast path: grab the (service, path) while holding the lock briefly.
        // We can't hold the lock across an await, so we copy the strings out.
        let state = self
            .state
            .try_lock()
            .map_err(|_| TraydError::Internal("state lock contended; retry".to_owned()))?;
        let tracked = state
            .items
            .get(item_id)
            .ok_or_else(|| TraydError::NotFound(item_id.to_owned()))?;
        Ok((tracked.service.clone(), tracked.path.clone()))
    }

    /// Return the [`ItemId`]s of all currently registered items.
    ///
    /// Exposed for testing; production callers should use [`Self::list`].
    pub async fn item_ids(&self) -> Vec<ItemId> {
        self.state
            .lock()
            .await
            .items
            .values()
            .map(|t| t.item_id.clone())
            .collect()
    }
}
