use super::{Cmd, MinimalTrayItem, Payload, Request, Response, TrayEvent};

#[test]
fn subscribe_request_serializes() {
    let req = Request {
        v: 1,
        cmd: Cmd::Subscribe,
    };
    let json = serde_json::to_string(&req).unwrap();
    assert!(json.contains("\"cmd\":\"subscribe\""), "json={json}");
    assert!(json.contains("\"v\":1"), "json={json}");
}

#[test]
fn get_menu_request_serializes() {
    let req = Request {
        v: 1,
        cmd: Cmd::GetMenu {
            app_id: "org.example.App".to_owned(),
            submenu_id: Some(42),
        },
    };
    let json = serde_json::to_string(&req).unwrap();
    assert!(json.contains("\"cmd\":\"get_menu\""), "json={json}");
    assert!(
        json.contains("\"app_id\":\"org.example.App\""),
        "json={json}"
    );
    assert!(json.contains("\"submenu_id\":42"), "json={json}");
}

#[test]
fn activate_request_serializes() {
    let req = Request {
        v: 1,
        cmd: Cmd::Activate {
            app_id: "org.nm".to_owned(),
            item_id: 7,
        },
    };
    let json = serde_json::to_string(&req).unwrap();
    assert!(json.contains("\"cmd\":\"activate\""), "json={json}");
    assert!(json.contains("\"item_id\":7"), "json={json}");
}

#[test]
fn items_response_deserializes() {
    let json = r#"{"v":1,"type":"items","items":[{"app_id":"org.nm","status":"Active"}]}"#;
    let resp: Response = serde_json::from_str(json).unwrap();
    match resp {
        Response::Ok(ok) => match ok.payload {
            Payload::Items { items } => {
                assert_eq!(items.len(), 1);
                assert_eq!(items[0].app_id, "org.nm");
                assert_eq!(items[0].status, "Active");
            }
            other => panic!("unexpected payload: {other:?}"),
        },
        Response::Err(e) => panic!("expected Ok, got error: {}", e.error.message),
    }
}

#[test]
fn event_response_deserializes() {
    let json = r#"{"v":1,"type":"event","event":{"kind":"update","items":[{"app_id":"nm","status":"Active"}]}}"#;
    let resp: Response = serde_json::from_str(json).unwrap();
    match resp {
        Response::Ok(ok) => match ok.payload {
            Payload::Event {
                event: TrayEvent::Update(items),
            } => {
                assert_eq!(items.len(), 1);
                assert_eq!(items[0].app_id, "nm");
            }
            other => panic!("unexpected payload: {other:?}"),
        },
        Response::Err(e) => panic!("expected Ok, got error: {}", e.error.message),
    }
}

#[test]
fn error_response_deserializes() {
    let json = r#"{"v":1,"error":{"code":"NOT_FOUND","message":"item not found"}}"#;
    let resp: Response = serde_json::from_str(json).unwrap();
    match resp {
        Response::Err(e) => {
            assert_eq!(e.error.code, "NOT_FOUND");
            assert_eq!(e.error.message, "item not found");
        }
        Response::Ok(_) => panic!("expected Err"),
    }
}

#[test]
fn minimal_tray_item_optional_fields_absent() {
    let json = r#"{"app_id":"org.nm","status":"Active"}"#;
    let item: MinimalTrayItem = serde_json::from_str(json).unwrap();
    assert_eq!(item.app_id, "org.nm");
    assert!(item.title.is_none());
    assert!(item.icon_handle.is_none());
    assert!(item.category.is_none());
    assert!(!item.item_is_menu, "item_is_menu must default to false");
    assert!(item.tooltip_title.is_none());
    assert!(item.tooltip_description.is_none());
}

#[test]
fn minimal_tray_item_with_title_and_icon() {
    let json =
        r#"{"app_id":"org.nm","title":"Network","status":"Active","icon_handle":"nm-active"}"#;
    let item: MinimalTrayItem = serde_json::from_str(json).unwrap();
    assert_eq!(item.title.as_deref(), Some("Network"));
    assert_eq!(item.icon_handle.as_deref(), Some("nm-active"));
}

#[test]
fn minimal_tray_item_new_fields_deserialize() {
    let json = r#"{"app_id":"org.sysmenu","status":"Active","category":"SystemServices","item_is_menu":true,"tooltip_title":"System Menu","tooltip_description":"Click for options"}"#;
    let item: MinimalTrayItem = serde_json::from_str(json).unwrap();
    assert_eq!(item.category.as_deref(), Some("SystemServices"));
    assert!(item.item_is_menu);
    assert_eq!(item.tooltip_title.as_deref(), Some("System Menu"));
    assert_eq!(
        item.tooltip_description.as_deref(),
        Some("Click for options")
    );
}
