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

use zbus::zvariant::OwnedValue;

use crate::{
    TraydError,
    dbus::{DBusMenuProxy, StatusNotifierItemProxy, StatusNotifierWatcher, WatcherMsg},
    model::{
        HostEvent, IconData, IconPixmap, ItemId, MenuNode, PixmapData, ToolTip, TrayItem,
        TrayStatus,
    },
};

// ─── Constants ───────────────────────────────────────────────────────────────

const SNI_WATCHER_NAME: &str = "org.kde.StatusNotifierWatcher";
const SNI_WATCHER_PATH: &str = "/StatusNotifierWatcher";
/// Capacity of the HostEvent broadcast channel.
const EVENTS_CAPACITY: usize = 64;

// ─── Internal state ──────────────────────────────────────────────────────────

struct HostState {
    items: HashMap<ItemId, TrayItem>,
    /// Derived pixmap cache: `(app_id, requested_size_px)` → best-matched ARGB32 bytes.
    /// Invalidated on icon change signals and item removal.
    pixmap_cache: HashMap<(ItemId, u16), PixmapData>,
}

impl HostState {
    fn new() -> Self {
        Self {
            items: HashMap::new(),
            pixmap_cache: HashMap::new(),
        }
    }

    /// Remove all pixmap cache entries for `id`.
    fn invalidate_pixmap_cache(&mut self, id: &ItemId) {
        self.pixmap_cache.retain(|(item_id, _), _| item_id != id);
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
        // Internal channel: watcher D-Bus object → host background loop.
        let (watcher_tx, watcher_rx) = mpsc::channel::<WatcherMsg>(32);
        let watcher = StatusNotifierWatcher::new(watcher_tx);

        // Register the object and claim the name atomically via Builder so no
        // method calls arrive before the interface is ready.
        let conn = zbus::connection::Builder::session()?
            .serve_at(SNI_WATCHER_PATH, watcher)?
            .name(SNI_WATCHER_NAME)?
            .build()
            .await?;
        info!("claimed D-Bus name {SNI_WATCHER_NAME}; watcher at {SNI_WATCHER_PATH}");

        let (events_tx, _) = broadcast::channel(EVENTS_CAPACITY);

        let inner = Arc::new(TrayHostInner {
            state: RwLock::new(HostState::new()),
            events_tx: events_tx.clone(),
            conn: conn.clone(),
        });

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
    /// Uses the in-process item cache; no D-Bus round-trip after the initial
    /// registration fetch.  Results are memoised in a derived pixmap cache
    /// keyed by `(app_id, size)` and invalidated when icon or status signals
    /// arrive.
    ///
    /// When the item's status is [`TrayStatus::NeedsAttention`] and the item
    /// has attention pixmap data, that is served instead of the normal icon.
    ///
    /// # Errors
    ///
    /// - [`TraydError::NotFound`] if the item or its pixmap data is absent.
    pub async fn get_pixmap(&self, id: &ItemId, size: u16) -> Result<PixmapData, TraydError> {
        // Fast path: return from derived pixmap cache.
        {
            let state = self.inner.state.read().await;
            if let Some(cached) = state.pixmap_cache.get(&(id.clone(), size)) {
                return Ok(cached.clone());
            }
        }

        // Slow path: pick the best pixmap from the in-process item cache.
        let result = {
            let state = self.inner.state.read().await;
            let item = state
                .items
                .get(id)
                .ok_or_else(|| TraydError::NotFound(id.to_string()))?;

            // Prefer attention icon when the item signals urgency and has pixmaps.
            let icon = if item.status == TrayStatus::NeedsAttention
                && !item.attention_icon.pixmaps.is_empty()
            {
                &item.attention_icon
            } else {
                &item.icon
            };

            if icon.pixmaps.is_empty() {
                return Err(TraydError::NotFound(format!("no pixmap for {id}")));
            }

            // Pick the nearest size; prefer an exact match, then larger, then smaller.
            let best = icon
                .pixmaps
                .iter()
                .min_by_key(|p| (p.width - size as i32).unsigned_abs())
                .expect("pixmaps is non-empty");

            PixmapData {
                width: best.width as u32,
                height: best.height as u32,
                data: best.data.clone(),
            }
        };

        // Memoise for subsequent calls with the same size.
        {
            let mut state = self.inner.state.write().await;
            state
                .pixmap_cache
                .insert((id.clone(), size), result.clone());
        }

        Ok(result)
    }

    /// Activate a tray item.
    ///
    /// - `item_id == 0` → primary `StatusNotifierItem.Activate(0, 0)` (D-Bus method).
    /// - `item_id > 0` → `DBusMenu.Event(item_id, "clicked", …)` on the item's menu.
    ///
    /// # Errors
    ///
    /// - [`TraydError::NotFound`] if the item is not in the cache or has no menu.
    /// - [`TraydError::ActivationFailed`] if `item_id == 0` and the item is menu-only
    ///   ([`TrayItem::item_is_menu`] is `true`), or on D-Bus activation error.
    /// - [`TraydError::DBus`] on D-Bus transport error.
    pub async fn activate(&self, id: &ItemId, item_id: u32) -> Result<(), TraydError> {
        if item_id == 0 {
            let (bus_name, object_path, item_is_menu) = {
                let state = self.inner.state.read().await;
                let item = state
                    .items
                    .get(id)
                    .ok_or_else(|| TraydError::NotFound(id.to_string()))?;
                (
                    item.bus_name.clone(),
                    item.object_path.clone(),
                    item.item_is_menu,
                )
            };

            if item_is_menu {
                return Err(TraydError::ActivationFailed {
                    app_id: id.to_string(),
                    reason: "item is menu-only; use get_menu / activate with item_id > 0"
                        .to_owned(),
                });
            }

            let proxy = build_proxy(&self.inner.conn, &bus_name, &object_path).await?;
            proxy
                .activate(0, 0)
                .await
                .map_err(|e| TraydError::ActivationFailed {
                    app_id: id.to_string(),
                    reason: e.to_string(),
                })
        } else {
            let (bus_name, menu_path) = {
                let state = self.inner.state.read().await;
                let item = state
                    .items
                    .get(id)
                    .ok_or_else(|| TraydError::NotFound(id.to_string()))?;
                if item.menu_path.is_empty() || item.menu_path == "/" {
                    return Err(TraydError::NotFound(format!("{id} has no menu")));
                }
                (item.bus_name.clone(), item.menu_path.clone())
            };

            let proxy = build_menu_proxy(&self.inner.conn, &bus_name, &menu_path).await?;
            proxy
                .event(
                    item_id as i32,
                    "clicked",
                    zbus::zvariant::Value::from(0i32),
                    0,
                )
                .await
                .map_err(|e| TraydError::ActivationFailed {
                    app_id: id.to_string(),
                    reason: e.to_string(),
                })
        }
    }

    /// Fetch the direct children of a menu node for the given tray item.
    ///
    /// - `submenu_id = None` fetches the top-level menu (parent id `0`).
    /// - `submenu_id = Some(n)` fetches children of the submenu with DBusMenu id `n`.
    ///
    /// Returns **direct children only** (one level deep via `GetLayout` depth=1).
    ///
    /// # Errors
    ///
    /// - [`TraydError::NotFound`] if the item is absent or has no associated menu.
    /// - [`TraydError::DBus`] on transport failure.
    pub async fn get_menu(
        &self,
        id: &ItemId,
        submenu_id: Option<u32>,
    ) -> Result<Vec<MenuNode>, TraydError> {
        let (bus_name, menu_path) = {
            let state = self.inner.state.read().await;
            let item = state
                .items
                .get(id)
                .ok_or_else(|| TraydError::NotFound(id.to_string()))?;
            if item.menu_path.is_empty() || item.menu_path == "/" {
                return Err(TraydError::NotFound(format!("{id} has no menu")));
            }
            (item.bus_name.clone(), item.menu_path.clone())
        };

        let proxy = build_menu_proxy(&self.inner.conn, &bus_name, &menu_path).await?;
        let parent_id = submenu_id.map(|n| n as i32).unwrap_or(0);

        let (_revision, (_root_id, _root_props, children)) = proxy
            .get_layout(parent_id, 1, &[])
            .await
            .map_err(TraydError::DBus)?;

        Ok(menu_nodes_from_av(children))
    }
}

// ─── Proxy helpers ───────────────────────────────────────────────────────────

/// Given a unique D-Bus bus name (e.g. `:1.201`), return the first well-known
/// name belonging to the same process (matched by Unix PID).
///
/// This handles apps like Telegram that register SNI on one connection while
/// their well-known name (e.g. `org.telegram.desktop`) is claimed on another.
///
/// Returns `None` when the input is already a well-known name, no match is
/// found, or any D-Bus call fails.
async fn lookup_well_known_name(conn: &zbus::Connection, unique_name: &str) -> Option<String> {
    if !unique_name.starts_with(':') {
        return None; // Already a well-known name — nothing to look up.
    }
    let fdo = zbus::fdo::DBusProxy::new(conn).await.ok()?;
    // Resolve the PID of the SNI connection.
    let our_pid = fdo
        .get_connection_unix_process_id(zbus::names::BusName::try_from(unique_name).ok()?)
        .await
        .ok()?;
    let names = fdo.list_names().await.ok()?;
    for owned_name in names {
        let s = owned_name.to_string();
        if s.starts_with(':') {
            continue; // Skip unique names.
        }
        // Accept any well-known name whose owning connection shares our PID.
        if let Ok(pid) = fdo
            .get_connection_unix_process_id((*owned_name).clone())
            .await
            && pid == our_pid
        {
            return Some(s);
        }
    }
    None
}

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

async fn build_menu_proxy<'c>(
    conn: &'c zbus::Connection,
    bus_name: &str,
    menu_path: &str,
) -> Result<DBusMenuProxy<'c>, TraydError> {
    DBusMenuProxy::builder(conn)
        .destination(bus_name.to_owned())?
        .path(menu_path.to_owned())?
        .build()
        .await
        .map_err(TraydError::DBus)
}

// ─── Menu layout parsing ──────────────────────────────────────────────────────

/// Convert the `av` children array from a `GetLayout` response into [`MenuNode`]s.
///
/// Each `OwnedValue` in the array is a D-Bus variant containing
/// `(i32 id, a{sv} properties, av children)`.
pub(crate) fn menu_nodes_from_av(children: Vec<OwnedValue>) -> Vec<MenuNode> {
    children.into_iter().filter_map(parse_child_ov).collect()
}

fn parse_child_ov(ov: OwnedValue) -> Option<MenuNode> {
    // Deserialize the child structure (i32, a{sv}, av) via TryFrom.
    let (id, props, nested_children): (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>) =
        ov.try_into().ok()?;

    let label = get_str_prop(&props, "label");
    let enabled = get_bool_prop(&props, "enabled").unwrap_or(true);
    let visible = get_bool_prop(&props, "visible").unwrap_or(true);
    let icon_name = get_str_prop(&props, "icon-name");
    // A submenu is indicated by `children-display` in props, or non-empty children
    // (the latter occurs when GetLayout was called with depth > 1).
    let is_submenu = props.contains_key("children-display") || !nested_children.is_empty();
    let children = menu_nodes_from_av(nested_children);

    Some(MenuNode {
        id,
        label,
        enabled,
        visible,
        icon_name,
        is_submenu,
        children,
    })
}

pub(crate) fn get_str_prop(props: &HashMap<String, OwnedValue>, key: &str) -> String {
    props
        .get(key)
        .and_then(|v| String::try_from(v.clone()).ok())
        .unwrap_or_default()
}

pub(crate) fn get_bool_prop(props: &HashMap<String, OwnedValue>, key: &str) -> Option<bool> {
    props.get(key).and_then(|v| bool::try_from(v.clone()).ok())
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

    let well_known = lookup_well_known_name(conn, &bus_name).await;
    let item = fetch_item_properties(
        &proxy,
        &service_id,
        &bus_name,
        &object_path,
        well_known.as_deref(),
    )
    .await;
    let id = item.id.clone();

    inner.state.write().await.items.insert(id, item.clone());
    let _ = inner.events_tx.send(HostEvent::ItemAdded(item));
    debug!(%service_id, "item added to cache");

    // Spawn a per-item signal watcher to keep the cache fresh.
    tokio::spawn(run_item_signal_watcher(
        conn.clone(),
        Arc::clone(inner),
        ItemId(service_id.to_owned()),
        bus_name.to_owned(),
        object_path.to_owned(),
    ));
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
        state.invalidate_pixmap_cache(&id);
        let _ = inner.events_tx.send(HostEvent::ItemRemoved(id.clone()));
        debug!(%id, %gone, "item removed from cache");
    }
}

// ─── Property fetch helper ────────────────────────────────────────────────────

/// Select the best icon name from the three available sources.
///
/// Priority: explicit `IconName` SNI property → well-known bus name of the
/// owning process (e.g. `org.telegram.desktop`) → SNI `Id` as a last resort
/// (e.g. `TelegramDesktop`).
pub(crate) fn resolve_icon_name(
    icon_name: String,
    well_known_name: Option<&str>,
    sni_id: String,
) -> String {
    if !icon_name.is_empty() {
        icon_name
    } else if let Some(wkn) = well_known_name.filter(|s| !s.is_empty()) {
        wkn.to_owned()
    } else {
        sni_id
    }
}

async fn fetch_item_properties(
    proxy: &StatusNotifierItemProxy<'_>,
    service_id: &str,
    bus_name: &str,
    object_path: &str,
    well_known_name: Option<&str>,
) -> TrayItem {
    let title = proxy.title().await.unwrap_or_else(|e| {
        warn!(%e, %service_id, "failed to fetch title");
        String::new()
    });
    let status = TrayStatus::from_dbus(&proxy.status().await.unwrap_or_else(|e| {
        warn!(%e, %service_id, "failed to fetch status");
        String::new()
    }));
    let icon_name = proxy.icon_name().await.unwrap_or_else(|e| {
        warn!(%e, %service_id, "failed to fetch icon_name");
        String::new()
    });
    // Priority: explicit `IconName` → well-known bus name (e.g. `org.telegram.desktop`)
    // → SNI `Id` as a last resort (e.g. `TelegramDesktop`).
    let sni_id = proxy.id().await.unwrap_or_else(|e| {
        warn!(%e, %service_id, "failed to fetch id");
        String::new()
    });
    let icon_name = resolve_icon_name(icon_name, well_known_name, sni_id);
    let raw_pixmaps = proxy.icon_pixmap().await.unwrap_or_else(|e| {
        warn!(%e, %service_id, "failed to fetch icon_pixmap");
        Vec::new()
    });
    let category = proxy.category().await.unwrap_or_else(|e| {
        warn!(%e, %service_id, "failed to fetch category");
        String::new()
    });
    let item_is_menu = proxy.item_is_menu().await.unwrap_or_else(|e| {
        warn!(%e, %service_id, "failed to fetch item_is_menu");
        false
    });
    let raw_tool_tip = proxy.tool_tip().await.unwrap_or_else(|e| {
        warn!(%e, %service_id, "failed to fetch tool_tip");
        Default::default()
    });
    let overlay_icon_name = proxy.overlay_icon_name().await.unwrap_or_else(|e| {
        warn!(%e, %service_id, "failed to fetch overlay_icon_name");
        String::new()
    });
    let raw_overlay_pixmaps = proxy.overlay_icon_pixmap().await.unwrap_or_else(|e| {
        warn!(%e, %service_id, "failed to fetch overlay_icon_pixmap");
        Vec::new()
    });
    let attention_icon_name = proxy.attention_icon_name().await.unwrap_or_else(|e| {
        warn!(%e, %service_id, "failed to fetch attention_icon_name");
        String::new()
    });
    let raw_attention_pixmaps = proxy.attention_icon_pixmap().await.unwrap_or_else(|e| {
        warn!(%e, %service_id, "failed to fetch attention_icon_pixmap");
        Vec::new()
    });
    let menu_path = proxy
        .menu_path()
        .await
        .map(|p| p.to_string())
        .unwrap_or_else(|e| {
            warn!(%e, %service_id, "failed to fetch menu_path");
            String::new()
        });

    let tool_tip = ToolTip {
        icon_name: raw_tool_tip.0,
        icon_pixmaps: raw_tool_tip
            .1
            .into_iter()
            .map(|(w, h, data)| IconPixmap {
                width: w,
                height: h,
                data,
            })
            .collect(),
        title: raw_tool_tip.2,
        description: raw_tool_tip.3,
    };

    let overlay_icon = IconData {
        name: overlay_icon_name,
        pixmaps: raw_overlay_pixmaps
            .into_iter()
            .map(|(w, h, data)| IconPixmap {
                width: w,
                height: h,
                data,
            })
            .collect(),
    };

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
        category,
        item_is_menu,
        tool_tip,
        overlay_icon,
        icon: IconData {
            name: icon_name,
            pixmaps,
        },
        attention_icon: IconData {
            name: attention_icon_name,
            pixmaps: raw_attention_pixmaps
                .into_iter()
                .map(|(w, h, data)| IconPixmap {
                    width: w,
                    height: h,
                    data,
                })
                .collect(),
        },
        menu_path,
    }
}

// ─── Per-item signal watcher ──────────────────────────────────────────────────────────────────────────────────

/// Long-running per-item task: subscribes to SNI property-change signals and
/// keeps the in-process item cache — and derived pixmap cache — fresh.
///
/// The task exits naturally when all signal streams close (i.e. the app's bus
/// name disappears).
async fn run_item_signal_watcher(
    conn: zbus::Connection,
    inner: Arc<TrayHostInner>,
    id: ItemId,
    bus_name: String,
    object_path: String,
) {
    let proxy = match build_proxy(&conn, &bus_name, &object_path).await {
        Ok(p) => p,
        Err(e) => {
            warn!(%e, %id, "signal watcher: failed to build proxy");
            return;
        }
    };

    let new_icon = match proxy.receive_new_icon().await {
        Ok(s) => s,
        Err(e) => {
            warn!(%e, %id, "signal watcher: new_icon subscribe failed");
            return;
        }
    };
    let new_title = match proxy.receive_new_title().await {
        Ok(s) => s,
        Err(e) => {
            warn!(%e, %id, "signal watcher: new_title subscribe failed");
            return;
        }
    };
    let new_status = match proxy.receive_new_status().await {
        Ok(s) => s,
        Err(e) => {
            warn!(%e, %id, "signal watcher: new_status subscribe failed");
            return;
        }
    };
    let new_attention = match proxy.receive_new_attention_icon().await {
        Ok(s) => s,
        Err(e) => {
            warn!(%e, %id, "signal watcher: new_attention_icon subscribe failed");
            return;
        }
    };
    let new_overlay = match proxy.receive_new_overlay_icon().await {
        Ok(s) => s,
        Err(e) => {
            warn!(%e, %id, "signal watcher: new_overlay_icon subscribe failed");
            return;
        }
    };
    let new_menu = match proxy.receive_new_menu().await {
        Ok(s) => s,
        Err(e) => {
            warn!(%e, %id, "signal watcher: new_menu subscribe failed");
            return;
        }
    };
    let new_tool_tip = match proxy.receive_new_tool_tip().await {
        Ok(s) => s,
        Err(e) => {
            warn!(%e, %id, "signal watcher: new_tool_tip subscribe failed");
            return;
        }
    };

    tokio::pin!(
        new_icon,
        new_title,
        new_status,
        new_attention,
        new_overlay,
        new_menu,
        new_tool_tip
    );
    debug!(%id, "item signal watcher started");

    loop {
        tokio::select! {
            Some(_) = new_icon.next() => {
                on_icon_changed(&inner, &id, &proxy, false, &bus_name).await;
            }
            Some(_) = new_title.next() => {
                on_title_changed(&inner, &id, &proxy).await;
            }
            Some(sig) = new_status.next() => {
                if let Ok(args) = sig.args() {
                    on_status_changed(&inner, &id, TrayStatus::from_dbus(args.status())).await;
                }
            }
            Some(_) = new_attention.next() => {
                on_icon_changed(&inner, &id, &proxy, true, &bus_name).await;
            }
            Some(_) = new_overlay.next() => {
                on_overlay_icon_changed(&inner, &id, &proxy).await;
            }
            Some(_) = new_menu.next() => {
                on_menu_path_changed(&inner, &id, &proxy).await;
            }
            Some(_) = new_tool_tip.next() => {
                on_tool_tip_changed(&inner, &id, &proxy).await;
            }
            else => break,
        }
    }
    debug!(%id, "item signal watcher ended");
}

/// Re-fetch the normal or attention icon from D-Bus, update the item cache,
/// and invalidate the derived pixmap cache.
async fn on_icon_changed(
    inner: &Arc<TrayHostInner>,
    id: &ItemId,
    proxy: &StatusNotifierItemProxy<'_>,
    attention: bool,
    bus_name: &str,
) {
    let (icon_name, raw) = if attention {
        let name = proxy.attention_icon_name().await.unwrap_or_else(|e| {
            warn!(%e, %id, "failed to fetch attention_icon_name");
            String::new()
        });
        let pixmaps = proxy.attention_icon_pixmap().await.unwrap_or_else(|e| {
            warn!(%e, %id, "failed to fetch attention_icon_pixmap");
            Vec::new()
        });
        (name, pixmaps)
    } else {
        let name = proxy.icon_name().await.unwrap_or_else(|e| {
            warn!(%e, %id, "failed to fetch icon_name");
            String::new()
        });
        // Re-apply the same fallback used at registration time so that a
        // `NewIcon` signal can't clobber the resolved well-known name.
        let sni_id = proxy.id().await.unwrap_or_else(|e| {
            warn!(%e, %id, "failed to fetch id");
            String::new()
        });
        let resolved = resolve_icon_name(
            name,
            lookup_well_known_name(&inner.conn, bus_name)
                .await
                .as_deref(),
            sni_id,
        );
        let pixmaps = proxy.icon_pixmap().await.unwrap_or_else(|e| {
            warn!(%e, %id, "failed to fetch icon_pixmap");
            Vec::new()
        });
        (resolved, pixmaps)
    };
    let new_icon = IconData {
        name: icon_name,
        pixmaps: raw
            .into_iter()
            .map(|(w, h, d)| IconPixmap {
                width: w,
                height: h,
                data: d,
            })
            .collect(),
    };

    let event = {
        let mut state = inner.state.write().await;
        let maybe_item = state.items.get_mut(id).map(|item| {
            if attention {
                item.attention_icon = new_icon;
            } else {
                item.icon = new_icon;
            }
            item.clone()
        });
        if let Some(item_clone) = maybe_item {
            state.invalidate_pixmap_cache(id);
            Some(HostEvent::ItemChanged(item_clone))
        } else {
            None
        }
    };
    if let Some(e) = event {
        let _ = inner.events_tx.send(e);
        debug!(%id, attention, "icon updated and pixmap cache cleared");
    }
}

/// Re-fetch the title from D-Bus and update the item cache.
async fn on_title_changed(
    inner: &Arc<TrayHostInner>,
    id: &ItemId,
    proxy: &StatusNotifierItemProxy<'_>,
) {
    let title = proxy.title().await.unwrap_or_else(|e| {
        warn!(%e, %id, "failed to fetch title");
        String::new()
    });
    let event = {
        let mut state = inner.state.write().await;
        if let Some(item) = state.items.get_mut(id) {
            item.title = title;
            Some(HostEvent::ItemChanged(item.clone()))
        } else {
            None
        }
    };
    if let Some(e) = event {
        let _ = inner.events_tx.send(e);
        debug!(%id, "title updated");
    }
}

/// Update the item status and invalidate the pixmap cache (`NeedsAttention`
/// changes which icon `get_pixmap` serves).
async fn on_status_changed(inner: &Arc<TrayHostInner>, id: &ItemId, new_status: TrayStatus) {
    let event = {
        let mut state = inner.state.write().await;
        let maybe_item = state.items.get_mut(id).map(|item| {
            item.status = new_status;
            item.clone()
        });
        if let Some(item_clone) = maybe_item {
            state.invalidate_pixmap_cache(id);
            Some(HostEvent::ItemChanged(item_clone))
        } else {
            None
        }
    };
    if let Some(e) = event {
        let _ = inner.events_tx.send(e);
        debug!(%id, "status updated and pixmap cache cleared");
    }
}

/// Re-fetch the tooltip from D-Bus and update the item cache.
async fn on_tool_tip_changed(
    inner: &Arc<TrayHostInner>,
    id: &ItemId,
    proxy: &StatusNotifierItemProxy<'_>,
) {
    let raw = proxy.tool_tip().await.unwrap_or_else(|e| {
        warn!(%e, %id, "failed to fetch tool_tip");
        Default::default()
    });
    let new_tip = ToolTip {
        icon_name: raw.0,
        icon_pixmaps: raw
            .1
            .into_iter()
            .map(|(w, h, data)| IconPixmap {
                width: w,
                height: h,
                data,
            })
            .collect(),
        title: raw.2,
        description: raw.3,
    };

    let event = {
        let mut state = inner.state.write().await;
        state.items.get_mut(id).map(|item| {
            item.tool_tip = new_tip;
            HostEvent::ItemChanged(item.clone())
        })
    };
    if let Some(e) = event {
        let _ = inner.events_tx.send(e);
        debug!(%id, "tooltip updated");
    }
}

/// Re-fetch the overlay icon from D-Bus and update the item cache.
async fn on_overlay_icon_changed(
    inner: &Arc<TrayHostInner>,
    id: &ItemId,
    proxy: &StatusNotifierItemProxy<'_>,
) {
    let icon_name = proxy.overlay_icon_name().await.unwrap_or_else(|e| {
        warn!(%e, %id, "failed to fetch overlay_icon_name");
        String::new()
    });
    let raw = proxy.overlay_icon_pixmap().await.unwrap_or_else(|e| {
        warn!(%e, %id, "failed to fetch overlay_icon_pixmap");
        Vec::new()
    });
    let new_overlay = IconData {
        name: icon_name,
        pixmaps: raw
            .into_iter()
            .map(|(w, h, d)| IconPixmap {
                width: w,
                height: h,
                data: d,
            })
            .collect(),
    };

    let event = {
        let mut state = inner.state.write().await;
        state.items.get_mut(id).map(|item| {
            item.overlay_icon = new_overlay;
            HostEvent::ItemChanged(item.clone())
        })
    };
    if let Some(e) = event {
        let _ = inner.events_tx.send(e);
        debug!(%id, "overlay icon updated");
    }
}

/// Re-fetch the menu path from D-Bus and update the item cache.
async fn on_menu_path_changed(
    inner: &Arc<TrayHostInner>,
    id: &ItemId,
    proxy: &StatusNotifierItemProxy<'_>,
) {
    let menu_path = proxy
        .menu_path()
        .await
        .map(|p| p.to_string())
        .unwrap_or_else(|e| {
            warn!(%e, %id, "failed to fetch menu_path");
            String::new()
        });

    let event = {
        let mut state = inner.state.write().await;
        state.items.get_mut(id).map(|item| {
            item.menu_path = menu_path;
            HostEvent::ItemChanged(item.clone())
        })
    };
    if let Some(e) = event {
        let _ = inner.events_tx.send(e);
        debug!(%id, "menu path updated");
    }
}

#[cfg(test)]
mod tests;
