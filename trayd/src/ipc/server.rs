//! Unix-socket IPC server: accepts connections and dispatches requests.
//!
//! The [`Handler`] trait is the seam between the IPC layer and the tray
//! host.  [`StubHandler`] provides an in-memory mock (used in tests and
//! as a fallback); [`TrayHostHandler`] wires a real [`libtrayd::TrayHost`]
//! and is used by the production daemon.

use std::path::Path;
use std::sync::Arc;

use tokio::net::{UnixListener, UnixStream};
use tracing::{debug, warn};

use super::protocol::{
    ErrorPayload, EventEnvelope, HostEvent, ItemInfo, ItemStatus, Method, PROTOCOL_VERSION,
    RequestEnvelope, Response, ResultPayload,
};
use super::{
    IpcError,
    codec::{FramedReader, FramedWriter},
};

// ── Handler trait ─────────────────────────────────────────────────────────────

/// Async request dispatcher plugged into the IPC server.
pub trait Handler: Send + Sync + 'static {
    /// Dispatch a single non-streaming request and return the response.
    fn handle(&self, method: &Method) -> impl std::future::Future<Output = Response> + Send;

    /// Return a broadcast receiver that yields [`HostEvent`]s for `subscribe`.
    fn subscribe(&self) -> tokio::sync::broadcast::Receiver<HostEvent>;
}

// ── Server ────────────────────────────────────────────────────────────────────

/// Bound Unix-socket IPC server.
pub struct Server {
    listener: UnixListener,
    socket_path: std::path::PathBuf,
}

impl Server {
    /// Bind to `path`, removing a stale socket file if present.
    pub fn bind(path: &Path) -> Result<Self, IpcError> {
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        let listener = UnixListener::bind(path)?;
        Ok(Self {
            listener,
            socket_path: path.to_owned(),
        })
    }

    /// Path the server is listening on.
    #[allow(dead_code)]
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Accept connections and dispatch requests until `shutdown` becomes `true`.
    pub async fn run<H: Handler>(
        &self,
        handler: Arc<H>,
        mut shutdown: tokio::sync::watch::Receiver<bool>,
    ) -> Result<(), IpcError> {
        debug!(socket = %self.socket_path.display(), "IPC server listening");
        loop {
            tokio::select! {
                biased;
                _ = shutdown.changed() => {
                    if *shutdown.borrow() {
                        debug!("IPC server received shutdown signal");
                        break;
                    }
                }
                accept = self.listener.accept() => {
                    let (stream, _peer) = accept?;
                    let h = Arc::clone(&handler);
                    tokio::spawn(async move {
                        if let Err(e) = handle_connection(stream, h).await {
                            warn!(error = %e, "IPC connection error");
                        }
                    });
                }
            }
        }
        Ok(())
    }
}

// ── Per-connection handler ────────────────────────────────────────────────────

async fn handle_connection<H: Handler>(
    stream: UnixStream,
    handler: Arc<H>,
) -> Result<(), IpcError> {
    let (read_half, write_half) = stream.into_split();
    let mut reader = FramedReader::new(read_half);
    let mut writer = FramedWriter::new(write_half);

    loop {
        let frame = reader.read_frame::<RequestEnvelope>().await?;
        let Some(envelope) = frame else { break };

        if envelope.v != PROTOCOL_VERSION {
            let resp = Response::err(ErrorPayload::invalid_request(format!(
                "unsupported protocol version {}; server speaks v{PROTOCOL_VERSION}",
                envelope.v
            )));
            writer.write_frame(&resp).await?;
            continue;
        }

        match &envelope.method {
            Method::Subscribe => {
                writer
                    .write_frame(&Response::ok(ResultPayload::Subscribed))
                    .await?;
                let mut rx = handler.subscribe();
                loop {
                    use tokio::sync::broadcast::error::RecvError;
                    match rx.recv().await {
                        Ok(event) => {
                            if writer
                                .write_frame(&EventEnvelope::new(event))
                                .await
                                .is_err()
                            {
                                return Ok(());
                            }
                        }
                        Err(RecvError::Lagged(missed)) => {
                            warn!(missed, "subscribe receiver lagged; some events dropped");
                        }
                        Err(RecvError::Closed) => return Ok(()),
                    }
                }
            }
            other => {
                let resp = handler.handle(other).await;
                writer.write_frame(&resp).await?;
            }
        }
    }
    Ok(())
}

// ── StubHandler (tests / fallback) ────────────────────────────────────────────

/// In-memory stub [`Handler`] for tests and the fallback daemon (no D-Bus).
pub struct StubHandler {
    items: Vec<ItemInfo>,
    event_tx: tokio::sync::broadcast::Sender<HostEvent>,
}

impl StubHandler {
    pub fn new() -> Self {
        let (tx, _) = tokio::sync::broadcast::channel(64);
        Self {
            items: vec![],
            event_tx: tx,
        }
    }

    #[allow(dead_code)]
    pub fn with_items(mut self, items: Vec<ItemInfo>) -> Self {
        self.items = items;
        self
    }

    #[allow(dead_code)]
    pub fn emit(&self, event: HostEvent) {
        let _ = self.event_tx.send(event);
    }

    #[allow(dead_code)]
    pub fn mock_item(id: impl Into<String>) -> ItemInfo {
        ItemInfo {
            id: id.into(),
            title: None,
            status: ItemStatus::Active,
            has_attention_icon: false,
            tooltip: None,
            category: None,
        }
    }
}

impl Default for StubHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl Handler for StubHandler {
    async fn handle(&self, method: &Method) -> Response {
        match method {
            Method::Ping => Response::ok(ResultPayload::Pong {
                version: env!("CARGO_PKG_VERSION").to_owned(),
            }),

            Method::List => Response::ok(ResultPayload::List {
                items: self.items.clone(),
            }),

            Method::Activate { item_id } | Method::SecondaryActivate { item_id } => {
                if self.items.iter().any(|i| &i.id == item_id) {
                    Response::ok(ResultPayload::Ok)
                } else {
                    Response::err(ErrorPayload::not_found(format!(
                        "item not found: {item_id}"
                    )))
                }
            }

            Method::Scroll { item_id, .. } => {
                if self.items.iter().any(|i| &i.id == item_id) {
                    Response::ok(ResultPayload::Ok)
                } else {
                    Response::err(ErrorPayload::not_found(format!(
                        "item not found: {item_id}"
                    )))
                }
            }

            Method::GetPixmap { item_id, .. } => Response::err(ErrorPayload::bus_failed(format!(
                "pixmap not yet available (Phase 2): {item_id}"
            ))),

            Method::MenuOpen { item_id } => Response::err(ErrorPayload::bus_failed(format!(
                "menu not yet available (Phase 3): {item_id}"
            ))),

            Method::MenuSelect { session_id, .. } | Method::MenuClose { session_id } => {
                Response::err(ErrorPayload::invalid_session(format!(
                    "no active session: {session_id}"
                )))
            }

            Method::Subscribe => Response::err(ErrorPayload::internal(
                "subscribe must be handled in the connection loop",
            )),
        }
    }

    fn subscribe(&self) -> tokio::sync::broadcast::Receiver<HostEvent> {
        self.event_tx.subscribe()
    }
}

// ── TrayHostHandler (Phase 2 production handler) ──────────────────────────────

/// [`Handler`] backed by a real [`libtrayd::TrayHost`].
///
/// Translates IPC requests into D-Bus calls and converts `libtrayd` events
/// into IPC wire events on the broadcast channel.
pub struct TrayHostHandler {
    host: Arc<libtrayd::TrayHost>,
    event_tx: tokio::sync::broadcast::Sender<HostEvent>,
}

impl TrayHostHandler {
    /// Create the handler.  Spawns a task that forwards `libtrayd`
    /// [`libtrayd::HostEvent`]s to the IPC [`HostEvent`] broadcast channel.
    pub fn new(host: libtrayd::TrayHost) -> Self {
        let host = Arc::new(host);
        let (event_tx, _) = tokio::sync::broadcast::channel(256);

        // Forward libtrayd domain events → IPC wire events.
        let mut lib_rx = host.subscribe();
        let ipc_tx = event_tx.clone();
        tokio::spawn(async move {
            loop {
                use tokio::sync::broadcast::error::RecvError;
                match lib_rx.recv().await {
                    Ok(ev) => {
                        let ipc_ev = map_host_event(ev);
                        let _ = ipc_tx.send(ipc_ev);
                    }
                    Err(RecvError::Lagged(n)) => {
                        tracing::warn!(missed = n, "libtrayd event receiver lagged");
                    }
                    Err(RecvError::Closed) => break,
                }
            }
        });

        Self { host, event_tx }
    }
}

impl Handler for TrayHostHandler {
    async fn handle(&self, method: &Method) -> Response {
        match method {
            Method::Ping => Response::ok(ResultPayload::Pong {
                version: env!("CARGO_PKG_VERSION").to_owned(),
            }),

            Method::List => {
                let items = self.host.list().await;
                Response::ok(ResultPayload::List {
                    items: items.into_iter().map(item_to_info).collect(),
                })
            }

            Method::Activate { item_id } => match self.host.activate(item_id).await {
                Ok(()) => Response::ok(ResultPayload::Ok),
                Err(libtrayd::TraydError::NotFound(id)) => {
                    Response::err(ErrorPayload::not_found(format!("item not found: {id}")))
                }
                Err(e) => Response::err(ErrorPayload::bus_failed(e.to_string())),
            },

            Method::SecondaryActivate { item_id } => {
                match self.host.secondary_activate(item_id).await {
                    Ok(()) => Response::ok(ResultPayload::Ok),
                    Err(libtrayd::TraydError::NotFound(id)) => {
                        Response::err(ErrorPayload::not_found(format!("item not found: {id}")))
                    }
                    Err(e) => Response::err(ErrorPayload::bus_failed(e.to_string())),
                }
            }

            Method::Scroll {
                item_id,
                direction,
                delta,
            } => {
                let lib_dir = map_scroll_dir(*direction);
                match self.host.scroll(item_id, lib_dir, *delta).await {
                    Ok(()) => Response::ok(ResultPayload::Ok),
                    Err(libtrayd::TraydError::NotFound(id)) => {
                        Response::err(ErrorPayload::not_found(format!("item not found: {id}")))
                    }
                    Err(e) => Response::err(ErrorPayload::bus_failed(e.to_string())),
                }
            }

            Method::GetPixmap { item_id, size } => {
                match self.host.get_pixmap(item_id, *size).await {
                    Ok(pix) => {
                        use base64::Engine;
                        let data = base64::engine::general_purpose::STANDARD.encode(&pix.data);
                        Response::ok(ResultPayload::Pixmap(super::protocol::PixmapPayload {
                            item_id: item_id.clone(),
                            format: super::protocol::PixmapFormat::Argb32,
                            width: pix.width,
                            height: pix.height,
                            data,
                        }))
                    }
                    Err(libtrayd::TraydError::NotFound(id)) => {
                        Response::err(ErrorPayload::not_found(format!("item not found: {id}")))
                    }
                    Err(libtrayd::TraydError::NoPixmap(id)) => {
                        Response::err(ErrorPayload::not_found(format!("no pixmap for item: {id}")))
                    }
                    Err(e) => Response::err(ErrorPayload::bus_failed(e.to_string())),
                }
            }

            Method::MenuOpen { item_id } => Response::err(ErrorPayload::bus_failed(format!(
                "menu not yet available (Phase 3): {item_id}"
            ))),

            Method::MenuSelect { session_id, .. } | Method::MenuClose { session_id } => {
                Response::err(ErrorPayload::invalid_session(format!(
                    "no active session: {session_id}"
                )))
            }

            Method::Subscribe => Response::err(ErrorPayload::internal(
                "subscribe must be handled in the connection loop",
            )),
        }
    }

    fn subscribe(&self) -> tokio::sync::broadcast::Receiver<HostEvent> {
        self.event_tx.subscribe()
    }
}

// ── Conversion helpers ────────────────────────────────────────────────────────

fn item_to_info(item: libtrayd::Item) -> ItemInfo {
    ItemInfo {
        id: item.id.0,
        title: item.title,
        status: match item.status {
            libtrayd::ItemStatus::Passive => ItemStatus::Passive,
            libtrayd::ItemStatus::Active => ItemStatus::Active,
            libtrayd::ItemStatus::NeedsAttention => ItemStatus::NeedsAttention,
        },
        has_attention_icon: item.has_attention_icon,
        tooltip: item.tooltip,
        category: item.category,
    }
}

fn map_host_event(ev: libtrayd::HostEvent) -> HostEvent {
    match ev {
        libtrayd::HostEvent::ItemAdded(item) => HostEvent::ItemAdded {
            item: item_to_info(item),
        },
        libtrayd::HostEvent::ItemRemoved(id) => HostEvent::ItemRemoved { id: id.0 },
        libtrayd::HostEvent::ItemUpdated(item) => HostEvent::ItemUpdated {
            item: item_to_info(item),
        },
    }
}

fn map_scroll_dir(dir: super::protocol::ScrollDirection) -> libtrayd::ScrollDirection {
    match dir {
        super::protocol::ScrollDirection::Up => libtrayd::ScrollDirection::Up,
        super::protocol::ScrollDirection::Down => libtrayd::ScrollDirection::Down,
        super::protocol::ScrollDirection::Left => libtrayd::ScrollDirection::Left,
        super::protocol::ScrollDirection::Right => libtrayd::ScrollDirection::Right,
    }
}
