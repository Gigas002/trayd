use serde::{Deserialize, Serialize};

pub const V: u8 = 1;

// Skip serializing `item_is_menu` when it is `false` (the common case).
fn is_false(b: &bool) -> bool {
    !b
}

/// Consumer → daemon request.
#[derive(Debug, Serialize, Deserialize)]
pub struct IpcRequest {
    pub v: u8,
    #[serde(flatten)]
    pub cmd: Cmd,
}

impl IpcRequest {
    pub fn new(cmd: Cmd) -> Self {
        Self { v: V, cmd }
    }
}

/// IPC commands (§3.2, §3.6).
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Cmd {
    Ping,
    Subscribe,
    GetItems,
    GetMenu {
        app_id: String,
        #[serde(default)]
        submenu_id: Option<u32>,
    },
    Activate {
        app_id: String,
        item_id: u32,
    },
    GetPixmap {
        app_id: String,
        size: u32,
    },
}

/// Daemon → consumer response. Tries `Err` variant first on deserialization.
#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum IpcResponse {
    Err(ErrResponse),
    Ok(OkResponse),
}

impl IpcResponse {
    pub fn ok(payload: OkPayload) -> Self {
        Self::Ok(OkResponse { v: V, payload })
    }

    pub fn err(code: ErrorCode, message: impl Into<String>) -> Self {
        Self::Err(ErrResponse {
            v: V,
            error: IpcError {
                code,
                message: message.into(),
            },
        })
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OkResponse {
    pub v: u8,
    #[serde(flatten)]
    pub payload: OkPayload,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ErrResponse {
    pub v: u8,
    pub error: IpcError,
}

/// Successful response payloads, tagged by `"type"`.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OkPayload {
    Pong,
    Items {
        items: Vec<MinimalTrayItem>,
    },
    Event {
        event: TrayEvent,
    },
    Menu {
        app_id: String,
        items: Vec<MenuItem>,
    },
    Ack,
    Pixmap {
        app_id: String,
        /// Requested size (from the `get_pixmap` command).
        size: u32,
        /// Actual pixel width of the returned pixmap surface.
        width: u32,
        /// Actual pixel height of the returned pixmap surface.
        height: u32,
        /// Base64-encoded ARGB32 bytes (`width × height × 4`).
        data: String,
    },
}

#[derive(Debug, Serialize, Deserialize)]
pub struct IpcError {
    pub code: ErrorCode,
    pub message: String,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    NotFound,
    BusFailed,
    InvalidAppId,
    NotImplemented,
}

/// Minimal per-item snapshot sent to bar consumers.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MinimalTrayItem {
    pub app_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_handle: Option<String>,
    /// Item category per SNI spec (e.g. `"ApplicationStatus"`, `"Communications"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// `true` when this item is a pure menu (no application window — should not activate).
    /// Omitted from JSON when `false`.
    #[serde(default, skip_serializing_if = "is_false")]
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

/// One row in a DBusMenu tree.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MenuItem {
    pub item_id: u32,
    pub label: String,
    pub is_submenu: bool,
}

/// Events pushed to `subscribe` consumers.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", content = "items", rename_all = "snake_case")]
pub enum TrayEvent {
    Update(Vec<MinimalTrayItem>),
}
