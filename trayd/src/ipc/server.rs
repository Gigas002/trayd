use std::path::PathBuf;

use tokio::io::{AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::broadcast::error::RecvError;

use libtrayd::{ItemId, PixmapData, TrayHost, TraydError};

use crate::error::TraydBinError;
use crate::ipc::codec;
use crate::ipc::protocol::{
    Cmd, ErrorCode, IpcResponse, MenuItem, MinimalTrayItem, OkPayload, TrayEvent,
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
        let _ = std::fs::remove_file(&self.socket_path);
        let listener = UnixListener::bind(&self.socket_path)?;
        tracing::info!(path = %self.socket_path.display(), "IPC server listening");
        loop {
            let (stream, _) = listener.accept().await?;
            let host = self.host.clone();
            tokio::spawn(handle_connection(stream, host));
        }
    }
}

async fn handle_connection(stream: UnixStream, host: TrayHost) {
    let (read, mut write) = stream.into_split();
    let mut reader = BufReader::new(read);

    loop {
        let req = match codec::read_request(&mut reader).await {
            Ok(Some(r)) => r,
            Ok(None) => return,
            Err(e) => {
                tracing::warn!(%e, "IPC codec error");
                let resp = IpcResponse::err(ErrorCode::NotImplemented, e.to_string());
                let _ = codec::write_response(&mut write, &resp).await;
                return;
            }
        };

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
                        .map(|n| MenuItem {
                            item_id: n.id as u32,
                            label: n.label,
                            is_submenu: n.is_submenu,
                        })
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
            let resp = match host.get_pixmap(&id, size as u16).await {
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

        Cmd::Subscribe => {
            run_subscribe(write, host).await;
            false
        }
    }
}

/// Coalescing window: after the first event arrives, collect further events
/// for this long before emitting a single snapshot to the subscriber.
const COALESCE_MS: u64 = 50;

async fn run_subscribe<W: AsyncWriteExt + Unpin>(write: &mut W, host: &TrayHost) {
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
    }
}

async fn items_snapshot(host: &TrayHost) -> Vec<MinimalTrayItem> {
    host.items().await.iter().map(to_minimal).collect()
}
