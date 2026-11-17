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
    host: libtrayd::TrayHost,
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
    let server = IpcServer::new(&path, host.clone());
    let handle = tokio::spawn(async move {
        let _ = server.run().await;
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    Some(Server {
        bus,
        host,
        handle,
        path,
    })
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
                // Fields added later default to a plain, enabled row.
                assert!(items[0].enabled && !items[0].separator);
                assert_eq!(items[0].checked, None);
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

/// Send raw bytes, then read every reply line until the server closes or
/// 2 s pass.
async fn raw_exchange(path: &std::path::Path, bytes: &[u8]) -> Vec<String> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
    let (mut r, mut w) = connect(path).await;
    // The server may close mid-write on oversized input.
    let _ = w.write_all(bytes).await;
    let mut lines = Vec::new();
    loop {
        let mut line = String::new();
        match tokio::time::timeout(Duration::from_secs(2), r.read_line(&mut line)).await {
            Ok(Ok(0)) | Ok(Err(_)) | Err(_) => return lines,
            Ok(Ok(_)) => lines.push(line.trim().to_owned()),
        }
    }
}

#[tokio::test]
async fn bad_requests_get_invalid_request_and_the_connection_stays_open() {
    let Some(srv) = start_server("badreq").await else {
        return;
    };
    let lines = raw_exchange(
        &srv.path,
        b"not json\n{\"v\":1,\"cmd\":\"frobnicate\"}\n{\"v\":2,\"cmd\":\"ping\"}\n{\"v\":1,\"cmd\":\"ping\"}\n",
    )
    .await;
    assert_eq!(lines.len(), 4, "{lines:?}");
    for line in &lines[..3] {
        assert!(line.contains("\"INVALID_REQUEST\""), "{line}");
    }
    assert_eq!(lines[3], r#"{"v":1,"type":"pong"}"#);
}

#[tokio::test]
async fn an_oversized_request_is_refused_without_buffering_it() {
    let Some(srv) = start_server("huge").await else {
        return;
    };
    let huge = vec![b'a'; 4 << 20]; // 4 MiB, no newline
    let lines = raw_exchange(&srv.path, &huge).await;
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].contains("\"INVALID_REQUEST\""), "{}", lines[0]);
}

#[tokio::test]
async fn codec_rejects_lines_over_the_limit() {
    let mut ok = vec![b' '; codec::MAX_REQUEST_LEN - 30];
    ok.extend_from_slice(b"{\"v\":1,\"cmd\":\"ping\"}\n");
    let mut reader = BufReader::new(&ok[..]);
    assert!(codec::read_request(&mut reader).await.unwrap().is_some());

    let long = vec![b' '; codec::MAX_REQUEST_LEN + 1];
    let mut reader = BufReader::new(&long[..]);
    assert!(codec::read_request(&mut reader).await.is_err());
}

#[test]
fn prepare_socket_path_replaces_only_sockets() {
    use super::server::prepare_socket_path;
    let dir = std::env::temp_dir().join(format!("trayd-prep-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    let file = dir.join("precious.txt");
    std::fs::write(&file, "keep me").unwrap();
    assert!(prepare_socket_path(&file).is_err());
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "keep me");

    let stale = dir.join("stale.sock");
    drop(std::os::unix::net::UnixListener::bind(&stale).unwrap());
    prepare_socket_path(&stale).unwrap();
    assert!(!stale.exists());

    prepare_socket_path(&dir.join("absent.sock")).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn socket_is_private_to_the_user() {
    use std::os::unix::fs::PermissionsExt;
    let Some(srv) = start_server("mode").await else {
        return;
    };
    let mode = std::fs::metadata(&srv.path).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
}

#[test]
fn menu_rows_carry_state_and_plain_rows_keep_their_old_shape() {
    use super::server::to_menu_item;
    let node = |id, label: &str| libtrayd::MenuNode {
        id,
        label: label.into(),
        enabled: true,
        visible: true,
        icon_name: String::new(),
        is_submenu: false,
        is_separator: false,
        toggle_state: None,
        children: Vec::new(),
    };
    let json = |n| serde_json::to_string(&to_menu_item(n)).unwrap();

    assert_eq!(
        json(node(1, "Open")),
        r#"{"item_id":1,"label":"Open","is_submenu":false}"#
    );
    let mut separator = node(2, "");
    separator.is_separator = true;
    assert_eq!(
        json(separator),
        r#"{"item_id":2,"label":"","is_submenu":false,"separator":true}"#
    );
    let mut disabled_toggle = node(3, "Mute");
    disabled_toggle.enabled = false;
    disabled_toggle.toggle_state = Some(true);
    assert_eq!(
        json(disabled_toggle),
        r#"{"item_id":3,"label":"Mute","is_submenu":false,"enabled":false,"checked":true}"#
    );
}

#[tokio::test]
async fn a_subscriber_that_disconnects_is_dropped_without_waiting_for_an_event() {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

    let Some(srv) = start_server("unsub").await else {
        return;
    };
    let wait_count = |n: usize| {
        let host = srv.host.clone();
        async move {
            for _ in 0..3000 {
                if host.subscriber_count() == n {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            panic!(
                "subscriber count stayed {} (want {n})",
                host.subscriber_count()
            );
        }
    };

    let stream = UnixStream::connect(&srv.path).await.unwrap();
    let (read, mut write) = stream.into_split();
    write
        .write_all(b"{\"v\":1,\"cmd\":\"subscribe\"}\n")
        .await
        .unwrap();
    let mut line = String::new();
    BufReader::new(read).read_line(&mut line).await.unwrap();
    assert!(line.contains("\"type\":\"event\""), "{line}");
    wait_count(1).await;

    // The tray stays quiet: nothing would make a write fail.
    drop(write);
    wait_count(0).await;
}
