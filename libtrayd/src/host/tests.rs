use super::*;
use crate::model::{IconData, ItemId, ToolTip, TrayItem, TrayStatus};

// ─── resolve_icon_name ────────────────────────────────────────────────────────────────

#[test]
fn resolve_icon_name_explicit_icon_name_wins() {
    // When `IconName` is set it should always be used, regardless of the other sources.
    assert_eq!(
        resolve_icon_name(
            "nm-device-wired".to_owned(),
            Some("org.example.App"),
            "ExampleApp".to_owned(),
        ),
        "nm-device-wired",
    );
}

#[test]
fn resolve_icon_name_falls_back_to_well_known() {
    // When `IconName` is empty, the well-known bus name is used (Telegram case).
    assert_eq!(
        resolve_icon_name(
            String::new(),
            Some("org.telegram.desktop"),
            "TelegramDesktop".to_owned(),
        ),
        "org.telegram.desktop",
    );
}

#[test]
fn resolve_icon_name_falls_back_to_sni_id() {
    // When both `IconName` and well-known name are absent, the SNI `Id` is used.
    assert_eq!(
        resolve_icon_name(String::new(), None, "SomeApp".to_owned(),),
        "SomeApp",
    );
}

#[test]
fn resolve_icon_name_empty_well_known_skipped() {
    // An empty well-known name string should not be returned; fall through to SNI `Id`.
    assert_eq!(
        resolve_icon_name(String::new(), Some(""), "FallbackId".to_owned()),
        "FallbackId",
    );
}

#[test]
fn resolve_icon_name_all_empty_returns_empty() {
    assert_eq!(resolve_icon_name(String::new(), None, String::new()), "",);
}

/// Build a dummy `TrayItem` for use in unit tests (no D-Bus required).
fn dummy_item(id: &str) -> TrayItem {
    TrayItem {
        id: ItemId(id.to_owned()),
        bus_name: format!("org.example.{id}"),
        object_path: "/StatusNotifierItem".to_owned(),
        title: id.to_owned(),
        status: TrayStatus::Active,
        category: String::new(),
        item_is_menu: false,
        tool_tip: ToolTip::default(),
        overlay_icon: IconData::default(),
        icon: IconData::default(),
        attention_icon: IconData::default(),
        menu_path: String::new(),
    }
}

#[test]
fn host_state_insert_and_retrieve() {
    let mut state = HostState::new();
    let item = dummy_item("App");
    state.items.insert(item.id.clone(), item.clone());

    let retrieved = state.items.get(&item.id).unwrap();
    assert_eq!(retrieved.title, "App");
}

#[test]
fn host_state_remove_item() {
    let mut state = HostState::new();
    let item = dummy_item("App");
    let id = item.id.clone();
    state.items.insert(id.clone(), item);
    assert!(state.items.contains_key(&id));

    state.items.remove(&id);
    assert!(!state.items.contains_key(&id));
}

#[test]
fn host_state_multiple_items() {
    let mut state = HostState::new();
    for name in ["Alpha", "Beta", "Gamma"] {
        let item = dummy_item(name);
        state.items.insert(item.id.clone(), item);
    }
    assert_eq!(state.items.len(), 3);
}

// ─── menu_nodes_from_av / prop helpers ───────────────────────────────────────

#[test]
fn menu_nodes_from_av_empty_slice() {
    let nodes = menu_nodes_from_av(vec![]);
    assert!(nodes.is_empty());
}

#[test]
fn get_str_prop_missing_key_returns_empty() {
    let props: std::collections::HashMap<String, zbus::OwnedVariant> = Default::default();
    assert_eq!(get_str_prop(&props, "label"), "");
}

#[test]
fn get_str_prop_with_string_value() {
    use zbus::{OwnedVariant, Variant};
    let mut props: std::collections::HashMap<String, OwnedVariant> = Default::default();
    let ov: OwnedVariant = Variant::from("Hello")
        .try_into()
        .expect("OwnedVariant from str");
    props.insert("label".to_owned(), ov);
    assert_eq!(get_str_prop(&props, "label"), "Hello");
}

#[test]
fn get_bool_prop_missing_key_returns_none() {
    let props: std::collections::HashMap<String, zbus::OwnedVariant> = Default::default();
    assert_eq!(get_bool_prop(&props, "enabled"), None);
}

#[test]
fn get_bool_prop_with_true() {
    use zbus::{OwnedVariant, Variant};
    let mut props: std::collections::HashMap<String, OwnedVariant> = Default::default();
    let ov: OwnedVariant = Variant::from(true)
        .try_into()
        .expect("OwnedVariant from bool");
    props.insert("enabled".to_owned(), ov);
    assert_eq!(get_bool_prop(&props, "enabled"), Some(true));
}

#[test]
fn tray_item_attention_icon_defaults_empty() {
    let item = dummy_item("App");
    assert!(
        item.attention_icon.is_empty(),
        "attention_icon defaults to empty IconData"
    );
}

#[test]
fn tray_item_item_is_menu_defaults_false() {
    let item = dummy_item("App");
    assert!(!item.item_is_menu, "item_is_menu defaults to false");
}

#[test]
fn tray_item_tool_tip_defaults_empty() {
    let item = dummy_item("App");
    assert!(
        item.tool_tip.title.is_empty(),
        "tool_tip.title defaults to empty"
    );
    assert!(
        item.tool_tip.description.is_empty(),
        "tool_tip.description defaults to empty"
    );
    assert!(
        item.tool_tip.icon_name.is_empty(),
        "tool_tip.icon_name defaults to empty"
    );
}

#[tokio::test]
async fn replacing_an_item_task_aborts_the_old_one() {
    let mut state = HostState::new();
    let id = ItemId::from("org.example.App");
    let first = tokio::spawn(std::future::pending::<()>());
    let first_abort = first.abort_handle();
    state.set_item_task(id.clone(), first);

    state.set_item_task(id.clone(), tokio::spawn(std::future::pending::<()>()));
    tokio::task::yield_now().await;
    assert!(first_abort.is_finished(), "replaced task must be aborted");
    assert_eq!(state.item_tasks.len(), 1);
}

#[test]
fn items_keep_registration_order() {
    let ids = |state: &HostState| -> Vec<String> {
        state
            .sorted_items()
            .iter()
            .map(|i| i.id.to_string())
            .collect()
    };
    let mut state = HostState::new();
    for id in ["c", "a", "b"] {
        state.insert_item(dummy_item(id));
    }
    assert_eq!(ids(&state), ["c", "a", "b"]);

    // Updating an item keeps its place; a returning item goes last.
    state.insert_item(dummy_item("c"));
    state.remove_item(&ItemId::from("a"));
    state.insert_item(dummy_item("a"));
    assert_eq!(ids(&state), ["c", "b", "a"]);
}

#[test]
fn pixmaps_from_dbus_keeps_only_consistent_pixmaps() {
    let kept = pixmaps_from_dbus(vec![
        (1, 1, vec![0; 4]),
        (i32::MAX, i32::MAX, vec![0; 4]),
        (-1, -1, vec![0; 4]),
        (2, 1, vec![0; 4]),
        (0, 5, Vec::new()),
        (2, 3, vec![0; 24]),
    ]);
    let dims: Vec<_> = kept.iter().map(|p| (p.width, p.height)).collect();
    assert_eq!(dims, [(1, 1), (2, 3)]);
}
