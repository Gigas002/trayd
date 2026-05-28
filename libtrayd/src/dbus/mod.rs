//! Session D-Bus: StatusNotifierWatcher server + StatusNotifierItem proxy.
//!
//! Both `WatcherIface` and `StatusNotifierItemProxy` are internal to the crate.
//! External callers use [`crate::TrayHost`].

pub(crate) mod menu;

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{Mutex, broadcast};
use tokio_stream::StreamExt;
use tracing::{debug, warn};
use zbus::fdo;
use zbus::object_server::SignalEmitter;

use crate::error::TraydError;
use crate::model::{HostEvent, Item, ItemId, ItemStatus, Pixmap, PixmapFormat, ScrollDirection};

#[cfg(test)]
mod tests;

// ── SNI item proxy ────────────────────────────────────────────────────────────

/// Proxy for a registered `org.kde.StatusNotifierItem` instance.
#[zbus::proxy(
    interface = "org.kde.StatusNotifierItem",
    default_path = "/StatusNotifierItem",
    gen_blocking = false
)]
pub(crate) trait StatusNotifierItem {
    fn activate(&self, x: i32, y: i32) -> zbus::Result<()>;
    fn secondary_activate(&self, x: i32, y: i32) -> zbus::Result<()>;
    fn scroll(&self, delta: i32, orientation: &str) -> zbus::Result<()>;

    #[zbus(property)]
    fn category(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn id(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn title(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn status(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn icon_name(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn icon_pixmap(&self) -> zbus::Result<Vec<(i32, i32, Vec<u8>)>>;
    #[zbus(property)]
    fn attention_icon_name(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn attention_icon_pixmap(&self) -> zbus::Result<Vec<(i32, i32, Vec<u8>)>>;
    /// `(icon_name, icon_pixmaps, title, description)`
    #[zbus(property)]
    fn tool_tip(&self) -> zbus::Result<(String, Vec<(i32, i32, Vec<u8>)>, String, String)>;
    /// Object path of the item's `com.canonical.dbusmenu` menu (if any).
    #[zbus(property)]
    fn menu(&self) -> zbus::Result<zbus::zvariant::OwnedObjectPath>;
}

// ── Shared watcher state ──────────────────────────────────────────────────────

/// A registered item with the metadata needed to manage its lifecycle.
#[derive(Debug)]
pub(crate) struct TrackedItem {
    /// IPC-facing identity: `{service}{path}`.
    pub item_id: ItemId,
    /// D-Bus service name used to build the proxy.
    pub service: String,
    /// Object path (typically `/StatusNotifierItem`).
    pub path: String,
    /// Unique bus name of the registrant (`:1.xxx`), used for departure detection.
    pub unique_sender: String,
    /// Last-known properties.
    pub item: Item,
    /// Object path of the `com.canonical.dbusmenu` menu, if the item has one.
    pub menu_path: Option<String>,
}

/// Shared mutable state of the SNI watcher.
#[derive(Debug, Default)]
pub(crate) struct WatcherState {
    /// Items keyed by `item_id` string.
    pub items: HashMap<String, TrackedItem>,
    pub host_registered: bool,
}

// ── StatusNotifierWatcher D-Bus interface ─────────────────────────────────────

/// D-Bus interface object registered at `/StatusNotifierWatcher`.
pub(crate) struct WatcherIface {
    state: Arc<Mutex<WatcherState>>,
    conn: zbus::Connection,
    event_tx: broadcast::Sender<HostEvent>,
}

impl WatcherIface {
    pub fn new(
        state: Arc<Mutex<WatcherState>>,
        conn: zbus::Connection,
        event_tx: broadcast::Sender<HostEvent>,
    ) -> Self {
        Self {
            state,
            conn,
            event_tx,
        }
    }

    async fn read_item(
        conn: &zbus::Connection,
        service: &str,
        path: &str,
        item_id: &ItemId,
    ) -> Item {
        let proxy_result = StatusNotifierItemProxy::builder(conn)
            .destination(service)
            .and_then(|b| b.path(path))
            .map(|b| b.build());

        let proxy = match proxy_result {
            Ok(future) => match future.await {
                Ok(p) => p,
                Err(e) => {
                    warn!(item = %item_id, error = %e, "failed to build item proxy");
                    return Item {
                        id: item_id.clone(),
                        title: None,
                        status: ItemStatus::Active,
                        has_attention_icon: false,
                        tooltip: None,
                        category: None,
                        icon_name: None,
                    };
                }
            },
            Err(e) => {
                warn!(item = %item_id, error = %e, "invalid proxy destination/path");
                return Item {
                    id: item_id.clone(),
                    title: None,
                    status: ItemStatus::Active,
                    has_attention_icon: false,
                    tooltip: None,
                    category: None,
                    icon_name: None,
                };
            }
        };

        let title = proxy.title().await.ok().filter(|s| !s.is_empty());
        let status = proxy
            .status()
            .await
            .map(|s| ItemStatus::from_sni_str(&s))
            .unwrap_or_default();
        let category = proxy.category().await.ok().filter(|s| !s.is_empty());
        let icon_name = proxy.icon_name().await.ok().filter(|s| !s.is_empty());
        let has_attention_icon = proxy
            .attention_icon_pixmap()
            .await
            .map(|v| !v.is_empty())
            .unwrap_or(false)
            || proxy
                .attention_icon_name()
                .await
                .map(|s| !s.is_empty())
                .unwrap_or(false);
        let tooltip = proxy.tool_tip().await.ok().and_then(|(_, _, title, desc)| {
            let t = if !title.is_empty() { title } else { desc };
            if t.is_empty() { None } else { Some(t) }
        });

        Item {
            id: item_id.clone(),
            title,
            status,
            has_attention_icon,
            tooltip,
            category,
            icon_name,
        }
    }
}

#[zbus::interface(name = "org.kde.StatusNotifierWatcher")]
impl WatcherIface {
    async fn register_status_notifier_item(
        &self,
        service: String,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> fdo::Result<()> {
        let unique_sender = header.sender().map(|n| n.to_string()).unwrap_or_default();

        // Determine destination and path from the registration string.
        let (dest, path) = if service.starts_with('/') {
            (unique_sender.clone(), service.clone())
        } else {
            (service.clone(), "/StatusNotifierItem".to_owned())
        };

        let item_id = ItemId(format!("{dest}{path}"));

        debug!(
            item = %item_id,
            sender = %unique_sender,
            "RegisterStatusNotifierItem"
        );

        let item = Self::read_item(&self.conn, &dest, &path, &item_id).await;
        let menu_path = read_menu_path(&self.conn, &dest, &path).await;

        {
            let mut state = self.state.lock().await;
            state.items.insert(
                item_id.0.clone(),
                TrackedItem {
                    item_id: item_id.clone(),
                    service: dest.clone(),
                    path: path.clone(),
                    unique_sender: unique_sender.clone(),
                    item: item.clone(),
                    menu_path: menu_path.clone(),
                },
            );
        }

        if let Some(ref mp) = menu_path {
            spawn_menu_monitor(
                self.conn.clone(),
                dest.clone(),
                mp.clone(),
                item_id.clone(),
                self.event_tx.clone(),
            );
        }

        let _ = self.event_tx.send(HostEvent::ItemAdded(item));

        Self::status_notifier_item_registered(&emitter, &format!("{dest}{path}"))
            .await
            .ok();

        Ok(())
    }

    async fn register_status_notifier_host(
        &self,
        _service: String,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> fdo::Result<()> {
        {
            let mut state = self.state.lock().await;
            state.host_registered = true;
        }
        Self::status_notifier_host_registered(&emitter).await.ok();
        Ok(())
    }

    #[zbus(property)]
    async fn registered_status_notifier_items(&self) -> Vec<String> {
        self.state
            .lock()
            .await
            .items
            .values()
            .map(|t| format!("{}{}", t.service, t.path))
            .collect()
    }

    #[zbus(property)]
    async fn is_status_notifier_host_registered(&self) -> bool {
        self.state.lock().await.host_registered
    }

    #[zbus(property)]
    async fn protocol_version(&self) -> i32 {
        0
    }

    #[zbus(signal)]
    async fn status_notifier_item_registered(
        emitter: &SignalEmitter<'_>,
        service: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn status_notifier_item_unregistered(
        emitter: &SignalEmitter<'_>,
        service: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn status_notifier_host_registered(ctxt: &SignalEmitter<'_>) -> zbus::Result<()>;
}

// ── Menu helpers ──────────────────────────────────────────────────────────────

/// Fetch the `Menu` property (object path) from a StatusNotifierItem.
///
/// Returns `None` if the item does not expose a menu or the property call fails.
pub(crate) async fn read_menu_path(
    conn: &zbus::Connection,
    service: &str,
    path: &str,
) -> Option<String> {
    let proxy = StatusNotifierItemProxy::builder(conn)
        .destination(service)
        .ok()?
        .path(path)
        .ok()?
        .build()
        .await
        .ok()?;
    let op = proxy.menu().await.ok()?;
    let s = op.as_str();
    // Treat "/" (no-menu sentinel) as absent.
    if s.is_empty() || s == "/" {
        None
    } else {
        Some(s.to_owned())
    }
}

/// Spawn a background task that forwards `LayoutUpdated` signals from the
/// item's DBusMenu into `HostEvent::MenuChanged` events.
pub(crate) fn spawn_menu_monitor(
    conn: zbus::Connection,
    service: String,
    menu_path: String,
    item_id: ItemId,
    event_tx: broadcast::Sender<HostEvent>,
) {
    tokio::spawn(async move {
        if let Err(e) = run_menu_monitor(conn, service, menu_path, item_id, event_tx).await {
            warn!(error = %e, "menu monitor exited with error");
        }
    });
}

async fn run_menu_monitor(
    conn: zbus::Connection,
    service: String,
    menu_path: String,
    item_id: ItemId,
    event_tx: broadcast::Sender<HostEvent>,
) -> Result<(), TraydError> {
    let proxy = menu::menu_proxy(&conn, &service, &menu_path).await?;
    let mut stream = proxy.receive_layout_updated().await?;
    while stream.next().await.is_some() {
        debug!(item = %item_id, "menu LayoutUpdated");
        let _ = event_tx.send(HostEvent::MenuChanged(item_id.clone()));
    }
    Ok(())
}

// ── Pixmap helpers ────────────────────────────────────────────────────────────

/// Pick the best-fit pixmap from an SNI `a(iiay)` array for the requested size.
///
/// Selects the frame whose longer dimension is closest to (and at least)
/// `target_size`. Falls back to the largest available frame.
pub(crate) fn pick_pixmap(frames: Vec<(i32, i32, Vec<u8>)>, target_size: u32) -> Option<Pixmap> {
    if frames.is_empty() {
        return None;
    }
    let target = target_size as i32;
    let best = frames
        .iter()
        .min_by_key(|(w, h, _)| {
            let dim = (*w).max(*h);
            let diff = dim - target;
            if diff >= 0 {
                diff
            } else {
                -diff + i32::MAX / 2
            }
        })
        .or_else(|| frames.first());

    best.map(|(w, h, data)| Pixmap {
        format: PixmapFormat::Argb32,
        width: *w as u32,
        height: *h as u32,
        data: data.clone(),
    })
}

// ── Name departure monitor ────────────────────────────────────────────────────

/// Spawns a background task that removes items when their bus name disappears.
pub(crate) fn spawn_name_monitor(
    conn: zbus::Connection,
    state: Arc<Mutex<WatcherState>>,
    event_tx: broadcast::Sender<HostEvent>,
) {
    tokio::spawn(async move {
        if let Err(e) = run_name_monitor(conn, state, event_tx).await {
            warn!(error = %e, "SNI name monitor exited with error");
        }
    });
}

async fn run_name_monitor(
    conn: zbus::Connection,
    state: Arc<Mutex<WatcherState>>,
    event_tx: broadcast::Sender<HostEvent>,
) -> Result<(), TraydError> {
    use tokio_stream::StreamExt;

    let dbus = zbus::fdo::DBusProxy::new(&conn).await?;
    let mut changes = dbus.receive_name_owner_changed().await?;

    while let Some(signal) = changes.next().await {
        let args = match signal.args() {
            Ok(a) => a,
            Err(e) => {
                warn!(error = %e, "NameOwnerChanged args failed");
                continue;
            }
        };

        // Only care about name releases (new_owner is empty).
        let new_owner = args.new_owner();
        if new_owner.as_deref().map(|s| s.is_empty()).unwrap_or(true) {
            let lost_name = args.name().to_string();
            remove_items_by_name(&lost_name, &state, &event_tx).await;
        }
    }

    Ok(())
}

async fn remove_items_by_name(
    name: &str,
    state: &Arc<Mutex<WatcherState>>,
    event_tx: &broadcast::Sender<HostEvent>,
) {
    let mut st = state.lock().await;
    let removed: Vec<String> = st
        .items
        .iter()
        .filter(|(_, t)| t.unique_sender == name || t.service == name)
        .map(|(k, _)| k.clone())
        .collect();

    for key in removed {
        if let Some(tracked) = st.items.remove(&key) {
            debug!(item = %tracked.item_id, name = %name, "item departed");
            let _ = event_tx.send(HostEvent::ItemRemoved(tracked.item_id));
        }
    }
}

// ── Proxy builder helper ──────────────────────────────────────────────────────

/// Build a [`StatusNotifierItemProxy`] for an already-tracked item.
pub(crate) async fn item_proxy<'a>(
    conn: &'a zbus::Connection,
    service: &'a str,
    path: &'a str,
) -> Result<StatusNotifierItemProxy<'a>, TraydError> {
    Ok(StatusNotifierItemProxy::builder(conn)
        .destination(service)?
        .path(path)?
        .build()
        .await?)
}

/// Convert an SNI orientation string to [`ScrollDirection`].
pub(crate) fn scroll_orientation(dir: ScrollDirection) -> &'static str {
    match dir {
        ScrollDirection::Up | ScrollDirection::Down => "vertical",
        ScrollDirection::Left | ScrollDirection::Right => "horizontal",
    }
}

/// Convert a [`ScrollDirection`] to a signed delta (up/right positive).
pub(crate) fn scroll_delta(dir: ScrollDirection, delta: i32) -> i32 {
    match dir {
        ScrollDirection::Up | ScrollDirection::Right => delta.abs(),
        ScrollDirection::Down | ScrollDirection::Left => -delta.abs(),
    }
}
