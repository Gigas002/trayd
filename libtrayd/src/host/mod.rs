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

use zbus::{ObjectPath, OwnedVariant, object_server::SignalEmitter};

use crate::{
    TraydError,
    dbus::watcher::{SharedWatcherInner, service_belongs_to},
    dbus::{DBusMenuProxy, StatusNotifierItemProxy, StatusNotifierWatcher, WatcherMsg},
    model::{
        HostEvent, IconData, IconPixmap, ItemId, MenuNode, PixmapData, ScrollOrientation, ToolTip,
        TrayItem, TrayStatus,
    },
};

// ─── Constants ───────────────────────────────────────────────────────────────

const SNI_WATCHER_NAME: &str = "org.kde.StatusNotifierWatcher";
const SNI_WATCHER_PATH: &str = "/StatusNotifierWatcher";
/// Longest wait for any D-Bus reply.  A hung tray app must not stall an
/// `activate` or `get_menu` forever.
const METHOD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
/// Capacity of the HostEvent broadcast channel.
const EVENTS_CAPACITY: usize = 64;

// ─── Internal state ──────────────────────────────────────────────────────────

struct HostState {
    items: HashMap<ItemId, TrayItem>,
    /// Per-item signal-watcher task handles.  Aborted when the item is removed so
    /// that the proxy, its property-cache task, and all signal subscriptions are
    /// cleaned up immediately rather than leaking until the process is restarted.
    item_tasks: HashMap<ItemId, tokio::task::JoinHandle<()>>,
    /// Registrations whose properties are still being fetched, by bus name.
    /// Dropped when that name leaves the bus, so the fetch cannot insert an
    /// item for an app that is already gone.
    pending: HashMap<ItemId, String>,
    /// Registration sequence number per item: lists come out in this order,
    /// so icons keep their place in a bar instead of following hash order.
    order: HashMap<ItemId, u64>,
    next_order: u64,
}

impl HostState {
    fn new() -> Self {
        Self {
            items: HashMap::new(),
            item_tasks: HashMap::new(),
            pending: HashMap::new(),
            order: HashMap::new(),
            next_order: 0,
        }
    }

    /// Track the signal-watcher task of `id`, aborting the one it replaces
    /// (a detached task would keep its proxy and match rules forever).
    fn set_item_task(&mut self, id: ItemId, handle: tokio::task::JoinHandle<()>) {
        if let Some(old) = self.item_tasks.insert(id, handle) {
            old.abort();
        }
    }

    /// Add or replace an item; a new item goes after all existing ones.
    fn insert_item(&mut self, item: TrayItem) {
        if !self.order.contains_key(&item.id) {
            self.order.insert(item.id.clone(), self.next_order);
            self.next_order += 1;
        }
        self.items.insert(item.id.clone(), item);
    }

    /// All items in registration order.
    fn sorted_items(&self) -> Vec<TrayItem> {
        let mut items: Vec<&TrayItem> = self.items.values().collect();
        items.sort_by_key(|item| self.order.get(&item.id).copied().unwrap_or(u64::MAX));
        items.into_iter().cloned().collect()
    }

    /// Remove an item and abort its associated signal-watcher task.
    fn remove_item(&mut self, id: &ItemId) {
        self.items.remove(id);
        self.order.remove(id);
        if let Some(handle) = self.item_tasks.remove(id) {
            handle.abort();
        }
    }
}

// ─── TrayHostInner ────────────────────────────────────────────────────────────

struct TrayHostInner {
    state: RwLock<HostState>,
    events_tx: broadcast::Sender<HostEvent>,
    conn: zbus::Connection,
    /// Shared watcher state, used to prune stale service-id entries when a
    /// bus name disappears so the `RegisteredStatusNotifierItems` property
    /// stays accurate.
    watcher_inner: SharedWatcherInner,
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
    /// Returns `Err(TraydError::DBus(_))` if the session bus is unreachable, and
    /// `Err(TraydError::DBus(zbus::Error::NameTaken))` if another process
    /// already owns the watcher name (another tray is running).
    pub async fn start() -> Result<Self, TraydError> {
        Self::start_with(zbus::connection::Builder::session()).await
    }

    /// Like [`TrayHost::start`], but on the bus at `address` (a D-Bus address
    /// such as `unix:path=/run/user/1000/bus`) instead of the session bus.
    ///
    /// Useful for nested sessions and for tests that run a private bus.
    ///
    /// # Errors
    ///
    /// Same as [`TrayHost::start`], plus `Err(TraydError::DBus(_))` if
    /// `address` is not a valid D-Bus address.
    pub async fn start_on(address: &str) -> Result<Self, TraydError> {
        Self::start_with(zbus::connection::Builder::address(address)).await
    }

    async fn start_with(builder: zbus::connection::Builder<'_>) -> Result<Self, TraydError> {
        // Internal channel: watcher D-Bus object → host background loop.
        let (watcher_tx, watcher_rx) = mpsc::channel::<WatcherMsg>(32);
        let (watcher, watcher_inner) = StatusNotifierWatcher::new(watcher_tx);

        // Register the object and claim the name atomically via Builder so no
        // method calls arrive before the interface is ready.
        //
        // Never replace a running watcher (zbus does by default), and never
        // let one replace us: a second instance would silently take every
        // new registration and, on exit, leave the name with no owner at all.
        let conn = builder
            .serve_at(SNI_WATCHER_PATH, watcher)
            .name(SNI_WATCHER_NAME)
            .replace_existing_names(false)
            .allow_name_replacements(false)
            .method_timeout(METHOD_TIMEOUT)
            .build()
            .await?;
        info!("claimed D-Bus name {SNI_WATCHER_NAME}; watcher at {SNI_WATCHER_PATH}");

        let (events_tx, _) = broadcast::channel(EVENTS_CAPACITY);

        let inner = Arc::new(TrayHostInner {
            state: RwLock::new(HostState::new()),
            events_tx: events_tx.clone(),
            conn: conn.clone(),
            watcher_inner,
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

    /// Number of live [`TrayHost::subscribe`] receivers.
    pub fn subscriber_count(&self) -> usize {
        self.inner.events_tx.receiver_count()
    }

    /// Snapshot of all currently cached items, in registration order.
    pub async fn items(&self) -> Vec<TrayItem> {
        self.inner.state.read().await.sorted_items()
    }

    /// Look up a single item by id.
    pub async fn item(&self, id: &ItemId) -> Option<TrayItem> {
        self.inner.state.read().await.items.get(id).cloned()
    }

    /// Fetch raw ARGB32 pixmap bytes for an item at approximately `size` pixels.
    ///
    /// Picks among the pixmaps held in the item cache: no D-Bus round-trip.
    /// Nothing is stored per requested size, so clients asking for many sizes
    /// cannot grow the daemon's memory.
    ///
    /// When the item's status is [`TrayStatus::NeedsAttention`] and the item
    /// has attention pixmap data, that is served instead of the normal icon.
    ///
    /// # Errors
    ///
    /// - [`TraydError::NotFound`] if the item or its pixmap data is absent.
    pub async fn get_pixmap(&self, id: &ItemId, size: u16) -> Result<PixmapData, TraydError> {
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

        nearest_pixmap(icon, size)
            .ok_or_else(|| TraydError::NotFound(format!("no pixmap for {id}")))
    }

    /// Like [`TrayHost::get_pixmap`], for the overlay icon (a badge drawn
    /// over the normal icon).
    ///
    /// # Errors
    ///
    /// - [`TraydError::NotFound`] if the item or its overlay pixmap data is
    ///   absent.
    pub async fn get_overlay_pixmap(
        &self,
        id: &ItemId,
        size: u16,
    ) -> Result<PixmapData, TraydError> {
        let state = self.inner.state.read().await;
        let item = state
            .items
            .get(id)
            .ok_or_else(|| TraydError::NotFound(id.to_string()))?;
        nearest_pixmap(&item.overlay_icon, size)
            .ok_or_else(|| TraydError::NotFound(format!("no overlay pixmap for {id}")))
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
                .event(item_id as i32, "clicked", zbus::Variant::from(0i32), 0)
                .await
                .map_err(|e| TraydError::ActivationFailed {
                    app_id: id.to_string(),
                    reason: e.to_string(),
                })
        }
    }

    /// Secondary activation (usually a middle click) at screen position
    /// `(x, y)`; apps that don't care get `(0, 0)`.
    ///
    /// # Errors
    ///
    /// - [`TraydError::NotFound`] if the item is not in the cache.
    /// - [`TraydError::ActivationFailed`] when the app returns an error.
    pub async fn secondary_activate(&self, id: &ItemId, x: i32, y: i32) -> Result<(), TraydError> {
        let proxy = self.item_proxy(id).await?;
        proxy
            .secondary_activate(x, y)
            .await
            .map_err(|e| activation_failed(id, e))
    }

    /// Scroll over the item by `delta` (sign is the direction; KDE sends 120
    /// per wheel notch) in `orientation`.
    ///
    /// # Errors
    ///
    /// - [`TraydError::NotFound`] if the item is not in the cache.
    /// - [`TraydError::ActivationFailed`] when the app returns an error.
    pub async fn scroll(
        &self,
        id: &ItemId,
        delta: i32,
        orientation: ScrollOrientation,
    ) -> Result<(), TraydError> {
        let proxy = self.item_proxy(id).await?;
        proxy
            .scroll(delta, orientation.as_str())
            .await
            .map_err(|e| activation_failed(id, e))
    }

    /// Ask the app to show its own context menu at screen position `(x, y)`.
    /// For items without a DBusMenu (see [`TrayItem::menu_path`]); items with
    /// one usually ignore it.
    ///
    /// # Errors
    ///
    /// - [`TraydError::NotFound`] if the item is not in the cache.
    /// - [`TraydError::ActivationFailed`] when the app returns an error.
    pub async fn context_menu(&self, id: &ItemId, x: i32, y: i32) -> Result<(), TraydError> {
        let proxy = self.item_proxy(id).await?;
        proxy
            .context_menu(x, y)
            .await
            .map_err(|e| activation_failed(id, e))
    }

    /// A proxy for the item's `StatusNotifierItem` object.
    async fn item_proxy(
        &self,
        id: &ItemId,
    ) -> Result<StatusNotifierItemProxy<'static>, TraydError> {
        let (bus_name, object_path) = {
            let state = self.inner.state.read().await;
            let item = state
                .items
                .get(id)
                .ok_or_else(|| TraydError::NotFound(id.to_string()))?;
            (item.bus_name.clone(), item.object_path.clone())
        };
        build_proxy(&self.inner.conn, &bus_name, &object_path).await
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

        // Apps that build menus lazily need this first; the ones that don't
        // implement it answer with an error, which changes nothing.
        if let Err(e) = proxy.about_to_show(parent_id).await {
            debug!(%e, %id, parent_id, "AboutToShow failed");
        }

        let (_revision, (_root_id, _root_props, children)) = proxy
            .get_layout(parent_id, 1, &[])
            .await
            .map_err(TraydError::DBus)?;

        Ok(menu_nodes_from_av(children))
    }
}

// ─── Proxy helpers ───────────────────────────────────────────────────────────

/// The pixmap of `icon` closest to `size` wide; on a tie the larger one,
/// which scales down better than a small one scales up.  `None` without
/// pixmaps.
pub(crate) fn nearest_pixmap(icon: &IconData, size: u16) -> Option<PixmapData> {
    let best = icon
        .pixmaps
        .iter()
        .min_by_key(|p| (p.width.abs_diff(i32::from(size)), p.width < i32::from(size)))?;
    // `pixmaps_from_dbus` guarantees positive dimensions.
    Some(PixmapData {
        width: best.width.unsigned_abs(),
        height: best.height.unsigned_abs(),
        data: best.data.clone(),
    })
}

fn activation_failed(id: &ItemId, e: zbus::Error) -> TraydError {
    TraydError::ActivationFailed {
        app_id: id.to_string(),
        reason: e.to_string(),
    }
}

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

async fn build_proxy(
    conn: &zbus::Connection,
    bus_name: &str,
    object_path: &str,
) -> Result<StatusNotifierItemProxy<'static>, TraydError> {
    // No property cache: zbus refreshes cached values only on the standard
    // `PropertiesChanged` signal, which SNI apps don't emit.  They announce
    // changes with `NewTitle`/`NewIcon`/… instead, and a cached proxy would
    // keep serving the first value it ever read.
    StatusNotifierItemProxy::builder(conn)
        .destination(bus_name.to_owned())
        .path(object_path.to_owned())
        .cache_properties(zbus::proxy::CacheProperties::No)
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
        .destination(bus_name.to_owned())
        .path(menu_path.to_owned())
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .await
        .map_err(TraydError::DBus)
}

// ─── Menu layout parsing ──────────────────────────────────────────────────────

/// Convert the `av` children array from a `GetLayout` response into [`MenuNode`]s.
///
/// Each `OwnedVariant` in the array is a D-Bus variant containing
/// `(i32 id, a{sv} properties, av children)`.
pub(crate) fn menu_nodes_from_av(children: Vec<OwnedVariant>) -> Vec<MenuNode> {
    children.into_iter().filter_map(parse_child_ov).collect()
}

fn parse_child_ov(ov: OwnedVariant) -> Option<MenuNode> {
    // Deserialize the child structure (i32, a{sv}, av) via TryFrom.
    let (id, props, nested_children): (i32, HashMap<String, OwnedVariant>, Vec<OwnedVariant>) =
        ov.try_into().ok()?;

    let label = strip_mnemonics(&get_str_prop(&props, "label"));
    let enabled = get_bool_prop(&props, "enabled").unwrap_or(true);
    let visible = get_bool_prop(&props, "visible").unwrap_or(true);
    let icon_name = get_str_prop(&props, "icon-name");
    let icon_data = props
        .get("icon-data")
        .and_then(|v| Vec::<u8>::try_from(v.clone()).ok())
        .unwrap_or_default();
    let shortcuts = props
        .get("shortcut")
        .and_then(|v| Vec::<Vec<String>>::try_from(v.clone()).ok())
        .unwrap_or_default();
    // A submenu is indicated by `children-display` in props, or non-empty children
    // (the latter occurs when GetLayout was called with depth > 1).
    let is_submenu = props.contains_key("children-display") || !nested_children.is_empty();
    let children = menu_nodes_from_av(nested_children);
    let is_separator = get_str_prop(&props, "type") == "separator";
    let toggle_state = match get_str_prop(&props, "toggle-type").as_str() {
        "checkmark" | "radio" => Some(
            props
                .get("toggle-state")
                .and_then(|v| i32::try_from(v.clone()).ok())
                == Some(1),
        ),
        _ => None,
    };

    Some(MenuNode {
        id,
        label,
        enabled,
        visible,
        icon_name,
        icon_data,
        shortcuts,
        is_submenu,
        is_separator,
        toggle_state,
        children,
    })
}

/// Remove DBusMenu access-key markers: `_` before a character marks it as
/// the mnemonic, `__` is a literal underscore.
pub(crate) fn strip_mnemonics(label: &str) -> String {
    let mut out = String::with_capacity(label.len());
    let mut chars = label.chars();
    while let Some(c) = chars.next() {
        if c == '_' {
            match chars.next() {
                Some(next) => out.push(next),
                None => out.push('_'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Convert pixmaps as received over D-Bus, dropping malformed ones.
///
/// Apps control these values: a pixmap is kept only when both sides are
/// positive and the data holds exactly `width × height` ARGB32 pixels, so
/// consumers can trust the dimensions they are given.
pub(crate) fn pixmaps_from_dbus(raw: Vec<(i32, i32, Vec<u8>)>) -> Vec<IconPixmap> {
    raw.into_iter()
        .filter(|(w, h, data)| {
            let expected = usize::try_from(*w)
                .ok()
                .zip(usize::try_from(*h).ok())
                .and_then(|(w, h)| w.checked_mul(h)?.checked_mul(4));
            *w > 0 && *h > 0 && expected == Some(data.len())
        })
        .map(|(width, height, data)| IconPixmap {
            width,
            height,
            data,
        })
        .collect()
}

pub(crate) fn get_str_prop(props: &HashMap<String, OwnedVariant>, key: &str) -> String {
    props
        .get(key)
        .and_then(|v| String::try_from(v.clone()).ok())
        .unwrap_or_default()
}

pub(crate) fn get_bool_prop(props: &HashMap<String, OwnedVariant>, key: &str) -> Option<bool> {
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
                begin_registration(&conn, &inner, msg).await;
            }
            return;
        }
    };

    let name_changes = match fdo.receive_name_owner_changed().await {
        Ok(s) => s,
        Err(e) => {
            error!(%e, "failed to subscribe to NameOwnerChanged; removal tracking disabled");
            while let Some(msg) = watcher_rx.recv().await {
                begin_registration(&conn, &inner, msg).await;
            }
            return;
        }
    };
    tokio::pin!(name_changes);

    loop {
        tokio::select! {
            msg = watcher_rx.recv() => {
                match msg {
                    Some(watcher_msg) => begin_registration(&conn, &inner, watcher_msg).await,
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

/// Record the registration as pending and fetch the app's properties in a
/// task of its own: a slow or hung app delays only its own item.
async fn begin_registration(conn: &zbus::Connection, inner: &Arc<TrayHostInner>, msg: WatcherMsg) {
    let WatcherMsg::ItemRegistered {
        service_id,
        bus_name,
        ..
    } = &msg;
    inner
        .state
        .write()
        .await
        .pending
        .insert(ItemId(service_id.clone()), bus_name.clone());
    tokio::spawn(handle_item_registered(conn.clone(), Arc::clone(inner), msg));
}

async fn handle_item_registered(
    conn: zbus::Connection,
    inner: Arc<TrayHostInner>,
    msg: WatcherMsg,
) {
    let conn = &conn;
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
            inner
                .state
                .write()
                .await
                .pending
                .remove(&ItemId(service_id));
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

    // Spawn a per-item signal watcher to keep the cache fresh, then store
    // the handle so it can be aborted when the item is removed.
    let mut state = inner.state.write().await;
    if state.pending.remove(&id).is_none() {
        // The app left the bus while we were reading its properties.
        debug!(%service_id, "app left during registration; not adding it");
        return;
    }
    let handle = tokio::spawn(run_item_signal_watcher(
        conn.clone(),
        Arc::clone(&inner),
        id.clone(),
        bus_name.to_owned(),
        object_path.to_owned(),
        well_known,
    ));
    state.set_item_task(id.clone(), handle);
    state.insert_item(item.clone());
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
    let pending_before = state.pending.len();
    state.pending.retain(|_, bus_name| *bus_name != gone);
    let cancelled_pending = state.pending.len() != pending_before;
    let ids_to_remove: Vec<ItemId> = state
        .items
        .values()
        .filter(|item| item.bus_name == gone)
        .map(|item| item.id.clone())
        .collect();

    if ids_to_remove.is_empty() && !cancelled_pending {
        return;
    }

    // Also prune the watcher's registered-items list so the D-Bus property
    // reflects reality and so that the service can re-register cleanly if it
    // reconnects under a new unique bus name.  Collect what was actually
    // removed (not just `ids_to_remove`, which only reflects the host's own
    // item cache) so we can emit `StatusNotifierItemUnregistered` for each
    // per the SNI watcher spec.
    let removed_services: Vec<String> = {
        let mut watcher = inner.watcher_inner.lock().await;
        let drained = std::mem::take(&mut watcher.items);
        let (removed, kept): (Vec<String>, Vec<String>) = drained
            .into_iter()
            .partition(|svc| service_belongs_to(svc, &gone));
        watcher.items = kept;
        removed
    };

    for id in ids_to_remove {
        state.remove_item(&id);
        let _ = inner.events_tx.send(HostEvent::ItemRemoved(id.clone()));
        debug!(%id, %gone, "item removed from cache");
    }

    if removed_services.is_empty() {
        return;
    }

    let path = match ObjectPath::from_static_str(SNI_WATCHER_PATH) {
        Ok(p) => p,
        Err(e) => {
            warn!(%e, "invalid watcher object path; skipping StatusNotifierItemUnregistered");
            return;
        }
    };
    let emitter = match SignalEmitter::new(&inner.conn, path) {
        Ok(e) => e,
        Err(e) => {
            warn!(
                %e,
                "failed to create signal emitter; skipping StatusNotifierItemUnregistered"
            );
            return;
        }
    };
    for service in &removed_services {
        if let Err(e) =
            StatusNotifierWatcher::status_notifier_item_unregistered(&emitter, service).await
        {
            warn!(%e, %service, "failed to emit StatusNotifierItemUnregistered");
        }
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

/// Where registration reads an app's properties from: one `GetAll` round
/// trip instead of thirteen `Get`s.
enum Props {
    /// The `GetAll` reply.
    All(HashMap<String, OwnedVariant>),
    /// The app answered `GetAll` with an error: read properties one by one.
    Single,
    /// The app did not answer at all (timeout, gone): don't wait on it once
    /// per property.
    Unreachable(zbus::Error),
}

impl Props {
    async fn load(proxy: &StatusNotifierItemProxy<'_>) -> Self {
        let inner = proxy.inner();
        let reply = inner
            .connection()
            .call_method(
                Some(inner.destination()),
                inner.path(),
                Some("org.freedesktop.DBus.Properties"),
                "GetAll",
                &(inner.interface(),),
            )
            .await;
        match reply.and_then(|m| m.body().deserialize::<HashMap<String, OwnedVariant>>()) {
            Ok(all) => Self::All(all),
            Err(e @ zbus::Error::MethodError(..)) => {
                debug!(%e, "GetAll refused; reading properties one by one");
                Self::Single
            }
            Err(e) => Self::Unreachable(e),
        }
    }

    /// Property `name`, from the `GetAll` reply or through `single`.
    async fn read<T>(
        &self,
        name: &str,
        single: impl std::future::Future<Output = zbus::Result<T>>,
    ) -> zbus::Result<T>
    where
        T: TryFrom<OwnedVariant>,
    {
        match self {
            Self::All(all) => {
                let value = all
                    .get(name)
                    .ok_or_else(|| zbus::Error::Failure(format!("no property {name}")))?;
                T::try_from(value.try_clone()?).map_err(|_| zbus::Error::IncorrectType)
            }
            Self::Single => single.await,
            Self::Unreachable(e) => Err(e.clone()),
        }
    }
}

async fn fetch_item_properties(
    proxy: &StatusNotifierItemProxy<'_>,
    service_id: &str,
    bus_name: &str,
    object_path: &str,
    well_known_name: Option<&str>,
) -> TrayItem {
    let props = Props::load(proxy).await;
    let title = props
        .read("Title", proxy.title())
        .await
        .unwrap_or_else(|e| {
            warn!(%e, %service_id, "failed to fetch title");
            String::new()
        });
    let status = TrayStatus::from_dbus(&props.read("Status", proxy.status()).await.unwrap_or_else(
        |e| {
            warn!(%e, %service_id, "failed to fetch status");
            String::new()
        },
    ));
    let icon_name = props
        .read("IconName", proxy.icon_name())
        .await
        .unwrap_or_else(|e| {
            warn!(%e, %service_id, "failed to fetch icon_name");
            String::new()
        });
    // Priority: explicit `IconName` → well-known bus name (e.g. `org.telegram.desktop`)
    // → SNI `Id` as a last resort (e.g. `TelegramDesktop`).
    let sni_id = props.read("Id", proxy.id()).await.unwrap_or_else(|e| {
        warn!(%e, %service_id, "failed to fetch id");
        String::new()
    });
    let icon_name = resolve_icon_name(icon_name, well_known_name, sni_id);
    let raw_pixmaps = props
        .read("IconPixmap", proxy.icon_pixmap())
        .await
        .unwrap_or_else(|e| {
            warn!(%e, %service_id, "failed to fetch icon_pixmap");
            Vec::new()
        });
    let category = props
        .read("Category", proxy.category())
        .await
        .unwrap_or_else(|e| {
            warn!(%e, %service_id, "failed to fetch category");
            String::new()
        });
    // Defer resolving item_is_menu until after menu_path is known — if the
    // property is absent (e.g. Ayatana/AppIndicator items like nm-applet) we
    // fall back to "true" whenever a valid menu path exists, which prevents a
    // crash from calling Activate on an item that only exposes a menu.
    let item_is_menu_result = props.read("ItemIsMenu", proxy.item_is_menu()).await;
    let raw_tool_tip = props
        .read("ToolTip", proxy.tool_tip())
        .await
        .unwrap_or_else(|e| {
            warn!(%e, %service_id, "failed to fetch tool_tip");
            Default::default()
        });
    let overlay_icon_name = props
        .read("OverlayIconName", proxy.overlay_icon_name())
        .await
        .unwrap_or_else(|e| {
            warn!(%e, %service_id, "failed to fetch overlay_icon_name");
            String::new()
        });
    let raw_overlay_pixmaps = props
        .read("OverlayIconPixmap", proxy.overlay_icon_pixmap())
        .await
        .unwrap_or_else(|e| {
            warn!(%e, %service_id, "failed to fetch overlay_icon_pixmap");
            Vec::new()
        });
    let attention_icon_name = props
        .read("AttentionIconName", proxy.attention_icon_name())
        .await
        .unwrap_or_else(|e| {
            warn!(%e, %service_id, "failed to fetch attention_icon_name");
            String::new()
        });
    let raw_attention_pixmaps = props
        .read("AttentionIconPixmap", proxy.attention_icon_pixmap())
        .await
        .unwrap_or_else(|e| {
            warn!(%e, %service_id, "failed to fetch attention_icon_pixmap");
            Vec::new()
        });
    // Optional in the specification and absent for most apps: no warning.
    let icon_theme_path = props
        .read("IconThemePath", proxy.icon_theme_path())
        .await
        .unwrap_or_default();
    let menu_path = props
        .read("Menu", proxy.menu_path())
        .await
        .map(|p| p.to_string())
        .unwrap_or_else(|e| {
            warn!(%e, %service_id, "failed to fetch menu_path");
            String::new()
        });

    let item_is_menu = item_is_menu_result.unwrap_or_else(|e| {
        // If the property is unavailable, infer from the menu path: an item
        // that exposes a menu but doesn't declare ItemIsMenu is treated as
        // menu-only so we never call Activate on it by mistake.
        let has_menu = !menu_path.is_empty() && menu_path != "/";
        warn!(%e, %service_id, has_menu, "failed to fetch item_is_menu, inferring from menu_path");
        has_menu
    });

    let tool_tip = ToolTip {
        icon_name: raw_tool_tip.0,
        icon_pixmaps: pixmaps_from_dbus(raw_tool_tip.1),
        title: raw_tool_tip.2,
        description: raw_tool_tip.3,
    };

    let overlay_icon = IconData {
        name: overlay_icon_name,
        pixmaps: pixmaps_from_dbus(raw_overlay_pixmaps),
    };

    let pixmaps = pixmaps_from_dbus(raw_pixmaps);

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
            pixmaps: pixmaps_from_dbus(raw_attention_pixmaps),
        },
        icon_theme_path,
        menu_path,
    }
}

// ─── Per-item signal watcher ──────────────────────────────────────────────────────────────────────────────────

/// What a burst of change signals asks the refresher to re-read.
#[derive(Debug, Default)]
struct Dirty {
    icon: bool,
    attention: bool,
    title: bool,
    overlay: bool,
    menu: bool,
    tool_tip: bool,
}

/// Aborts the wrapped task when dropped.
struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Long-running per-item task: subscribes to SNI property-change signals and
/// keeps the in-process item cache fresh.
///
/// The signal loop itself never waits on D-Bus: it only records what changed
/// and wakes a refresher task.  zbus applies backpressure to signal streams,
/// so a loop that stopped reading while it waited for a property reply would
/// block the connection's reader, and with it that very reply — a burst of
/// more than 64 signals froze the whole connection.  Recording also coalesces
/// bursts into one re-read.
///
/// The task exits naturally when all signal streams close (i.e. the app's bus
/// name disappears); the refresher is aborted with it.
async fn run_item_signal_watcher(
    conn: zbus::Connection,
    inner: Arc<TrayHostInner>,
    id: ItemId,
    bus_name: String,
    object_path: String,
    well_known: Option<String>,
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
    let new_theme_path = match proxy.receive_new_icon_theme_path().await {
        Ok(s) => s,
        Err(e) => {
            warn!(%e, %id, "signal watcher: new_icon_theme_path subscribe failed");
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
        new_theme_path,
        new_tool_tip
    );
    debug!(%id, "item signal watcher started");

    let dirty = Arc::new(std::sync::Mutex::new(Dirty::default()));
    let wake = Arc::new(tokio::sync::Notify::new());
    let _refresher = AbortOnDrop(tokio::spawn(run_item_refresher(
        Arc::clone(&inner),
        id.clone(),
        proxy.clone(),
        well_known,
        Arc::clone(&dirty),
        Arc::clone(&wake),
    )));
    let mark = |set: fn(&mut Dirty)| {
        set(&mut dirty.lock().unwrap_or_else(|e| e.into_inner()));
        wake.notify_one();
    };

    loop {
        tokio::select! {
            Some(_) = new_icon.next() => mark(|d| d.icon = true),
            Some(_) = new_title.next() => mark(|d| d.title = true),
            Some(sig) = new_status.next() => {
                // The new status travels in the signal: no D-Bus read needed.
                if let Ok(args) = sig.args() {
                    on_status_changed(&inner, &id, TrayStatus::from_dbus(args.status())).await;
                }
            }
            Some(_) = new_attention.next() => mark(|d| d.attention = true),
            Some(_) = new_overlay.next() => mark(|d| d.overlay = true),
            Some(_) = new_menu.next() => mark(|d| d.menu = true),
            Some(_) = new_tool_tip.next() => mark(|d| d.tool_tip = true),
            Some(sig) = new_theme_path.next() => {
                // The new path travels in the signal: no D-Bus read needed.
                if let Ok(args) = sig.args() {
                    on_icon_theme_path_changed(&inner, &id, args.icon_theme_path().clone()).await;
                }
            }
            else => break,
        }
    }
    debug!(%id, "item signal watcher ended");
}

/// Re-reads whatever the signal loop marked dirty, one round at a time.
async fn run_item_refresher(
    inner: Arc<TrayHostInner>,
    id: ItemId,
    proxy: StatusNotifierItemProxy<'static>,
    well_known: Option<String>,
    dirty: Arc<std::sync::Mutex<Dirty>>,
    wake: Arc<tokio::sync::Notify>,
) {
    loop {
        wake.notified().await;
        let d = std::mem::take(&mut *dirty.lock().unwrap_or_else(|e| e.into_inner()));
        if d.icon {
            on_icon_changed(&inner, &id, &proxy, false, well_known.as_deref()).await;
        }
        if d.attention {
            on_icon_changed(&inner, &id, &proxy, true, well_known.as_deref()).await;
        }
        if d.title {
            on_title_changed(&inner, &id, &proxy).await;
        }
        if d.overlay {
            on_overlay_icon_changed(&inner, &id, &proxy).await;
        }
        if d.menu {
            on_menu_path_changed(&inner, &id, &proxy).await;
        }
        if d.tool_tip {
            on_tool_tip_changed(&inner, &id, &proxy).await;
        }
    }
}

/// Re-fetch the normal or attention icon from D-Bus and update the item cache.
async fn on_icon_changed(
    inner: &Arc<TrayHostInner>,
    id: &ItemId,
    proxy: &StatusNotifierItemProxy<'_>,
    attention: bool,
    well_known: Option<&str>,
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
        // The well-known name was looked up once at registration; repeating
        // the lookup (one call per name on the bus) for every animation
        // frame of an icon cost ~24 round trips per NewIcon.
        let resolved = resolve_icon_name(name, well_known, sni_id);
        let pixmaps = proxy.icon_pixmap().await.unwrap_or_else(|e| {
            warn!(%e, %id, "failed to fetch icon_pixmap");
            Vec::new()
        });
        (resolved, pixmaps)
    };
    // `NewIcon` also announces a new `IconThemePath` (KDE has no separate
    // signal for it).
    let theme_path = if attention {
        None
    } else {
        Some(proxy.icon_theme_path().await.unwrap_or_default())
    };
    let new_icon = IconData {
        name: icon_name,
        pixmaps: pixmaps_from_dbus(raw),
    };

    let event = {
        let mut state = inner.state.write().await;
        let maybe_item = state.items.get_mut(id).map(|item| {
            if attention {
                item.attention_icon = new_icon;
            } else {
                item.icon = new_icon;
            }
            if let Some(path) = theme_path {
                item.icon_theme_path = path;
            }
            item.clone()
        });
        maybe_item.map(HostEvent::ItemChanged)
    };
    if let Some(e) = event {
        let _ = inner.events_tx.send(e);
        debug!(%id, attention, "icon updated");
    }
}

/// Store a new `IconThemePath` sent with `NewIconThemePath`.
async fn on_icon_theme_path_changed(inner: &Arc<TrayHostInner>, id: &ItemId, path: String) {
    let event = {
        let mut state = inner.state.write().await;
        state.items.get_mut(id).map(|item| {
            item.icon_theme_path = path;
            HostEvent::ItemChanged(item.clone())
        })
    };
    if let Some(e) = event {
        let _ = inner.events_tx.send(e);
        debug!(%id, "icon theme path updated");
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

/// Update the item status (`NeedsAttention` changes which icon `get_pixmap`
/// serves).
async fn on_status_changed(inner: &Arc<TrayHostInner>, id: &ItemId, new_status: TrayStatus) {
    let event = {
        let mut state = inner.state.write().await;
        let maybe_item = state.items.get_mut(id).map(|item| {
            item.status = new_status;
            item.clone()
        });
        maybe_item.map(HostEvent::ItemChanged)
    };
    if let Some(e) = event {
        let _ = inner.events_tx.send(e);
        debug!(%id, "status updated");
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
        icon_pixmaps: pixmaps_from_dbus(raw.1),
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
        pixmaps: pixmaps_from_dbus(raw),
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
