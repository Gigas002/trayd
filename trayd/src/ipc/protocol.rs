//! Wire request/response types for the trayd IPC v1 protocol.
//!
//! All messages travel as **NDJSON** over a Unix domain socket.
//! Every request includes `"v": 1`; every response likewise.
//!
//! See `docs/IPC.md` for the full specification.

use serde::{Deserialize, Serialize};

// ── Version ───────────────────────────────────────────────────────────────────

/// The only supported IPC protocol version.
pub const PROTOCOL_VERSION: u32 = 1;

// ── Request ───────────────────────────────────────────────────────────────────

/// Top-level request envelope: `{ "v": 1, "method": "...", <params> }`.
///
/// The `method` field and any extra params are inlined into the same JSON object
/// via `#[serde(flatten)]` on the internally-tagged [`Method`] enum.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RequestEnvelope {
    /// Protocol version — must be [`PROTOCOL_VERSION`].
    pub v: u32,
    #[serde(flatten)]
    pub method: Method,
}

impl RequestEnvelope {
    /// Construct a v1 request envelope.
    pub fn new(method: Method) -> Self {
        Self {
            v: PROTOCOL_VERSION,
            method,
        }
    }
}

/// All v1 IPC methods. Serialized with `"method": "<snake_case>"` as the tag.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum Method {
    /// Health / version check.
    Ping,
    /// Return all registered tray items.
    List,
    /// Subscribe to a live stream of [`HostEvent`]s. Long-lived connection.
    Subscribe,
    /// Fetch a pixmap for an item at the requested icon size (pixels).
    GetPixmap { item_id: String, size: u32 },
    /// Primary left-click activation.
    Activate { item_id: String },
    /// Middle-click / secondary activation.
    SecondaryActivate { item_id: String },
    /// Scroll event on the item's icon area.
    Scroll {
        item_id: String,
        direction: ScrollDirection,
        delta: i32,
    },
    /// Open a DBusMenu tree for an item; returns a [`MenuSession`].
    ///
    /// `parent_id` selects the subtree root (0 = top-level menu).
    MenuOpen {
        item_id: String,
        #[serde(default)]
        parent_id: i32,
    },
    /// Select a menu node within an open session.
    ///
    /// For leaf nodes returns `{ "type": "ok" }`.
    /// For sub-menu nodes returns a new `{ "type": "menu_opened", ... }`.
    MenuSelect { session_id: String, node_id: String },
    /// Close/discard a menu session.
    MenuClose { session_id: String },
}

// ── Response ──────────────────────────────────────────────────────────────────

/// Top-level response envelope.
///
/// Exactly one of `result` or `error` is present:
/// - Success: `{ "v": 1, "result": { "type": "...", ... } }`
/// - Failure: `{ "v": 1, "error": { "code": "...", "message": "..." } }`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    pub v: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<ResultPayload>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorPayload>,
}

impl Response {
    /// Build a success response.
    pub fn ok(result: ResultPayload) -> Self {
        Self {
            v: PROTOCOL_VERSION,
            result: Some(result),
            error: None,
        }
    }

    /// Build an error response.
    pub fn err(error: ErrorPayload) -> Self {
        Self {
            v: PROTOCOL_VERSION,
            result: None,
            error: Some(error),
        }
    }

    /// Consume the response into a `Result`.
    pub fn into_result(self) -> Result<ResultPayload, ErrorPayload> {
        if let Some(result) = self.result {
            Ok(result)
        } else if let Some(error) = self.error {
            Err(error)
        } else {
            Err(ErrorPayload::internal(
                "malformed response: neither `result` nor `error` is set",
            ))
        }
    }
}

/// Success payload variants. Serialized as `{ "type": "<snake_case>", ... }`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResultPayload {
    /// Response to `ping`: carries the daemon version string.
    Pong { version: String },
    /// Response to `list`: carries all current tray items.
    List { items: Vec<ItemInfo> },
    /// Ack sent immediately after a `subscribe` request before streaming begins.
    Subscribed,
    /// Response to `get_pixmap`.
    Pixmap(PixmapPayload),
    /// Generic "void" success (activate, scroll, menu_close, leaf menu_select).
    Ok,
    /// Response to `menu_open` and sub-menu `menu_select`.
    MenuOpened(MenuSession),
}

// ── Event (subscribe stream) ──────────────────────────────────────────────────

/// A single event pushed on a `subscribe` connection.
/// Wire format: `{ "v": 1, "event": { "type": "...", ... } }`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventEnvelope {
    pub v: u32,
    pub event: HostEvent,
}

impl EventEnvelope {
    pub fn new(event: HostEvent) -> Self {
        Self {
            v: PROTOCOL_VERSION,
            event,
        }
    }
}

/// Host-emitted tray events. Serialized as `{ "type": "<snake_case>", ... }`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HostEvent {
    /// A new StatusNotifier item appeared.
    ItemAdded { item: ItemInfo },
    /// A previously registered item disappeared.
    ItemRemoved { id: String },
    /// An item's properties (title, status, icon) changed.
    ItemUpdated { item: ItemInfo },
    /// A menu tree for an item changed.
    MenuChanged { item_id: String },
}

// ── Error ─────────────────────────────────────────────────────────────────────

/// Stable error descriptor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorPayload {
    pub code: ErrorCode,
    pub message: String,
}

/// Stable error codes. Wire representation is `SCREAMING_SNAKE_CASE`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    /// Item or resource not found.
    NotFound,
    /// D-Bus operation failed.
    BusFailed,
    /// The referenced menu session does not exist or has expired.
    InvalidSession,
    /// The request is malformed or uses an unsupported protocol version.
    InvalidRequest,
    /// Unexpected internal daemon error.
    InternalError,
}

impl std::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::NotFound => "NOT_FOUND",
            Self::BusFailed => "BUS_FAILED",
            Self::InvalidSession => "INVALID_SESSION",
            Self::InvalidRequest => "INVALID_REQUEST",
            Self::InternalError => "INTERNAL_ERROR",
        };
        f.write_str(s)
    }
}

impl ErrorPayload {
    pub fn not_found(msg: impl Into<String>) -> Self {
        Self {
            code: ErrorCode::NotFound,
            message: msg.into(),
        }
    }

    pub fn bus_failed(msg: impl Into<String>) -> Self {
        Self {
            code: ErrorCode::BusFailed,
            message: msg.into(),
        }
    }

    pub fn invalid_session(msg: impl Into<String>) -> Self {
        Self {
            code: ErrorCode::InvalidSession,
            message: msg.into(),
        }
    }

    pub fn invalid_request(msg: impl Into<String>) -> Self {
        Self {
            code: ErrorCode::InvalidRequest,
            message: msg.into(),
        }
    }

    pub fn internal(msg: impl Into<String>) -> Self {
        Self {
            code: ErrorCode::InternalError,
            message: msg.into(),
        }
    }
}

// ── Domain value types ────────────────────────────────────────────────────────

/// Summary of a single registered StatusNotifier item.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ItemInfo {
    /// Stable identity: D-Bus service name (e.g. `org.kde.plasma.nm`).
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub status: ItemStatus,
    pub has_attention_icon: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tooltip: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
}

/// `StatusNotifierItem.Status` values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemStatus {
    Passive,
    Active,
    NeedsAttention,
}

/// Scroll direction for `scroll` requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScrollDirection {
    Up,
    Down,
    Left,
    Right,
}

/// Pixel data returned by `get_pixmap`.
///
/// `data` is **base64-encoded** pixel bytes in the declared `format`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PixmapPayload {
    pub item_id: String,
    pub format: PixmapFormat,
    pub width: u32,
    pub height: u32,
    /// Base64-encoded pixel data.
    pub data: String,
}

/// Pixel encoding used in [`PixmapPayload`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PixmapFormat {
    /// 32-bit ARGB, big-endian, row-major.
    Argb32,
    /// Portable Network Graphics.
    Png,
}

/// A single node in a DBusMenu tree.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MenuNodeInfo {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub enabled: bool,
    pub visible: bool,
    pub is_separator: bool,
    /// `"submenu"` when the node has nested children; `None` for leaf nodes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children_display: Option<String>,
}

/// An open DBusMenu session returned by `menu_open` (or sub-menu `menu_select`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MenuSession {
    pub session_id: String,
    pub item_id: String,
    pub nodes: Vec<MenuNodeInfo>,
}
