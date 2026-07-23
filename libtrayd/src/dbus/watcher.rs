//! `org.kde.StatusNotifierWatcher` D-Bus service implementation.
//!
//! The watcher is registered at `/StatusNotifierWatcher` and claims the
//! `org.kde.StatusNotifierWatcher` well-known bus name.  Apps call
//! `RegisterStatusNotifierItem` on it; the watcher forwards registrations to
//! [`TrayHost`](crate::host::TrayHost) via an internal mpsc channel.

use std::sync::Arc;

use tokio::sync::{Mutex, mpsc};
use tracing::debug;
use zbus::{interface, object_server::SignalEmitter};

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

// ─── Internal watcher state ──────────────────────────────────────────────────

pub struct WatcherInner {
    pub items: Vec<String>,
}

/// Shared, mutable watcher state.  Also held by the tray host so it can remove
/// stale entries when bus names disappear.
pub type SharedWatcherInner = Arc<Mutex<WatcherInner>>;

// ─── StatusNotifierWatcher ───────────────────────────────────────────────────────

/// D-Bus implementation of `org.kde.StatusNotifierWatcher`.
///
/// Registered at `/StatusNotifierWatcher` via [`zbus::ObjectServer`].
pub struct StatusNotifierWatcher {
    inner: SharedWatcherInner,
    msg_tx: mpsc::Sender<WatcherMsg>,
}

impl StatusNotifierWatcher {
    /// Create a new watcher and return it together with the shared inner state
    /// so that the host loop can remove stale entries when services disappear.
    pub fn new(msg_tx: mpsc::Sender<WatcherMsg>) -> (Self, SharedWatcherInner) {
        let inner = Arc::new(Mutex::new(WatcherInner { items: Vec::new() }));
        let watcher = Self {
            inner: Arc::clone(&inner),
            msg_tx,
        };
        (watcher, inner)
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
            let mut inner = self.inner.lock().await;
            if !inner.items.contains(&service_id) {
                inner.items.push(service_id.clone());
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

    /// Called by SNI hosts to announce themselves.  We are always the host, so
    /// this is a no-op other than emitting the confirmation signal.
    async fn register_status_notifier_host(
        &self,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
        _service: &str,
    ) -> zbus::fdo::Result<()> {
        Self::status_notifier_host_registered(&emitter)
            .await
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        Ok(())
    }

    /// List of currently registered items.
    #[zbus(property)]
    async fn registered_status_notifier_items(&self) -> Vec<String> {
        self.inner.lock().await.items.clone()
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

    /// Emitted when a previously registered `StatusNotifierItem` disappears
    /// from the bus.  Called from [`crate::host`] after it prunes the stale
    /// entry from the shared [`WatcherInner::items`] list.
    #[zbus(signal)]
    pub(crate) async fn status_notifier_item_unregistered(
        emitter: &SignalEmitter<'_>,
        service: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn status_notifier_host_registered(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
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
