//! Socket client + wire types per `docs/IPC.md` (not shared with the trayd crate).

use std::path::Path;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};

use crate::error::TuiError;

// ---------------------------------------------------------------------------
// Wire types — own copies, not imported from the trayd crate
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct Request {
    v: u8,
    #[serde(flatten)]
    cmd: Cmd,
}

#[derive(Debug, Serialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
enum Cmd {
    Subscribe,
    GetItems,
    GetMenu {
        app_id: String,
        submenu_id: Option<u32>,
    },
    Activate {
        app_id: String,
        item_id: u32,
    },
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum Response {
    Err(ErrResponse),
    Ok(OkResponse),
}

#[derive(Debug, Deserialize)]
pub struct OkResponse {
    #[allow(dead_code)]
    pub v: u8,
    #[serde(flatten)]
    pub payload: Payload,
}

#[derive(Debug, Deserialize)]
pub struct ErrResponse {
    #[allow(dead_code)]
    pub v: u8,
    pub error: IpcError,
}

#[derive(Debug, Deserialize)]
pub struct IpcError {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Payload {
    Menu {
        #[allow(dead_code)]
        app_id: String,
        items: Vec<MenuItem>,
    },
    Ack,
    Items {
        items: Vec<MinimalTrayItem>,
    },
    Pong,
    Event {
        event: TrayEvent,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MenuItem {
    pub item_id: u32,
    pub label: String,
    pub is_submenu: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MinimalTrayItem {
    pub app_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_handle: Option<String>,
    /// Item category per SNI spec.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// `true` when this item is a pure menu (no application window — should not activate).
    #[serde(default)]
    pub item_is_menu: bool,
    /// Tooltip title text, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tooltip_title: Option<String>,
    /// Tooltip description text, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tooltip_description: Option<String>,
    /// Overlay icon theme name, if set (displayed on top of the normal icon).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overlay_icon_handle: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", content = "items", rename_all = "snake_case")]
pub enum TrayEvent {
    Update(Vec<MinimalTrayItem>),
}

// ---------------------------------------------------------------------------
// IPC client
// ---------------------------------------------------------------------------

pub struct IpcClient {
    reader: BufReader<OwnedReadHalf>,
    writer: OwnedWriteHalf,
}

impl IpcClient {
    pub async fn connect(socket_path: &Path) -> Result<Self, TuiError> {
        let stream = UnixStream::connect(socket_path)
            .await
            .map_err(|e| TuiError::DaemonUnreachable(e.to_string()))?;
        let (read_half, write_half) = stream.into_split();
        Ok(Self {
            reader: BufReader::new(read_half),
            writer: write_half,
        })
    }

    pub async fn get_items(&mut self) -> Result<Vec<MinimalTrayItem>, TuiError> {
        let req = Request {
            v: 1,
            cmd: Cmd::GetItems,
        };
        self.send(&req).await?;
        match self.recv().await? {
            Response::Ok(OkResponse {
                payload: Payload::Items { items },
                ..
            }) => Ok(items),
            Response::Ok(ok) => Err(TuiError::Ipc(format!("unexpected response: {ok:?}"))),
            Response::Err(e) => Err(TuiError::Ipc(format!(
                "{}: {}",
                e.error.code, e.error.message
            ))),
        }
    }

    pub async fn get_menu(
        &mut self,
        app_id: &str,
        submenu_id: Option<u32>,
    ) -> Result<Vec<MenuItem>, TuiError> {
        let req = Request {
            v: 1,
            cmd: Cmd::GetMenu {
                app_id: app_id.to_owned(),
                submenu_id,
            },
        };
        self.send(&req).await?;
        match self.recv().await? {
            Response::Ok(OkResponse {
                payload: Payload::Menu { items, .. },
                ..
            }) => Ok(items),
            Response::Ok(ok) => Err(TuiError::Ipc(format!("unexpected response: {ok:?}"))),
            Response::Err(e) => Err(TuiError::Ipc(format!(
                "{}: {}",
                e.error.code, e.error.message
            ))),
        }
    }

    pub async fn activate(&mut self, app_id: &str, item_id: u32) -> Result<(), TuiError> {
        let req = Request {
            v: 1,
            cmd: Cmd::Activate {
                app_id: app_id.to_owned(),
                item_id,
            },
        };
        self.send(&req).await?;
        match self.recv().await? {
            Response::Ok(OkResponse {
                payload: Payload::Ack,
                ..
            }) => Ok(()),
            Response::Ok(ok) => Err(TuiError::Ipc(format!("unexpected response: {ok:?}"))),
            Response::Err(e) => Err(TuiError::Ipc(format!(
                "{}: {}",
                e.error.code, e.error.message
            ))),
        }
    }

    /// Send a `subscribe` request. Call [`Self::recv_event`] in a loop after this.
    pub async fn send_subscribe(&mut self) -> Result<(), TuiError> {
        let req = Request {
            v: 1,
            cmd: Cmd::Subscribe,
        };
        self.send(&req).await
    }

    /// Read one [`TrayEvent`] from the subscribe stream.
    pub async fn recv_event(&mut self) -> Result<TrayEvent, TuiError> {
        match self.recv().await? {
            Response::Ok(OkResponse {
                payload: Payload::Event { event },
                ..
            }) => Ok(event),
            Response::Ok(ok) => Err(TuiError::Ipc(format!("unexpected response: {ok:?}"))),
            Response::Err(e) => Err(TuiError::Ipc(format!(
                "{}: {}",
                e.error.code, e.error.message
            ))),
        }
    }

    async fn send<T: Serialize>(&mut self, req: &T) -> Result<(), TuiError> {
        let mut line = serde_json::to_string(req)?;
        line.push('\n');
        self.writer.write_all(line.as_bytes()).await?;
        Ok(())
    }

    async fn recv(&mut self) -> Result<Response, TuiError> {
        let mut line = String::new();
        self.reader.read_line(&mut line).await?;
        if line.is_empty() {
            return Err(TuiError::DaemonUnreachable("connection closed".to_owned()));
        }
        Ok(serde_json::from_str(&line)?)
    }
}

#[cfg(test)]
mod tests;
