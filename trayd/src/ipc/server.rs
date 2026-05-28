use std::path::PathBuf;

use tokio::io::BufReader;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::RecvError;

use crate::error::TraydBinError;
use crate::ipc::codec;
use crate::ipc::protocol::{
    Cmd, ErrorCode, IpcResponse, MenuItem, MinimalTrayItem, OkPayload, TrayEvent,
};

pub struct IpcServer {
    pub socket_path: PathBuf,
    pub events_tx: broadcast::Sender<TrayEvent>,
}

impl IpcServer {
    pub fn new(socket_path: impl Into<PathBuf>) -> Self {
        let (events_tx, _) = broadcast::channel(32);
        Self {
            socket_path: socket_path.into(),
            events_tx,
        }
    }

    pub async fn run(&self) -> Result<(), TraydBinError> {
        let _ = std::fs::remove_file(&self.socket_path);
        let listener = UnixListener::bind(&self.socket_path)?;
        tracing::info!(path = %self.socket_path.display(), "IPC server listening");
        loop {
            let (stream, _) = listener.accept().await?;
            let events_rx = self.events_tx.subscribe();
            tokio::spawn(handle_connection(stream, events_rx));
        }
    }
}

async fn handle_connection(stream: UnixStream, mut events_rx: broadcast::Receiver<TrayEvent>) {
    let (read, mut write) = stream.into_split();
    let mut reader = BufReader::new(read);

    loop {
        let req = match codec::read_request(&mut reader).await {
            Ok(Some(r)) => r,
            Ok(None) => break,
            Err(e) => {
                tracing::warn!(%e, "IPC codec error");
                let resp = IpcResponse::err(ErrorCode::NotImplemented, e.to_string());
                let _ = codec::write_response(&mut write, &resp).await;
                break;
            }
        };

        match req.cmd {
            Cmd::Ping => {
                let resp = IpcResponse::ok(OkPayload::Pong);
                if codec::write_response(&mut write, &resp).await.is_err() {
                    break;
                }
            }
            Cmd::GetItems => {
                let resp = IpcResponse::ok(OkPayload::Items {
                    items: mock_items(),
                });
                if codec::write_response(&mut write, &resp).await.is_err() {
                    break;
                }
            }
            Cmd::GetMenu { app_id, submenu_id } => {
                let items = mock_menu(submenu_id);
                let resp = IpcResponse::ok(OkPayload::Menu { app_id, items });
                if codec::write_response(&mut write, &resp).await.is_err() {
                    break;
                }
            }
            Cmd::Activate { .. } => {
                let resp = IpcResponse::ok(OkPayload::Ack);
                if codec::write_response(&mut write, &resp).await.is_err() {
                    break;
                }
            }
            Cmd::GetPixmap { app_id, size } => {
                let resp = IpcResponse::ok(OkPayload::Pixmap {
                    app_id,
                    size,
                    data: String::new(),
                });
                if codec::write_response(&mut write, &resp).await.is_err() {
                    break;
                }
            }
            Cmd::Subscribe => {
                let initial = IpcResponse::ok(OkPayload::Event {
                    event: TrayEvent::Update(mock_items()),
                });
                if codec::write_response(&mut write, &initial).await.is_err() {
                    break;
                }
                loop {
                    match events_rx.recv().await {
                        Ok(event) => {
                            let resp = IpcResponse::ok(OkPayload::Event { event });
                            if codec::write_response(&mut write, &resp).await.is_err() {
                                break;
                            }
                        }
                        Err(RecvError::Closed) => break,
                        Err(RecvError::Lagged(_)) => continue,
                    }
                }
                break;
            }
        }
    }
}

fn mock_items() -> Vec<MinimalTrayItem> {
    vec![MinimalTrayItem {
        app_id: "org.example.App".into(),
        title: Some("Example App".into()),
        status: "active".into(),
        icon_handle: Some("example-app".into()),
    }]
}

fn mock_menu(submenu_id: Option<u32>) -> Vec<MenuItem> {
    if submenu_id.is_some() {
        vec![MenuItem {
            item_id: 10,
            label: "Sub Item 1".into(),
            is_submenu: false,
        }]
    } else {
        vec![
            MenuItem {
                item_id: 1,
                label: "Action".into(),
                is_submenu: false,
            },
            MenuItem {
                item_id: 2,
                label: "Submenu".into(),
                is_submenu: true,
            },
        ]
    }
}
