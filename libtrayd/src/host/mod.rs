//! [`TrayHost`]: D-Bus SNI host, in-memory item cache, and event broadcasting.
//!
//! # Architecture
//!
//! ```text
//! TrayHost (cheap clone, Arc-backed)
//!   ├── state: RwLock<HostState>        — item cache
//!   ├── events_tx: broadcast::Sender    — HostEvent fan-out
//!   └── conn: zbus::Connection          — session bus (Arc-backed, cheap clone)
//!
//! Background task (spawned by start())
//!   ├── Watcher → mpsc → handle item registrations (fetch props inline)
//!   └── NameOwnerChanged stream → handle item removals
//! ```

use std::{collections::HashMap, sync::Arc};

use tokio::sync::{RwLock, broadcast, mpsc};
use tokio_stream::StreamExt as _;
use tracing::{debug, error, info, warn};

use crate::{
    TraydError,
    dbus::{StatusNotifierItemProxy, StatusNotifierWatcher, WatcherMsg},
    model::{HostEvent, IconData, IconPixmap, ItemId, TrayItem, TrayStatus},
};

// ─── Constants ───────────────────────────────────────────────────────────────

const SNI_WATCHER_NAME: &str = "org.kde.StatusNotifierWatcher";
const SNI_WATCHER_PATH: &str = "/StatusNotifierWatcher";
/// Capacity of the HostEvent broadcast channel.
const EVENTS_CAPACITY: usize = 64;

// ─── Internal state ──────────────────────────────────────────────────────────

struct HostState {
    items: HashMap<ItemId, TrayItem>,
}

impl HostState {
    fn new() -> Self {
        Self {
            items: HashMap::new(),
        }
    }
}

// ─── TrayHostInner ────────────────────────────────────────────────────────────

struct TrayHostInner {
    state: RwLock<HostState>,
    events_tx: broadcast::Sender<HostEvent>,
    conn: zbus::Connection,
}

// ─── TrayHost ─────────────────────────────────────────────────────────────────

/// The SNI tray host: registers on D-Bus, maintains an item cache, and fans out
/// [`HostEvent`]s to subscribers.
///
/// Cheap to clone — all state lives behind an `Arc`.
#[derive(Clone)]
pub struct TrayHost {
    inner: Arc<TrayHostInner>,
}

impl TrayHost {
    /// Connect to the session bus, claim `org.kde.StatusNotifierWatcher`, and
    /// start the background D-Bus event loop.
    ///
    /// # Errors
    ///
    /// Returns `Err(TraydError::DBus(_))` if the session bus is unreachable or
    /// the watcher name is already taken by another process.
    pub async fn start() -> Result<Self, TraydError> {
        let conn = zbus::Connection::session().await?;

        // Claim the SNI watcher well-known name.  Fails with NameTaken if
        // another trayd instance is already running.
        conn.request_name(SNI_WATCHER_NAME).await?;
        info!("claimed D-Bus name {SNI_WATCHER_NAME}");

        let (events_tx, _) = broadcast::channel(EVENTS_CAPACITY);

        let inner = Arc::new(TrayHostInner {
            state: RwLock::new(HostState::new()),
            events_tx: events_tx.clone(),
            conn: conn.clone(),
        });

        // Internal channel: watcher D-Bus object → host background loop.
        let (watcher_tx, watcher_rx) = mpsc::channel::<WatcherMsg>(32);
        let watcher = StatusNotifierWatcher::new(watcher_tx);

        // Register the watcher D-Bus object.
        conn.object_server().at(SNI_WATCHER_PATH, watcher).await?;
        info!("StatusNotifierWatcher registered at {SNI_WATCHER_PATH}");

        let host = TrayHost {
            inner: Arc::clone(&inner),
        };

        // Spawn the background loop.  It owns `conn` (cheap clone).
        let bg_conn = conn.clone();
        let bg_inner = Arc::clone(&inner);
        tokio::spawn(run_dbus_loop(bg_conn, bg_inner, watcher_rx));

        Ok(host)
    }

    // ── Public API ────────────────────────────────────────────────────────────

    /// Subscribe to [`HostEvent`]s.
    ///
    /// Buffered events are delivered as long as the receiver is alive.
    /// Use [`broadcast::Receiver::resubscribe`] to get a fresh receiver.
    pub fn subscribe(&self) -> broadcast::Receiver<HostEvent> {
        self.inner.events_tx.subscribe()
    }

    /// Snapshot of all currently cached items.
    pub async fn items(&self) -> Vec<TrayItem> {
        self.inner
            .state
            .read()
            .await
            .items
            .values()
            .cloned()
            .collect()
    }

    /// Look up a single item by id.
    pub async fn item(&self, id: &ItemId) -> Option<TrayItem> {
        self.inner.state.read().await.items.get(id).cloned()
    }

    /// Fetch raw ARGB32 pixmap bytes for an item at approximately `size` pixels.
    ///
    /// Fetches directly from D-Bus on every call — Phase 5 adds caching.
    ///
    /// # Errors
    ///
    /// - [`TraydError::NotFound`] if the item or its pixmap data is absent.
    /// - [`TraydError::DBus`] on transport failure.
    pub async fn get_pixmap(&self, id: &ItemId, size: u16) -> Result<Vec<u8>, TraydError> {
        let (bus_name, object_path) = {
            let state = self.inner.state.read().await;
            let item = state
                .items
                .get(id)
                .ok_or_else(|| TraydError::NotFound(id.to_string()))?;
            (item.bus_name.clone(), item.object_path.clone())
        };

        let proxy = build_proxy(&self.inner.conn, &bus_name, &object_path).await?;

        let pixmaps = proxy.icon_pixmap().await.unwrap_or_default();

        if pixmaps.is_empty() {
            return Err(TraydError::NotFound(format!("no pixmap for {id}")));
        }

        // Pick the nearest size; prefer an exact match, then larger, then smaller.
        let best = pixmaps
            .iter()
            .min_by_key(|(w, _, _)| (*w - size as i32).unsigned_abs())
            .expect("pixmaps is non-empty");

        Ok(best.2.clone())
    }

    /// Activate a tray item.
    ///
    /// - `item_id == 0` → primary `StatusNotifierItem.Activate(0, 0)` (D-Bus method).
    /// - `item_id > 0` → DBusMenu event (Phase 3; returns [`TraydError::NotImplemented`]).
    ///
    /// # Errors
    ///
    /// - [`TraydError::NotFound`] if the item is not in the cache.
    /// - [`TraydError::NotImplemented`] for menu item ids (Phase 3+).
    /// - [`TraydError::ActivationFailed`] / [`TraydError::DBus`] on D-Bus error.
    pub async fn activate(&self, id: &ItemId, item_id: u32) -> Result<(), TraydError> {
        if item_id > 0 {
            return Err(TraydError::NotImplemented);
        }

        let (bus_name, object_path) = {
            let state = self.inner.state.read().await;
            let item = state
                .items
                .get(id)
                .ok_or_else(|| TraydError::NotFound(id.to_string()))?;
            (item.bus_name.clone(), item.object_path.clone())
        };

        let proxy = build_proxy(&self.inner.conn, &bus_name, &object_path).await?;

        proxy
            .activate(0, 0)
            .await
            .map_err(|e| TraydError::ActivationFailed {
                app_id: id.to_string(),
                reason: e.to_string(),
            })
    }
}

// ─── Proxy helper ─────────────────────────────────────────────────────────────

async fn build_proxy<'c>(
    conn: &'c zbus::Connection,
    bus_name: &'c str,
    object_path: &'c str,
) -> Result<StatusNotifierItemProxy<'c>, TraydError> {
    StatusNotifierItemProxy::builder(conn)
        .destination(bus_name)?
        .path(object_path)?
        .build()
        .await
        .map_err(TraydError::DBus)
}

// ─── Background D-Bus loop ────────────────────────────────────────────────────

/// Runs until the watcher channel is closed.  Handles item registrations and
/// bus name disappearances.
async fn run_dbus_loop(
    conn: zbus::Connection,
    inner: Arc<TrayHostInner>,
    mut watcher_rx: mpsc::Receiver<WatcherMsg>,
) {
    // Set up NameOwnerChanged monitoring for item removal detection.
    let fdo = match zbus::fdo::DBusProxy::new(&conn).await {
        Ok(p) => p,
        Err(e) => {
            error!(%e, "failed to create FDO proxy; item removal tracking disabled");
            // Still process registrations, just without removal tracking.
            while let Some(msg) = watcher_rx.recv().await {
                handle_item_registered(&conn, &inner, msg).await;
            }
            return;
        }
    };

    let name_changes = match fdo.receive_name_owner_changed().await {
        Ok(s) => s,
        Err(e) => {
            error!(%e, "failed to subscribe to NameOwnerChanged; removal tracking disabled");
            while let Some(msg) = watcher_rx.recv().await {
                handle_item_registered(&conn, &inner, msg).await;
            }
            return;
        }
    };
    tokio::pin!(name_changes);

    loop {
        tokio::select! {
            msg = watcher_rx.recv() => {
                match msg {
                    Some(watcher_msg) => handle_item_registered(&conn, &inner, watcher_msg).await,
                    None => {
                        info!("watcher channel closed, stopping host loop");
                        break;
                    }
                }
            }
            Some(sig) = name_changes.next() => {
                handle_name_owner_changed(&inner, sig).await;
            }
        }
    }
}

// ─── Registration handler ─────────────────────────────────────────────────────

async fn handle_item_registered(
    conn: &zbus::Connection,
    inner: &Arc<TrayHostInner>,
    msg: WatcherMsg,
) {
    let WatcherMsg::ItemRegistered {
        service_id,
        bus_name,
        object_path,
    } = msg;
    debug!(%service_id, %bus_name, %object_path, "fetching SNI item properties");

    let proxy = match build_proxy(conn, &bus_name, &object_path).await {
        Ok(p) => p,
        Err(e) => {
            warn!(%e, %bus_name, "failed to build StatusNotifierItem proxy");
            return;
        }
    };

    let item = fetch_item_properties(&proxy, &service_id, &bus_name, &object_path).await;
    let id = item.id.clone();

    inner.state.write().await.items.insert(id, item.clone());
    let _ = inner.events_tx.send(HostEvent::ItemAdded(item));
    debug!(%service_id, "item added to cache");
}

// ─── Name-owner-change handler ────────────────────────────────────────────────

async fn handle_name_owner_changed(inner: &Arc<TrayHostInner>, sig: zbus::fdo::NameOwnerChanged) {
    let args = match sig.args() {
        Ok(a) => a,
        Err(e) => {
            warn!(%e, "failed to parse NameOwnerChanged signal args");
            return;
        }
    };

    // new_owner is an Optional<UniqueName>; None (empty string on the wire)
    // means the name has no new owner → it disappeared.
    if args.new_owner.is_some() {
        return;
    }

    let gone = args.name.to_string();
    debug!(%gone, "bus name disappeared, checking for registered items");

    let mut state = inner.state.write().await;
    let ids_to_remove: Vec<ItemId> = state
        .items
        .values()
        .filter(|item| item.bus_name == gone)
        .map(|item| item.id.clone())
        .collect();

    for id in ids_to_remove {
        state.items.remove(&id);
        let _ = inner.events_tx.send(HostEvent::ItemRemoved(id.clone()));
        debug!(%id, %gone, "item removed from cache");
    }
}

// ─── Property fetch helper ────────────────────────────────────────────────────

async fn fetch_item_properties(
    proxy: &StatusNotifierItemProxy<'_>,
    service_id: &str,
    bus_name: &str,
    object_path: &str,
) -> TrayItem {
    let title = proxy.title().await.unwrap_or_default();
    let status = TrayStatus::from_dbus(&proxy.status().await.unwrap_or_default());
    let icon_name = proxy.icon_name().await.unwrap_or_default();
    let raw_pixmaps = proxy.icon_pixmap().await.unwrap_or_default();
    let menu_path = proxy
        .menu_path()
        .await
        .map(|p| p.to_string())
        .unwrap_or_default();

    let pixmaps = raw_pixmaps
        .into_iter()
        .map(|(w, h, data)| IconPixmap {
            width: w,
            height: h,
            data,
        })
        .collect();

    TrayItem {
        id: ItemId(service_id.to_owned()),
        bus_name: bus_name.to_owned(),
        object_path: object_path.to_owned(),
        title,
        status,
        icon: IconData {
            name: icon_name,
            pixmaps,
        },
        menu_path,
    }
}

#[cfg(test)]
mod tests;
