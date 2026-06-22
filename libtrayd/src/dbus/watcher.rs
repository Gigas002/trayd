//! `org.kde.StatusNotifierWatcher` D-Bus service implementation.
//!
//! The watcher is registered at `/StatusNotifierWatcher` and claims the
//! `org.kde.StatusNotifierWatcher` well-known bus name.  Apps call
//! `RegisterStatusNotifierItem` on it; the watcher forwards registrations to
//! [`TrayHost`](crate::host::TrayHost) via an internal mpsc channel.
//!
//! # Ownership
//!
//! The registered-items and registered-hosts lists are **private** to the
//! watcher — the host never holds a reference to them.  When a bus name
//! disappears the host sends a
//! [`WatcherCmd`] through a dedicated channel; the internal
//! [`run_watcher_cmd_loop`] task removes the stale entry and emits the
//! appropriate D-Bus signal.

use std::sync::Arc;
use tokio::sync::{Mutex, mpsc};
use tracing::{debug, warn};
use zbus::{interface, object_server::SignalEmitter, zvariant::ObjectPath};

// ─── WatcherMsg ──────────────────────────────────────────────────────────────

/// Message sent from the watcher D-Bus object to the [`TrayHost`] background loop.
///
/// [`TrayHost`]: crate::host::TrayHost
#[derive(Debug)]
pub enum WatcherMsg {
    /// An app called `RegisterStatusNotifierItem`.
    ItemRegistered {
        /// Normalised service id stored in `RegisteredStatusNotifierItems`.
        service_id: String,
        /// Extracted D-Bus bus name.
        bus_name: String,
        /// Extracted D-Bus object path.
        object_path: String,
    },
}

// ─── WatcherCmd ──────────────────────────────────────────────────────────────

/// Command sent from the host background loop **to** the watcher when a D-Bus
/// bus name disappears.
///
/// The corresponding [`run_watcher_cmd_loop`] task processes these commands:
/// it removes the stale entry from the private items list and emits the
/// appropriate D-Bus signal.
#[derive(Debug)]
pub enum WatcherCmd {
    /// A bus name disappeared; remove this service from `RegisteredStatusNotifierItems`
    /// and emit `StatusNotifierItemUnregistered`.
    UnregisterItem(String),
    /// A bus name disappeared; remove it from `RegisteredStatusNotifierHosts`
    /// and emit `StatusNotifierHostUnregistered`.
    UnregisterHost(String),
}

// ─── StatusNotifierWatcher ───────────────────────────────────────────────────

/// D-Bus implementation of `org.kde.StatusNotifierWatcher`.
///
/// Registered at `/StatusNotifierWatcher` via [`zbus::ObjectServer`].
///
/// The registered-items list is **not** shared with the host — the host
/// communicates item removal through [`WatcherCmd`] messages instead.
pub struct StatusNotifierWatcher {
    /// Private item list; never shared with `TrayHostInner`.
    items: Arc<Mutex<Vec<String>>>,
    /// Private host list; never shared with `TrayHostInner`.
    hosts: Arc<Mutex<Vec<String>>>,
    msg_tx: mpsc::Sender<WatcherMsg>,
}

impl StatusNotifierWatcher {
    pub fn new(msg_tx: mpsc::Sender<WatcherMsg>) -> Self {
        Self {
            items: Arc::new(Mutex::new(Vec::new())),
            hosts: Arc::new(Mutex::new(Vec::new())),
            msg_tx,
        }
    }

    /// Clone the items `Arc` for the internal command-processing task.
    ///
    /// This is the **only** way the items list escapes the watcher, and it
    /// stays within this module (never reaches `TrayHostInner`).
    pub(crate) fn items_cloned(&self) -> Arc<Mutex<Vec<String>>> {
        Arc::clone(&self.items)
    }

    /// Clone the hosts `Arc` for the internal command-processing task.
    pub(crate) fn hosts_cloned(&self) -> Arc<Mutex<Vec<String>>> {
        Arc::clone(&self.hosts)
    }
}

#[interface(name = "org.kde.StatusNotifierWatcher")]
impl StatusNotifierWatcher {
    /// Called by SNI apps to register with the watcher.
    async fn register_status_notifier_item(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
        service: &str,
    ) -> zbus::fdo::Result<()> {
        let sender = hdr.sender().map(|s| s.to_string()).unwrap_or_default();

        let (bus_name, object_path) = parse_service(&sender, service);

        // Canonical id: sender+path if only a path was given, else the raw service string.
        let service_id = if service.starts_with('/') {
            format!("{}{}", sender, service)
        } else {
            service.to_owned()
        };

        let should_notify = {
            let mut items = self.items.lock().await;
            if !items.contains(&service_id) {
                items.push(service_id.clone());
                true
            } else {
                false
            }
        };

        if should_notify {
            debug!(service_id, "SNI item registered");
            let _ = self
                .msg_tx
                .send(WatcherMsg::ItemRegistered {
                    service_id: service_id.clone(),
                    bus_name,
                    object_path,
                })
                .await;
            Self::status_notifier_item_registered(&emitter, &service_id)
                .await
                .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        }

        Ok(())
    }

    /// Called by SNI hosts to announce themselves.
    async fn register_status_notifier_host(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
        service: &str,
    ) -> zbus::fdo::Result<()> {
        let host_id = if service.is_empty() {
            hdr.sender().map(|s| s.to_string()).unwrap_or_default()
        } else {
            service.to_owned()
        };

        if !host_id.is_empty() {
            let mut hosts = self.hosts.lock().await;
            if !hosts.contains(&host_id) {
                hosts.push(host_id.clone());
            }
        }

        debug!(%host_id, "SNI host registered");
        Self::status_notifier_host_registered(&emitter)
            .await
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        Ok(())
    }

    /// List of currently registered items.
    #[zbus(property)]
    async fn registered_status_notifier_items(&self) -> Vec<String> {
        self.items.lock().await.clone()
    }

    /// List of currently registered host service names.
    #[zbus(property)]
    async fn registered_status_notifier_hosts(&self) -> Vec<String> {
        self.hosts.lock().await.clone()
    }

    /// Always `true` — we are the host.
    #[zbus(property)]
    fn is_status_notifier_host_registered(&self) -> bool {
        true
    }

    /// SNI protocol version (always 0).
    #[zbus(property)]
    fn protocol_version(&self) -> i32 {
        0
    }

    // ── D-Bus signals ────────────────────────────────────────────────────────

    #[zbus(signal)]
    async fn status_notifier_item_registered(
        emitter: &SignalEmitter<'_>,
        service: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    pub async fn status_notifier_item_unregistered(
        emitter: &SignalEmitter<'_>,
        service: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn status_notifier_host_registered(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(signal)]
    pub async fn status_notifier_host_unregistered(
        emitter: &SignalEmitter<'_>,
        service: &str,
    ) -> zbus::Result<()>;
}

// ─── Watcher command loop ────────────────────────────────────────────────────

const SNI_WATCHER_PATH: &str = "/StatusNotifierWatcher";

/// Background task that processes [`WatcherCmd`] messages from the host.
///
/// Owns clones of the private items and hosts lists so the host never needs
/// direct access to them.
pub(crate) async fn run_watcher_cmd_loop(
    conn: zbus::Connection,
    items: Arc<Mutex<Vec<String>>>,
    hosts: Arc<Mutex<Vec<String>>>,
    mut cmd_rx: mpsc::Receiver<WatcherCmd>,
) {
    while let Some(cmd) = cmd_rx.recv().await {
        match cmd {
            WatcherCmd::UnregisterItem(service_id) => {
                // Remove from the registered list so clients querying the
                // `RegisteredStatusNotifierItems` property don't see stale entries.
                items.lock().await.retain(|s| s != &service_id);

                // Emit the D-Bus signal so watcher clients (e.g. tray-trigger)
                // receive timely removal notifications.
                let path = match ObjectPath::from_static_str(SNI_WATCHER_PATH) {
                    Ok(p) => p,
                    Err(e) => {
                        warn!(%e, "invalid watcher object path");
                        continue;
                    }
                };
                let emitter = match SignalEmitter::new(&conn, path) {
                    Ok(e) => e,
                    Err(e) => {
                        warn!(
                            %e,
                            "failed to create signal emitter, skipping StatusNotifierItemUnregistered"
                        );
                        continue;
                    }
                };
                if let Err(e) =
                    StatusNotifierWatcher::status_notifier_item_unregistered(&emitter, &service_id)
                        .await
                {
                    warn!(%e, %service_id, "failed to emit StatusNotifierItemUnregistered");
                }
            }
            WatcherCmd::UnregisterHost(bus_name) => {
                // Check whether the gone bus name is a registered host; if so,
                // remove it and emit the signal.
                let was_host = {
                    let mut hosts = hosts.lock().await;
                    if hosts.contains(&bus_name) {
                        hosts.retain(|s| s != &bus_name);
                        true
                    } else {
                        false
                    }
                };

                if was_host {
                    let path = match ObjectPath::from_static_str(SNI_WATCHER_PATH) {
                        Ok(p) => p,
                        Err(e) => {
                            warn!(%e, "invalid watcher object path");
                            continue;
                        }
                    };
                    let emitter = match SignalEmitter::new(&conn, path) {
                        Ok(e) => e,
                        Err(e) => {
                            warn!(
                                %e,
                                "failed to create signal emitter, skipping StatusNotifierHostUnregistered"
                            );
                            continue;
                        }
                    };
                    if let Err(e) = StatusNotifierWatcher::status_notifier_host_unregistered(
                        &emitter, &bus_name,
                    )
                    .await
                    {
                        warn!(%e, %bus_name, "failed to emit StatusNotifierHostUnregistered");
                    }
                }
            }
        }
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Split a raw SNI service registration string into `(bus_name, object_path)`.
///
/// The SNI spec allows three forms:
///
/// | Input                            | bus_name            | object_path              |
/// |----------------------------------|---------------------|--------------------------|
/// | `"com.example.App"`              | `"com.example.App"` | `"/StatusNotifierItem"`  |
/// | `"/StatusNotifierItem"`          | `<sender>`          | `"/StatusNotifierItem"`  |
/// | `"com.example.App/SomePath"`     | `"com.example.App"` | `"/SomePath"`            |
pub fn parse_service(sender: &str, service: &str) -> (String, String) {
    if service.starts_with('/') {
        // Only a path was given; the bus name is the message sender.
        (sender.to_owned(), service.to_owned())
    } else if let Some(slash) = service.find('/') {
        // Combined "busname/objectpath" form.
        (service[..slash].to_owned(), service[slash..].to_owned())
    } else {
        // Plain bus name; use the standard default path.
        (service.to_owned(), "/StatusNotifierItem".to_owned())
    }
}
