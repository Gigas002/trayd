//! IPC unit and integration tests.
//!
//! Codec and protocol tests are synchronous.  Handler and integration tests
//! use `#[tokio::test]` because the `Handler` trait is now async.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use super::client::Client;
use super::codec::{decode, encode};
use super::protocol::*;
use super::server::{Handler, Server, StubHandler};

// ── Helpers ───────────────────────────────────────────────────────────────────

static TEST_SOCKET_COUNTER: AtomicU32 = AtomicU32::new(0);

fn test_socket_path() -> PathBuf {
    let id = TEST_SOCKET_COUNTER.fetch_add(1, Ordering::SeqCst);
    PathBuf::from(format!("/tmp/trayd-test-{}-{id}.sock", std::process::id()))
}

async fn spawn_server(path: PathBuf, handler: StubHandler) -> tokio::sync::watch::Sender<bool> {
    let server = Server::bind(&path).expect("bind");
    let handler = Arc::new(handler);
    let (tx, rx) = tokio::sync::watch::channel(false);
    tokio::spawn(async move {
        server.run(handler, rx).await.ok();
    });
    tokio::time::sleep(Duration::from_millis(5)).await;
    tx
}

// ── Codec unit tests ──────────────────────────────────────────────────────────

#[test]
fn codec_encode_ping_request() {
    let req = RequestEnvelope::new(Method::Ping);
    let bytes = encode(&req).unwrap();
    let json = std::str::from_utf8(&bytes).unwrap();
    assert!(json.contains("\"v\":1"));
    assert!(json.contains("\"method\":\"ping\""));
    assert!(json.ends_with('\n'));
}

#[test]
fn codec_decode_ping_request() {
    let line = r#"{"v":1,"method":"ping"}"#;
    let req: RequestEnvelope = decode(line).unwrap();
    assert_eq!(req.v, 1);
    assert_eq!(req.method, Method::Ping);
}

#[test]
fn codec_roundtrip_request_with_params() {
    let req = RequestEnvelope::new(Method::Activate {
        item_id: "org.kde.plasma.nm".to_owned(),
    });
    let encoded = encode(&req).unwrap();
    let decoded: RequestEnvelope = decode(std::str::from_utf8(&encoded).unwrap()).unwrap();
    assert_eq!(req, decoded);
}

#[test]
fn codec_roundtrip_scroll_request() {
    let req = RequestEnvelope::new(Method::Scroll {
        item_id: "app".to_owned(),
        direction: ScrollDirection::Up,
        delta: 3,
    });
    let bytes = encode(&req).unwrap();
    let decoded: RequestEnvelope = decode(std::str::from_utf8(&bytes).unwrap()).unwrap();
    assert_eq!(req, decoded);
}

#[test]
fn codec_roundtrip_response_pong() {
    let resp = Response::ok(ResultPayload::Pong {
        version: "0.1.0".to_owned(),
    });
    let bytes = encode(&resp).unwrap();
    let decoded: Response = decode(std::str::from_utf8(&bytes).unwrap()).unwrap();
    assert_eq!(resp, decoded);
}

#[test]
fn codec_roundtrip_response_list() {
    let item = ItemInfo {
        id: "org.kde.plasma.nm".to_owned(),
        title: Some("Network Manager".to_owned()),
        status: ItemStatus::Active,
        has_attention_icon: false,
        tooltip: None,
        category: None,
    };
    let resp = Response::ok(ResultPayload::List { items: vec![item] });
    let bytes = encode(&resp).unwrap();
    let decoded: Response = decode(std::str::from_utf8(&bytes).unwrap()).unwrap();
    assert_eq!(resp, decoded);
}

#[test]
fn codec_roundtrip_response_error() {
    let resp = Response::err(ErrorPayload::not_found("item not found: foo"));
    let bytes = encode(&resp).unwrap();
    let decoded: Response = decode(std::str::from_utf8(&bytes).unwrap()).unwrap();
    assert_eq!(resp, decoded);
    assert!(decoded.error.is_some());
    assert_eq!(decoded.error.unwrap().code, ErrorCode::NotFound);
}

#[test]
fn codec_roundtrip_event_envelope() {
    let ev = EventEnvelope::new(HostEvent::ItemRemoved {
        id: "org.kde.plasma.nm".to_owned(),
    });
    let bytes = encode(&ev).unwrap();
    let decoded: EventEnvelope = decode(std::str::from_utf8(&bytes).unwrap()).unwrap();
    assert_eq!(ev, decoded);
}

// ── Golden fixture tests ──────────────────────────────────────────────────────

fn fixture(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../examples/ipc-examples")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("could not read fixture {name}: {e}"))
}

#[test]
fn golden_ping_request() {
    let json = fixture("ping-request.json");
    let req: RequestEnvelope = decode(json.trim()).unwrap();
    assert_eq!(req.v, 1);
    assert_eq!(req.method, Method::Ping);
}

#[test]
fn golden_ping_response() {
    let json = fixture("ping-response.json");
    let resp: Response = decode(json.trim()).unwrap();
    assert_eq!(resp.v, 1);
    assert!(matches!(resp.result, Some(ResultPayload::Pong { .. })));
}

#[test]
fn golden_list_request() {
    let json = fixture("list-request.json");
    let req: RequestEnvelope = decode(json.trim()).unwrap();
    assert_eq!(req.method, Method::List);
}

#[test]
fn golden_list_response_empty() {
    let json = fixture("list-response-empty.json");
    let resp: Response = decode(json.trim()).unwrap();
    assert!(matches!(
        resp.result,
        Some(ResultPayload::List { ref items }) if items.is_empty()
    ));
}

#[test]
fn golden_list_response_with_item() {
    let json = fixture("list-response.json");
    let resp: Response = decode(json.trim()).unwrap();
    let items = match resp.result {
        Some(ResultPayload::List { items }) => items,
        other => panic!("expected List result, got {other:?}"),
    };
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].id, "org.kde.plasma.nm");
}

#[test]
fn golden_activate_request() {
    let json = fixture("activate-request.json");
    let req: RequestEnvelope = decode(json.trim()).unwrap();
    assert!(matches!(
        req.method,
        Method::Activate { ref item_id } if item_id == "org.kde.plasma.nm"
    ));
}

#[test]
fn golden_ok_response() {
    let json = fixture("ok-response.json");
    let resp: Response = decode(json.trim()).unwrap();
    assert!(matches!(resp.result, Some(ResultPayload::Ok)));
}

#[test]
fn golden_error_not_found() {
    let json = fixture("error-not-found.json");
    let resp: Response = decode(json.trim()).unwrap();
    let err = resp.error.expect("expected error payload");
    assert_eq!(err.code, ErrorCode::NotFound);
}

#[test]
fn golden_subscribe_request() {
    let json = fixture("subscribe-request.json");
    let req: RequestEnvelope = decode(json.trim()).unwrap();
    assert_eq!(req.method, Method::Subscribe);
}

#[test]
fn golden_event_item_added() {
    let json = fixture("event-item-added.json");
    let env: EventEnvelope = decode(json.trim()).unwrap();
    assert!(matches!(env.event, HostEvent::ItemAdded { .. }));
}

#[test]
fn golden_event_item_removed() {
    let json = fixture("event-item-removed.json");
    let env: EventEnvelope = decode(json.trim()).unwrap();
    assert!(matches!(
        env.event,
        HostEvent::ItemRemoved { ref id } if id == "org.kde.plasma.nm"
    ));
}

// ── StubHandler unit tests (async — Handler::handle is now async) ─────────────

#[tokio::test]
async fn stub_handler_ping() {
    let h = StubHandler::new();
    let resp = h.handle(&Method::Ping).await;
    assert!(matches!(resp.result, Some(ResultPayload::Pong { .. })));
}

#[tokio::test]
async fn stub_handler_list_empty() {
    let h = StubHandler::new();
    let resp = h.handle(&Method::List).await;
    assert!(matches!(resp.result, Some(ResultPayload::List { ref items }) if items.is_empty()));
}

#[tokio::test]
async fn stub_handler_activate_not_found() {
    let h = StubHandler::new();
    let resp = h
        .handle(&Method::Activate {
            item_id: "missing".to_owned(),
        })
        .await;
    assert!(resp.error.is_some());
    assert_eq!(resp.error.unwrap().code, ErrorCode::NotFound);
}

#[tokio::test]
async fn stub_handler_activate_found() {
    let item = StubHandler::mock_item("app");
    let h = StubHandler::new().with_items(vec![item]);
    let resp = h
        .handle(&Method::Activate {
            item_id: "app".to_owned(),
        })
        .await;
    assert!(matches!(resp.result, Some(ResultPayload::Ok)));
}

// ── Integration tests with a real Unix socket ─────────────────────────────────

#[tokio::test]
async fn integration_ping_roundtrip() {
    let path = test_socket_path();
    let tx = spawn_server(path.clone(), StubHandler::new()).await;

    let mut client = Client::connect(&path).await.expect("connect");
    let resp = client.send(Method::Ping).await.expect("send");
    assert!(matches!(resp.result, Some(ResultPayload::Pong { .. })));

    tx.send(true).ok();
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn integration_list_empty() {
    let path = test_socket_path();
    let tx = spawn_server(path.clone(), StubHandler::new()).await;

    let mut client = Client::connect(&path).await.expect("connect");
    let resp = client.send(Method::List).await.expect("send");
    assert!(matches!(resp.result, Some(ResultPayload::List { ref items }) if items.is_empty()));

    tx.send(true).ok();
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn integration_list_with_items() {
    let path = test_socket_path();
    let item = StubHandler::mock_item("org.kde.plasma.nm");
    let handler = StubHandler::new().with_items(vec![item]);
    let tx = spawn_server(path.clone(), handler).await;

    let mut client = Client::connect(&path).await.expect("connect");
    let resp = client.send(Method::List).await.expect("send");
    let items = match resp.result {
        Some(ResultPayload::List { items }) => items,
        other => panic!("expected List, got {other:?}"),
    };
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].id, "org.kde.plasma.nm");

    tx.send(true).ok();
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn integration_activate_not_found() {
    let path = test_socket_path();
    let tx = spawn_server(path.clone(), StubHandler::new()).await;

    let mut client = Client::connect(&path).await.expect("connect");
    let resp = client
        .send(Method::Activate {
            item_id: "missing".to_owned(),
        })
        .await
        .expect("send");
    assert!(resp.error.is_some());
    assert_eq!(resp.error.unwrap().code, ErrorCode::NotFound);

    tx.send(true).ok();
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn integration_multiple_requests_on_same_connection() {
    let path = test_socket_path();
    let tx = spawn_server(path.clone(), StubHandler::new()).await;

    let mut client = Client::connect(&path).await.expect("connect");

    let r1 = client.send(Method::Ping).await.expect("ping");
    assert!(r1.result.is_some());

    let r2 = client.send(Method::List).await.expect("list");
    assert!(r2.result.is_some());

    tx.send(true).ok();
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn integration_invalid_protocol_version() {
    use super::codec::{FramedReader, FramedWriter};
    use tokio::net::UnixStream;

    let path = test_socket_path();
    let tx = spawn_server(path.clone(), StubHandler::new()).await;

    let stream = UnixStream::connect(&path).await.expect("connect");
    let (r, w) = stream.into_split();
    let mut reader = FramedReader::new(r);
    let mut writer = FramedWriter::new(w);

    let bad = serde_json::json!({"v": 99, "method": "ping"});
    writer.write_frame(&bad).await.expect("write");

    let resp: Response = reader.read_frame().await.expect("read").expect("frame");
    assert!(resp.error.is_some());
    assert_eq!(resp.error.unwrap().code, ErrorCode::InvalidRequest);

    tx.send(true).ok();
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn integration_subscribe_receives_event() {
    use super::codec::{FramedReader, FramedWriter};
    use tokio::net::UnixStream;

    let path = test_socket_path();
    let handler = Arc::new(StubHandler::new());
    let server = Server::bind(&path).expect("bind");
    let h_clone = Arc::clone(&handler);
    let (tx, rx) = tokio::sync::watch::channel(false);
    tokio::spawn(async move {
        server.run(h_clone, rx).await.ok();
    });
    tokio::time::sleep(Duration::from_millis(5)).await;

    let stream = UnixStream::connect(&path).await.expect("connect");
    let (r, w) = stream.into_split();
    let mut reader = FramedReader::new(r);
    let mut writer = FramedWriter::new(w);

    writer
        .write_frame(&RequestEnvelope::new(Method::Subscribe))
        .await
        .expect("write subscribe");

    let ack: Response = reader
        .read_frame()
        .await
        .expect("read ack")
        .expect("ack frame");
    assert!(matches!(ack.result, Some(ResultPayload::Subscribed)));

    tokio::time::sleep(Duration::from_millis(5)).await;
    handler.emit(HostEvent::ItemRemoved {
        id: "org.kde.plasma.nm".to_owned(),
    });

    let env: EventEnvelope = tokio::time::timeout(Duration::from_millis(200), async {
        reader
            .read_frame()
            .await
            .expect("read event")
            .expect("event frame")
    })
    .await
    .expect("event not received within timeout");

    assert!(matches!(
        env.event,
        HostEvent::ItemRemoved { ref id } if id == "org.kde.plasma.nm"
    ));

    tx.send(true).ok();
    let _ = std::fs::remove_file(&path);
}

// ── Phase 3: menu codec roundtrips ────────────────────────────────────────────

#[test]
fn codec_roundtrip_menu_open() {
    let req = RequestEnvelope::new(Method::MenuOpen {
        item_id: "org.kde.plasma.nm".to_owned(),
        parent_id: 0,
    });
    let bytes = encode(&req).unwrap();
    let decoded: RequestEnvelope = decode(std::str::from_utf8(&bytes).unwrap()).unwrap();
    assert_eq!(req, decoded);
}

#[test]
fn codec_roundtrip_menu_select() {
    let req = RequestEnvelope::new(Method::MenuSelect {
        session_id: "s-1".to_owned(),
        node_id: "42".to_owned(),
    });
    let bytes = encode(&req).unwrap();
    let decoded: RequestEnvelope = decode(std::str::from_utf8(&bytes).unwrap()).unwrap();
    assert_eq!(req, decoded);
}

#[test]
fn codec_roundtrip_menu_close() {
    let req = RequestEnvelope::new(Method::MenuClose {
        session_id: "s-1".to_owned(),
    });
    let bytes = encode(&req).unwrap();
    let decoded: RequestEnvelope = decode(std::str::from_utf8(&bytes).unwrap()).unwrap();
    assert_eq!(req, decoded);
}

#[test]
fn codec_roundtrip_menu_opened_result() {
    let session = MenuSession {
        session_id: "s-1".to_owned(),
        item_id: "org.kde.plasma.nm".to_owned(),
        nodes: vec![
            MenuNodeInfo {
                id: "1".to_owned(),
                label: Some("Enable Networking".to_owned()),
                enabled: true,
                visible: true,
                is_separator: false,
                children_display: None,
            },
            MenuNodeInfo {
                id: "2".to_owned(),
                label: Some("VPN".to_owned()),
                enabled: true,
                visible: true,
                is_separator: false,
                children_display: Some("submenu".to_owned()),
            },
        ],
    };
    let resp = Response::ok(ResultPayload::MenuOpened(session));
    let bytes = encode(&resp).unwrap();
    let decoded: Response = decode(std::str::from_utf8(&bytes).unwrap()).unwrap();
    assert_eq!(resp, decoded);
    assert!(matches!(decoded.result, Some(ResultPayload::MenuOpened(_))));
}

#[test]
fn codec_roundtrip_menu_changed_event() {
    let ev = EventEnvelope::new(HostEvent::MenuChanged {
        item_id: "org.kde.plasma.nm".to_owned(),
    });
    let bytes = encode(&ev).unwrap();
    let decoded: EventEnvelope = decode(std::str::from_utf8(&bytes).unwrap()).unwrap();
    assert_eq!(ev, decoded);
    assert!(matches!(decoded.event, HostEvent::MenuChanged { .. }));
}

// ── Phase 3: StubHandler menu tests ──────────────────────────────────────────

#[tokio::test]
async fn stub_menu_open_not_found() {
    let h = StubHandler::new();
    let resp = h
        .handle(&Method::MenuOpen {
            item_id: "missing".to_owned(),
            parent_id: 0,
        })
        .await;
    assert!(resp.error.is_some());
    assert_eq!(resp.error.unwrap().code, ErrorCode::NotFound);
}

#[tokio::test]
async fn stub_menu_open_found() {
    let item = StubHandler::mock_item("app");
    let h = StubHandler::new().with_items(vec![item]);
    let resp = h
        .handle(&Method::MenuOpen {
            item_id: "app".to_owned(),
            parent_id: 0,
        })
        .await;
    assert!(resp.error.is_none());
    assert!(matches!(resp.result, Some(ResultPayload::MenuOpened(_))));
}

#[tokio::test]
async fn stub_menu_select_invalid_session() {
    let h = StubHandler::new();
    let resp = h
        .handle(&Method::MenuSelect {
            session_id: "no-such".to_owned(),
            node_id: "1".to_owned(),
        })
        .await;
    assert!(resp.error.is_some());
    assert_eq!(resp.error.unwrap().code, ErrorCode::InvalidSession);
}

#[tokio::test]
async fn stub_menu_close_invalid_session() {
    let h = StubHandler::new();
    let resp = h
        .handle(&Method::MenuClose {
            session_id: "no-such".to_owned(),
        })
        .await;
    assert!(resp.error.is_some());
    assert_eq!(resp.error.unwrap().code, ErrorCode::InvalidSession);
}

// ── Phase 3: integration tests for menu open / select / close ─────────────────

#[tokio::test]
async fn integration_menu_open_select_close() {
    let path = test_socket_path();
    let item = StubHandler::mock_item("app");
    let handler = StubHandler::new().with_items(vec![item]);
    let tx = spawn_server(path.clone(), handler).await;

    let mut client = Client::connect(&path).await.expect("connect");

    // open
    let resp = client
        .send(Method::MenuOpen {
            item_id: "app".to_owned(),
            parent_id: 0,
        })
        .await
        .expect("menu_open");
    let session = match resp.result {
        Some(ResultPayload::MenuOpened(s)) => s,
        other => panic!("expected MenuOpened, got {other:?}"),
    };
    assert_eq!(session.item_id, "app");
    assert!(!session.nodes.is_empty());
    let session_id = session.session_id.clone();
    let first_node_id = session.nodes[0].id.clone();

    // select leaf
    let resp = client
        .send(Method::MenuSelect {
            session_id: session_id.clone(),
            node_id: first_node_id,
        })
        .await
        .expect("menu_select");
    assert!(matches!(resp.result, Some(ResultPayload::Ok)));

    // close
    let resp = client
        .send(Method::MenuClose {
            session_id: session_id.clone(),
        })
        .await
        .expect("menu_close");
    assert!(matches!(resp.result, Some(ResultPayload::Ok)));

    // second close should fail
    let resp = client
        .send(Method::MenuClose { session_id })
        .await
        .expect("menu_close again");
    assert!(resp.error.is_some());
    assert_eq!(resp.error.unwrap().code, ErrorCode::InvalidSession);

    tx.send(true).ok();
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn integration_menu_open_missing_item() {
    let path = test_socket_path();
    let tx = spawn_server(path.clone(), StubHandler::new()).await;

    let mut client = Client::connect(&path).await.expect("connect");
    let resp = client
        .send(Method::MenuOpen {
            item_id: "ghost".to_owned(),
            parent_id: 0,
        })
        .await
        .expect("send");
    assert!(resp.error.is_some());
    assert_eq!(resp.error.unwrap().code, ErrorCode::NotFound);

    tx.send(true).ok();
    let _ = std::fs::remove_file(&path);
}
