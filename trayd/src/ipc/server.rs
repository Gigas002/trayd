use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::path::{Path, PathBuf};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::broadcast::error::RecvError;

use libtrayd::{ItemId, PixmapData, TrayHost, TraydError};

use crate::error::TraydBinError;
use crate::ipc::codec;
use crate::ipc::protocol::{
    Cmd, ErrorCode, IpcResponse, MenuItem, MinimalTrayItem, OkPayload, TrayEvent, V,
};

pub struct IpcServer {
    pub socket_path: PathBuf,
    pub host: TrayHost,
}

impl IpcServer {
    pub fn new(socket_path: impl Into<PathBuf>, host: TrayHost) -> Self {
        Self {
            socket_path: socket_path.into(),
            host,
        }
    }

    pub async fn run(&self) -> Result<(), TraydBinError> {
        prepare_socket_path(&self.socket_path)?;
        let listener = UnixListener::bind(&self.socket_path)?;
        // Only this user may talk to the daemon, wherever the socket lives.
        std::fs::set_permissions(&self.socket_path, std::fs::Permissions::from_mode(0o600))?;
        tracing::info!(path = %self.socket_path.display(), "IPC server listening");
        loop {
            let (stream, _) = listener.accept().await?;
            let host = self.host.clone();
            tokio::spawn(handle_connection(stream, host));
        }
    }
}

/// Make `path` free for binding: remove a stale socket left by a daemon that
/// did not shut down cleanly, but never anything else.
///
/// # Errors
///
/// [`TraydBinError::SocketPathTaken`] when a non-socket file is in the way.
pub fn prepare_socket_path(path: &Path) -> Result<(), TraydBinError> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_socket() => Ok(std::fs::remove_file(path)?),
        Ok(_) => Err(TraydBinError::SocketPathTaken(path.display().to_string())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

async fn handle_connection(stream: UnixStream, host: TrayHost) {
    let (read, mut write) = stream.into_split();
    let mut reader = BufReader::new(read);

    loop {
        let req = match codec::read_request(&mut reader).await {
            Ok(Some(r)) => r,
            Ok(None) => return,
            Err(e @ TraydBinError::Json(_)) => {
                // The line was framed correctly; the client can carry on.
                tracing::debug!(%e, "invalid IPC request");
                let resp = IpcResponse::err(ErrorCode::InvalidRequest, e.to_string());
                if codec::write_response(&mut write, &resp).await.is_err() {
                    return;
                }
                continue;
            }
            Err(e @ TraydBinError::RequestTooLong) => {
                // The rest of the line is still unread: close the connection.
                tracing::debug!(%e, "invalid IPC request");
                let resp = IpcResponse::err(ErrorCode::InvalidRequest, e.to_string());
                let _ = codec::write_response(&mut write, &resp).await;
                return;
            }
            Err(e) => {
                tracing::debug!(%e, "IPC read failed");
                return;
            }
        };

        if req.v != V {
            let resp = IpcResponse::err(
                ErrorCode::InvalidRequest,
                format!(
                    "unsupported protocol version {}; this daemon speaks {V}",
                    req.v
                ),
            );
            if codec::write_response(&mut write, &resp).await.is_err() {
                return;
            }
            continue;
        }

        if req.cmd == Cmd::Subscribe {
            run_subscribe(&mut write, &mut reader, &host).await;
            return;
        }
        if !dispatch(&mut write, &host, req.cmd).await {
            return;
        }
    }
}

/// Returns `false` when the connection should be closed.
async fn dispatch<W: AsyncWriteExt + Unpin>(write: &mut W, host: &TrayHost, cmd: Cmd) -> bool {
    match cmd {
        Cmd::Ping => {
            let resp = IpcResponse::ok(OkPayload::Pong);
            codec::write_response(write, &resp).await.is_ok()
        }

        Cmd::GetItems => {
            let items = items_snapshot(host).await;
            let resp = IpcResponse::ok(OkPayload::Items { items });
            codec::write_response(write, &resp).await.is_ok()
        }

        Cmd::GetMenu { app_id, submenu_id } => {
            let id = ItemId::from(app_id.clone());
            let resp = match host.get_menu(&id, submenu_id).await {
                Ok(nodes) => {
                    let items = nodes
                        .into_iter()
                        .filter(|n| n.visible)
                        .map(to_menu_item)
                        .collect();
                    IpcResponse::ok(OkPayload::Menu { app_id, items })
                }
                Err(TraydError::NotFound(_)) => {
                    IpcResponse::err(ErrorCode::NotFound, format!("{app_id} not found"))
                }
                Err(e) => IpcResponse::err(ErrorCode::BusFailed, e.to_string()),
            };
            codec::write_response(write, &resp).await.is_ok()
        }

        Cmd::Activate { app_id, item_id } => {
            let id = ItemId::from(app_id.clone());
            let resp = match host.activate(&id, item_id).await {
                Ok(()) => IpcResponse::ok(OkPayload::Ack),
                Err(TraydError::NotFound(_)) => {
                    IpcResponse::err(ErrorCode::NotFound, format!("{app_id} not found"))
                }
                Err(e) => IpcResponse::err(ErrorCode::BusFailed, e.to_string()),
            };
            codec::write_response(write, &resp).await.is_ok()
        }

        Cmd::GetPixmap { app_id, size } => {
            let id = ItemId::from(app_id.clone());
            // Sizes beyond u16 can't match anything better than u16::MAX does.
            let wanted = u16::try_from(size).unwrap_or(u16::MAX);
            let resp = match host.get_pixmap(&id, wanted).await {
                Ok(PixmapData {
                    width,
                    height,
                    data: bytes,
                }) => {
                    let enc_len = base64_ng::STANDARD.encoded_len(bytes.len()).unwrap_or(0);
                    let mut buf = vec![0u8; enc_len];
                    let n = base64_ng::STANDARD
                        .encode_slice(&bytes, &mut buf)
                        .unwrap_or(0);
                    // SAFETY: base64 output is always ASCII
                    let data = String::from_utf8(buf[..n].to_vec()).unwrap_or_default();
                    IpcResponse::ok(OkPayload::Pixmap {
                        app_id,
                        size,
                        width,
                        height,
                        data,
                    })
                }
                Err(TraydError::NotFound(_)) => {
                    IpcResponse::err(ErrorCode::NotFound, format!("{app_id} not found"))
                }
                Err(e) => IpcResponse::err(ErrorCode::BusFailed, e.to_string()),
            };
            codec::write_response(write, &resp).await.is_ok()
        }

        Cmd::SecondaryActivate { app_id, x, y } => {
            let id = ItemId::from(app_id.clone());
            let resp = ack_or_error(&app_id, host.secondary_activate(&id, x, y).await);
            codec::write_response(write, &resp).await.is_ok()
        }

        Cmd::Scroll {
            app_id,
            delta,
            orientation,
        } => {
            let id = ItemId::from(app_id.clone());
            let resp = ack_or_error(&app_id, host.scroll(&id, delta, orientation.into()).await);
            codec::write_response(write, &resp).await.is_ok()
        }

        Cmd::ContextMenu { app_id, x, y } => {
            let id = ItemId::from(app_id.clone());
            let resp = ack_or_error(&app_id, host.context_menu(&id, x, y).await);
            codec::write_response(write, &resp).await.is_ok()
        }

        // Handled by `handle_connection`, which owns the read side.
        Cmd::Subscribe => false,
    }
}

/// `ack`, or the error response for a failed item method call.
fn ack_or_error(app_id: &str, result: Result<(), TraydError>) -> IpcResponse {
    match result {
        Ok(()) => IpcResponse::ok(OkPayload::Ack),
        Err(TraydError::NotFound(_)) => {
            IpcResponse::err(ErrorCode::NotFound, format!("{app_id} not found"))
        }
        Err(e) => IpcResponse::err(ErrorCode::BusFailed, e.to_string()),
    }
}

/// Coalescing window: after the first event arrives, collect further events
/// for this long before emitting a single snapshot to the subscriber.
const COALESCE_MS: u64 = 50;

/// Resolves when the client closes its end (or the read fails).  Anything the
/// client sends while subscribed is read and ignored.
async fn closed<R: AsyncRead + Unpin>(read: &mut R) {
    let mut buf = [0u8; 256];
    while let Ok(1..) = read.read(&mut buf).await {}
}

/// Stream tray snapshots to `write` until the client goes away.
///
/// The read side is watched too: a client that disconnects is dropped at
/// once, with its broadcast receiver, instead of at the next tray event,
/// when writing to it would fail.
async fn run_subscribe<W, R>(write: &mut W, read: &mut R, host: &TrayHost)
where
    W: AsyncWriteExt + Unpin,
    R: AsyncRead + Unpin,
{
    tokio::select! {
        () = stream_events(write, host) => {}
        () = closed(read) => tracing::debug!("subscriber disconnected"),
    }
}

async fn stream_events<W: AsyncWriteExt + Unpin>(write: &mut W, host: &TrayHost) {
    let mut events_rx = host.subscribe();

    // Send the initial full snapshot immediately.
    let initial = IpcResponse::ok(OkPayload::Event {
        event: TrayEvent::Update(items_snapshot(host).await),
    });
    if codec::write_response(write, &initial).await.is_err() {
        return;
    }

    loop {
        // Wait for the first event.
        match events_rx.recv().await {
            Ok(_) | Err(RecvError::Lagged(_)) => {}
            Err(RecvError::Closed) => return,
        }

        // Coalesce: drain any events that arrive within the window.
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(COALESCE_MS);
        loop {
            tokio::select! {
                _ = tokio::time::sleep_until(deadline) => break,
                next = events_rx.recv() => match next {
                    Ok(_) | Err(RecvError::Lagged(_)) => continue,
                    Err(RecvError::Closed) => {
                        // Send one final update before exiting.
                        let resp = IpcResponse::ok(OkPayload::Event {
                            event: TrayEvent::Update(items_snapshot(host).await),
                        });
                        let _ = codec::write_response(write, &resp).await;
                        return;
                    }
                }
            }
        }

        // Emit one coalesced snapshot.
        let resp = IpcResponse::ok(OkPayload::Event {
            event: TrayEvent::Update(items_snapshot(host).await),
        });
        if codec::write_response(write, &resp).await.is_err() {
            return;
        }
    }
}

pub(crate) fn to_menu_item(node: libtrayd::MenuNode) -> MenuItem {
    MenuItem {
        // DBusMenu ids are i32; the cast round-trips through `activate`.
        item_id: node.id as u32,
        label: node.label,
        is_submenu: node.is_submenu,
        enabled: node.enabled,
        separator: node.is_separator,
        checked: node.toggle_state,
    }
}

fn to_minimal(item: &libtrayd::TrayItem) -> MinimalTrayItem {
    // For items needing attention, prefer the attention icon name when available.
    let icon_handle = if item.status == libtrayd::TrayStatus::NeedsAttention {
        item.attention_icon
            .as_handle()
            .or_else(|| item.icon.as_handle())
    } else {
        item.icon.as_handle()
    };
    MinimalTrayItem {
        app_id: item.id.to_string(),
        title: if item.title.is_empty() {
            None
        } else {
            Some(item.title.clone())
        },
        status: item.status.to_string(),
        icon_handle,
        category: if item.category.is_empty() {
            None
        } else {
            Some(item.category.clone())
        },
        item_is_menu: item.item_is_menu,
        tooltip_title: if item.tool_tip.title.is_empty() {
            None
        } else {
            Some(item.tool_tip.title.clone())
        },
        tooltip_description: if item.tool_tip.description.is_empty() {
            None
        } else {
            Some(item.tool_tip.description.clone())
        },
        overlay_icon_handle: item.overlay_icon.as_handle(),
        icon_theme_path: if item.icon_theme_path.is_empty() {
            None
        } else {
            Some(item.icon_theme_path.clone())
        },
    }
}

async fn items_snapshot(host: &TrayHost) -> Vec<MinimalTrayItem> {
    host.items().await.iter().map(to_minimal).collect()
}
