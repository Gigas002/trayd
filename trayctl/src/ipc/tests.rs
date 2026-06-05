use super::{Cmd, Payload, Request, Response, TrayEvent};

#[test]
fn get_menu_request_serializes() {
    let req = Request {
        v: 1,
        cmd: Cmd::GetMenu {
            app_id: "org.example.App".to_owned(),
            submenu_id: None,
        },
    };
    let json = serde_json::to_string(&req).unwrap();
    assert!(json.contains(r#""v":1"#), "got: {json}");
    assert!(json.contains(r#""cmd":"get_menu""#), "got: {json}");
    assert!(
        json.contains(r#""app_id":"org.example.App""#),
        "got: {json}"
    );
}

#[test]
fn get_menu_submenu_request_serializes() {
    let req = Request {
        v: 1,
        cmd: Cmd::GetMenu {
            app_id: "org.example.App".to_owned(),
            submenu_id: Some(3),
        },
    };
    let json = serde_json::to_string(&req).unwrap();
    assert!(json.contains(r#""submenu_id":3"#), "got: {json}");
}

#[test]
fn activate_request_serializes() {
    let req = Request {
        v: 1,
        cmd: Cmd::Activate {
            app_id: "org.example.App".to_owned(),
            item_id: 5,
        },
    };
    let json = serde_json::to_string(&req).unwrap();
    assert!(json.contains(r#""cmd":"activate""#), "got: {json}");
    assert!(json.contains(r#""item_id":5"#), "got: {json}");
}

#[test]
fn get_items_request_serializes() {
    let req = Request {
        v: 1,
        cmd: Cmd::GetItems,
    };
    let json = serde_json::to_string(&req).unwrap();
    assert!(json.contains(r#""cmd":"get_items""#), "got: {json}");
}

#[test]
fn menu_response_deserializes() {
    let json = r#"{"v":1,"type":"menu","app_id":"org.example.App","items":[{"item_id":1,"label":"Quit","is_submenu":false}]}"#;
    let resp: Response = serde_json::from_str(json).unwrap();
    match resp {
        Response::Ok(ok) => match ok.payload {
            Payload::Menu { items, .. } => {
                assert_eq!(items.len(), 1);
                assert_eq!(items[0].label, "Quit");
            }
            other => panic!("expected Menu payload, got {other:?}"),
        },
        Response::Err(e) => panic!("expected Ok, got error: {:?}", e.error),
    }
}

#[test]
fn ack_response_deserializes() {
    let json = r#"{"v":1,"type":"ack"}"#;
    let resp: Response = serde_json::from_str(json).unwrap();
    assert!(matches!(resp, Response::Ok(ok) if matches!(ok.payload, Payload::Ack)));
}

#[test]
fn error_response_deserializes() {
    let json = r#"{"v":1,"error":{"code":"NOT_FOUND","message":"not found"}}"#;
    let resp: Response = serde_json::from_str(json).unwrap();
    match resp {
        Response::Err(e) => assert_eq!(e.error.code, "NOT_FOUND"),
        Response::Ok(_) => panic!("expected Err, got Ok"),
    }
}

#[test]
fn items_response_deserializes() {
    let json = r#"{"v":1,"type":"items","items":[{"app_id":"org.example.App","title":"Example App","status":"active","icon_handle":"example-app"}]}"#;
    let resp: Response = serde_json::from_str(json).unwrap();
    match resp {
        Response::Ok(ok) => match ok.payload {
            Payload::Items { items } => assert_eq!(items.len(), 1),
            other => panic!("expected Items payload, got {other:?}"),
        },
        Response::Err(e) => panic!("expected Ok, got error: {:?}", e.error),
    }
}

#[test]
fn subscribe_request_serializes() {
    let req = Request {
        v: 1,
        cmd: Cmd::Subscribe,
    };
    let json = serde_json::to_string(&req).unwrap();
    assert!(json.contains(r#""cmd":"subscribe""#), "got: {json}");
}

#[test]
fn event_response_deserializes() {
    let json = r#"{"v":1,"type":"event","event":{"kind":"update","items":[{"app_id":"org.example.App","status":"Active"}]}}"#;
    let resp: Response = serde_json::from_str(json).unwrap();
    match resp {
        Response::Ok(ok) => match ok.payload {
            Payload::Event {
                event: TrayEvent::Update(items),
            } => {
                assert_eq!(items.len(), 1);
                assert_eq!(items[0].app_id, "org.example.App");
            }
            other => panic!("expected Event payload, got {other:?}"),
        },
        Response::Err(e) => panic!("expected Ok, got error: {:?}", e.error),
    }
}
