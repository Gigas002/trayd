//! Unix-socket IPC server: accepts connections and dispatches requests.
//!
//! The [`Handler`] trait is the seam between the IPC layer and the tray
//! host. [`StubHandler`] provides a mock implementation for Phase 1 tests
//! and the skeleton daemon; Phase 2 wires a real [`libtrayd::TrayHost`].

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

/// Synchronous request dispatcher plugged into the IPC server.
///
/// All methods are called from an async context but are themselves sync;
/// Phase 2 will introduce async D-Bus calls — at that point the trait can
/// be updated to return futures or the impl can use `tokio::task::spawn_blocking`.
pub trait Handler: Send + Sync + 'static {
    /// Dispatch a single non-streaming [`Method`] and return the [`Response`].
    fn handle(&self, method: &Method) -> Response;

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
    #[allow(dead_code)] // available for diagnostics and tests
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Accept connections and dispatch requests until `shutdown` becomes `true`.
    ///
    /// Each accepted connection is handled in its own Tokio task.
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
        let Some(envelope) = frame else { break }; // EOF — client disconnected

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
                // 1) Send the subscribe ack.
                writer
                    .write_frame(&Response::ok(ResultPayload::Subscribed))
                    .await?;
                // 2) Stream events until the client disconnects or the channel closes.
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
                                // Client disconnected.
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
                let resp = handler.handle(other);
                writer.write_frame(&resp).await?;
            }
        }
    }
    Ok(())
}

// ── StubHandler (Phase 1 mock) ────────────────────────────────────────────────

/// In-memory stub [`Handler`] for Phase 1 tests and the skeleton daemon.
///
/// All D-Bus-backed methods (`get_pixmap`, `menu_open`, …) return a
/// `BUS_FAILED` error until Phase 2/3 wires a real [`libtrayd::TrayHost`].
pub struct StubHandler {
    items: Vec<ItemInfo>,
    event_tx: tokio::sync::broadcast::Sender<HostEvent>,
}

impl StubHandler {
    /// Create an empty stub (no items registered).
    pub fn new() -> Self {
        let (tx, _) = tokio::sync::broadcast::channel(64);
        Self {
            items: vec![],
            event_tx: tx,
        }
    }

    /// Pre-populate the stub with the given items.
    #[allow(dead_code)] // used in tests and future daemon wiring
    pub fn with_items(mut self, items: Vec<ItemInfo>) -> Self {
        self.items = items;
        self
    }

    /// Push a synthetic event to all active `subscribe` connections.
    #[allow(dead_code)] // used in tests
    pub fn emit(&self, event: HostEvent) {
        // Ignore the error: no subscribers is fine.
        let _ = self.event_tx.send(event);
    }

    /// Convenience: build a minimal active item for test fixtures.
    #[allow(dead_code)] // used in tests
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
    fn handle(&self, method: &Method) -> Response {
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

            // `Subscribe` is handled at the connection level, not here.
            Method::Subscribe => Response::err(ErrorPayload::internal(
                "subscribe must be handled in the connection loop",
            )),
        }
    }

    fn subscribe(&self) -> tokio::sync::broadcast::Receiver<HostEvent> {
        self.event_tx.subscribe()
    }
}
