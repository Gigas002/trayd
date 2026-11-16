use std::time::Duration;

use tokio::io::BufReader;
use tokio::net::UnixStream;

use super::codec;
use super::protocol::{Cmd, IpcRequest, IpcResponse, OkPayload};
use super::server::IpcServer;

/// A private `dbus-daemon` plus an `IpcServer` on a temp socket, torn down on
/// drop.  The private bus keeps these tests away from the desktop session,
/// where the host would compete with the user's tray for the watcher name.
struct Server {
    bus: std::process::Child,
    handle: tokio::task::JoinHandle<()>,
    path: std::path::PathBuf,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.handle.abort();
        let _ = std::fs::remove_file(&self.path);
        let _ = self.bus.kill();
        let _ = self.bus.wait();
    }
}

/// Start a private bus and a server on it; `None` (test skipped) when
/// `dbus-daemon` is not installed.
async fn start_server(tag: &str) -> Option<Server> {
    use std::io::BufRead;

    let mut bus = match std::process::Command::new("dbus-daemon")
        .args(["--session", "--nofork", "--print-address=1"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("skipping: cannot run dbus-daemon: {e}");
            return None;
        }
    };
    let mut address = String::new();
    std::io::BufReader::new(bus.stdout.take().expect("piped"))
        .read_line(&mut address)
        .expect("read dbus-daemon address");
    let address = address.trim().to_owned();

    let path = std::env::temp_dir().join(format!("trayd-test-{}-{}.sock", tag, std::process::id()));
    let host = libtrayd::TrayHost::start_on(&address)
        .await
        .expect("start host on the private bus");
    let server = IpcServer::new(&path, host);
    let handle = tokio::spawn(async move {
        let _ = server.run().await;
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    Some(Server { bus, handle, path })
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
async fn ping_returns_pong() {
    let Some(srv) = start_server("ping").await else {
        return;
    };

    let (mut r, mut w) = connect(&srv.path).await;
    codec::write_request(&mut w, &IpcRequest::new(Cmd::Ping))
        .await
        .unwrap();
    let resp = codec::read_response(&mut r).await.unwrap().unwrap();

    assert!(matches!(resp, IpcResponse::Ok(ref ok) if ok.payload == OkPayload::Pong));
}

#[tokio::test]
async fn get_items_returns_list() {
    let Some(srv) = start_server("items").await else {
        return;
    };

    let (mut r, mut w) = connect(&srv.path).await;
    codec::write_request(&mut w, &IpcRequest::new(Cmd::GetItems))
        .await
        .unwrap();
    let resp = codec::read_response(&mut r).await.unwrap().unwrap();

    assert!(
        matches!(resp, IpcResponse::Ok(ref ok) if matches!(&ok.payload, OkPayload::Items { .. }))
    );
}

#[tokio::test]
async fn get_menu_returns_not_found_for_unknown_app() {
    let Some(srv) = start_server("menu").await else {
        return;
    };

    let (mut r, mut w) = connect(&srv.path).await;
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

    assert!(matches!(resp, IpcResponse::Err(_)));
}

#[tokio::test]
async fn activate_unknown_returns_not_found() {
    let Some(srv) = start_server("activate").await else {
        return;
    };

    let (mut r, mut w) = connect(&srv.path).await;
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

    assert!(matches!(resp, IpcResponse::Err(_)));
}

#[tokio::test]
async fn get_pixmap_unknown_returns_not_found() {
    let Some(srv) = start_server("pixmap").await else {
        return;
    };

    let (mut r, mut w) = connect(&srv.path).await;
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
async fn golden_get_menu_request_fixture() {
    // top-level (submenu_id = null)
    let req_line = r#"{"v":1,"cmd":"get_menu","app_id":"org.example.App","submenu_id":null}"#;
    let req: IpcRequest = serde_json::from_str(req_line).unwrap();
    assert!(
        matches!(req.cmd, Cmd::GetMenu { ref app_id, submenu_id: None } if app_id == "org.example.App")
    );

    // submenu (submenu_id = 2)
    let req_sub = r#"{"v":1,"cmd":"get_menu","app_id":"org.example.App","submenu_id":2}"#;
    let req_sub: IpcRequest = serde_json::from_str(req_sub).unwrap();
    assert!(
        matches!(req_sub.cmd, Cmd::GetMenu { ref app_id, submenu_id: Some(2) } if app_id == "org.example.App")
    );
}

#[tokio::test]
async fn golden_menu_response_fixture() {
    let resp_line = r#"{"v":1,"type":"menu","app_id":"org.example.App","items":[{"item_id":1,"label":"Action","is_submenu":false},{"item_id":2,"label":"Submenu","is_submenu":true}]}"#;
    let resp: IpcResponse = serde_json::from_str(resp_line).unwrap();
    match resp {
        IpcResponse::Ok(ok) => match &ok.payload {
            OkPayload::Menu { app_id, items } => {
                assert_eq!(app_id, "org.example.App");
                assert_eq!(items.len(), 2);
                assert_eq!(items[0].item_id, 1);
                assert_eq!(items[0].label, "Action");
                assert!(!items[0].is_submenu);
                assert!(items[1].is_submenu);
            }
            other => panic!("expected Menu payload, got: {other:?}"),
        },
        IpcResponse::Err(e) => panic!("expected Ok, got error: {:?}", e.error),
    }
}

#[tokio::test]
async fn golden_get_pixmap_response_fixture() {
    let resp_line = r#"{"v":1,"type":"pixmap","app_id":"org.example.App","size":22,"width":22,"height":22,"data":""}"#;
    let resp: IpcResponse = serde_json::from_str(resp_line).unwrap();
    match resp {
        IpcResponse::Ok(ok) => match &ok.payload {
            OkPayload::Pixmap {
                app_id,
                size,
                width,
                height,
                data,
            } => {
                assert_eq!(app_id, "org.example.App");
                assert_eq!(*size, 22);
                assert_eq!(*width, 22);
                assert_eq!(*height, 22);
                assert_eq!(data, "");
            }
            other => panic!("expected Pixmap payload, got: {other:?}"),
        },
        IpcResponse::Err(e) => panic!("expected Ok, got error: {:?}", e.error),
    }
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

#[tokio::test]
async fn minimal_tray_item_item_is_menu_omitted_when_false() {
    use super::protocol::MinimalTrayItem;
    let item = MinimalTrayItem {
        app_id: "org.example.App".into(),
        title: Some("Example App".into()),
        status: "Active".into(),
        icon_handle: Some("example-app".into()),
        category: Some("ApplicationStatus".into()),
        item_is_menu: false,
        tooltip_title: None,
        tooltip_description: None,
        overlay_icon_handle: None,
    };
    let json = serde_json::to_string(&item).unwrap();
    assert!(
        !json.contains("item_is_menu"),
        "item_is_menu must be omitted when false; got: {json}"
    );
    assert!(
        json.contains("ApplicationStatus"),
        "category must be present; got: {json}"
    );
}

#[tokio::test]
async fn minimal_tray_item_item_is_menu_present_when_true() {
    use super::protocol::MinimalTrayItem;
    let item = MinimalTrayItem {
        app_id: "org.example.MenuApp".into(),
        title: Some("Menu App".into()),
        status: "Active".into(),
        icon_handle: None,
        category: None,
        item_is_menu: true,
        tooltip_title: Some("Menu App Tooltip".into()),
        tooltip_description: Some("A tooltip description".into()),
        overlay_icon_handle: Some("overlay-badge".into()),
    };
    let json = serde_json::to_string(&item).unwrap();
    assert!(
        json.contains(r#""item_is_menu":true"#),
        "item_is_menu must be present when true; got: {json}"
    );
    assert!(
        json.contains(r#""tooltip_title":"Menu App Tooltip""#),
        "tooltip_title must be present; got: {json}"
    );
    assert!(
        json.contains(r#""tooltip_description":"A tooltip description""#),
        "tooltip_description must be present; got: {json}"
    );
    assert!(
        json.contains(r#""overlay_icon_handle":"overlay-badge""#),
        "overlay_icon_handle must be present; got: {json}"
    );
}
