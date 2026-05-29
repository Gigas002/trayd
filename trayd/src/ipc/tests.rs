use std::time::Duration;

use tokio::io::BufReader;
use tokio::net::UnixStream;

use super::codec;
use super::protocol::{Cmd, IpcRequest, IpcResponse, OkPayload};
use super::server::IpcServer;

fn temp_socket(tag: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("trayd-test-{}-{}.sock", tag, std::process::id()))
}

/// Starts a real IpcServer backed by a real TrayHost.
/// Requires a running D-Bus session bus — mark callers `#[ignore]` in CI.
async fn start_server(path: &std::path::Path) -> tokio::task::JoinHandle<()> {
    let host = libtrayd::TrayHost::start()
        .await
        .expect("D-Bus session bus required for this test");
    let server = IpcServer::new(path, host);
    let handle = tokio::spawn(async move {
        let _ = server.run().await;
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    handle
}

async fn connect(
    path: &std::path::Path,
) -> (
    tokio::io::BufReader<tokio::net::unix::OwnedReadHalf>,
    tokio::net::unix::OwnedWriteHalf,
) {
    let stream = UnixStream::connect(path).await.unwrap();
    let (r, w) = stream.into_split();
    (BufReader::new(r), w)
}

#[tokio::test]
#[ignore = "requires D-Bus session bus"]
async fn ping_returns_pong() {
    let path = temp_socket("ping");
    let handle = start_server(&path).await;

    let (mut r, mut w) = connect(&path).await;
    codec::write_request(&mut w, &IpcRequest::new(Cmd::Ping))
        .await
        .unwrap();
    let resp = codec::read_response(&mut r).await.unwrap().unwrap();

    handle.abort();
    let _ = std::fs::remove_file(&path);

    assert!(matches!(resp, IpcResponse::Ok(ref ok) if ok.payload == OkPayload::Pong));
}

#[tokio::test]
#[ignore = "requires D-Bus session bus"]
async fn get_items_returns_list() {
    let path = temp_socket("items");
    let handle = start_server(&path).await;

    let (mut r, mut w) = connect(&path).await;
    codec::write_request(&mut w, &IpcRequest::new(Cmd::GetItems))
        .await
        .unwrap();
    let resp = codec::read_response(&mut r).await.unwrap().unwrap();

    handle.abort();
    let _ = std::fs::remove_file(&path);

    assert!(
        matches!(resp, IpcResponse::Ok(ref ok) if matches!(&ok.payload, OkPayload::Items { .. }))
    );
}

#[tokio::test]
#[ignore = "requires D-Bus session bus; GetMenu is Phase 3 (returns NotImplemented)"]
async fn get_menu_returns_not_implemented() {
    let path = temp_socket("menu");
    let handle = start_server(&path).await;

    let (mut r, mut w) = connect(&path).await;
    codec::write_request(
        &mut w,
        &IpcRequest::new(Cmd::GetMenu {
            app_id: "org.example.App".into(),
            submenu_id: None,
        }),
    )
    .await
    .unwrap();
    let resp = codec::read_response(&mut r).await.unwrap().unwrap();

    handle.abort();
    let _ = std::fs::remove_file(&path);

    assert!(matches!(resp, IpcResponse::Err(_)));
}

#[tokio::test]
#[ignore = "requires D-Bus session bus; returns NotFound when no real app is registered"]
async fn activate_unknown_returns_not_found() {
    let path = temp_socket("activate");
    let handle = start_server(&path).await;

    let (mut r, mut w) = connect(&path).await;
    codec::write_request(
        &mut w,
        &IpcRequest::new(Cmd::Activate {
            app_id: "org.example.App".into(),
            item_id: 0,
        }),
    )
    .await
    .unwrap();
    let resp = codec::read_response(&mut r).await.unwrap().unwrap();

    handle.abort();
    let _ = std::fs::remove_file(&path);

    assert!(matches!(resp, IpcResponse::Err(_)));
}

#[tokio::test]
#[ignore = "requires D-Bus session bus; returns NotFound when no real app is registered"]
async fn get_pixmap_unknown_returns_not_found() {
    let path = temp_socket("pixmap");
    let handle = start_server(&path).await;

    let (mut r, mut w) = connect(&path).await;
    codec::write_request(
        &mut w,
        &IpcRequest::new(Cmd::GetPixmap {
            app_id: "org.example.App".into(),
            size: 22,
        }),
    )
    .await
    .unwrap();
    let resp = codec::read_response(&mut r).await.unwrap().unwrap();

    handle.abort();
    let _ = std::fs::remove_file(&path);

    assert!(matches!(resp, IpcResponse::Err(_)));
}

#[tokio::test]
async fn codec_roundtrip_request() {
    let req = IpcRequest::new(Cmd::GetMenu {
        app_id: "foo".into(),
        submenu_id: Some(3),
    });
    let json = serde_json::to_string(&req).unwrap();
    let decoded: IpcRequest = serde_json::from_str(&json).unwrap();
    assert!(
        matches!(decoded.cmd, Cmd::GetMenu { ref app_id, submenu_id: Some(3) } if app_id == "foo")
    );
}

#[tokio::test]
async fn codec_roundtrip_response_pong() {
    let resp = IpcResponse::ok(OkPayload::Pong);
    let json = serde_json::to_string(&resp).unwrap();
    let decoded: IpcResponse = serde_json::from_str(&json).unwrap();
    assert!(matches!(decoded, IpcResponse::Ok(ref ok) if ok.payload == OkPayload::Pong));
}

#[tokio::test]
async fn codec_roundtrip_error_response() {
    use super::protocol::ErrorCode;
    let resp = IpcResponse::err(ErrorCode::NotFound, "no items");
    let json = serde_json::to_string(&resp).unwrap();
    assert!(json.contains("NOT_FOUND"));
    let decoded: IpcResponse = serde_json::from_str(&json).unwrap();
    assert!(matches!(decoded, IpcResponse::Err(_)));
}

#[tokio::test]
async fn golden_ping_fixture() {
    let req_line = r#"{"v":1,"cmd":"ping"}"#;
    let resp_line = r#"{"v":1,"type":"pong"}"#;

    let req: IpcRequest = serde_json::from_str(req_line).unwrap();
    assert!(matches!(req.cmd, Cmd::Ping));

    let resp: IpcResponse = serde_json::from_str(resp_line).unwrap();
    assert!(matches!(resp, IpcResponse::Ok(ref ok) if ok.payload == OkPayload::Pong));
}
