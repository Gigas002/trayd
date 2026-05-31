use super::*;
use crate::model::{IconData, ItemId, TrayItem, TrayStatus};

/// Build a dummy `TrayItem` for use in unit tests (no D-Bus required).
fn dummy_item(id: &str) -> TrayItem {
    TrayItem {
        id: ItemId(id.to_owned()),
        bus_name: format!("org.example.{id}"),
        object_path: "/StatusNotifierItem".to_owned(),
        title: id.to_owned(),
        status: TrayStatus::Active,
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

/// Live D-Bus tests — skipped in CI, run manually:
///   cargo test --package libtrayd -- host::tests::live --ignored
#[tokio::test]
#[ignore]
async fn live_tray_host_start() {
    let host = TrayHost::start().await.expect("TrayHost::start failed");
    let items = host.items().await;
    println!("registered items: {items:?}");
}

// ─── menu_nodes_from_av / prop helpers ───────────────────────────────────────

#[test]
fn menu_nodes_from_av_empty_slice() {
    let nodes = menu_nodes_from_av(vec![]);
    assert!(nodes.is_empty());
}

#[test]
fn get_str_prop_missing_key_returns_empty() {
    let props: std::collections::HashMap<String, zbus::zvariant::OwnedValue> = Default::default();
    assert_eq!(get_str_prop(&props, "label"), "");
}

#[test]
fn get_str_prop_with_string_value() {
    use zbus::zvariant::{OwnedValue, Value};
    let mut props: std::collections::HashMap<String, OwnedValue> = Default::default();
    let ov: OwnedValue = Value::from("Hello")
        .try_into()
        .expect("OwnedValue from str");
    props.insert("label".to_owned(), ov);
    assert_eq!(get_str_prop(&props, "label"), "Hello");
}

#[test]
fn get_bool_prop_missing_key_returns_none() {
    let props: std::collections::HashMap<String, zbus::zvariant::OwnedValue> = Default::default();
    assert_eq!(get_bool_prop(&props, "enabled"), None);
}

#[test]
fn get_bool_prop_with_true() {
    use zbus::zvariant::{OwnedValue, Value};
    let mut props: std::collections::HashMap<String, OwnedValue> = Default::default();
    let ov: OwnedValue = Value::from(true).try_into().expect("OwnedValue from bool");
    props.insert("enabled".to_owned(), ov);
    assert_eq!(get_bool_prop(&props, "enabled"), Some(true));
}

/// Live D-Bus test for get_menu — skipped in CI.
#[tokio::test]
#[ignore = "requires D-Bus session bus with a registered tray item that has a menu"]
async fn live_get_menu_top_level() {
    let host = TrayHost::start().await.expect("TrayHost::start failed");
    let items = host.items().await;
    let item = items
        .iter()
        .find(|i| !i.menu_path.is_empty() && i.menu_path != "/")
        .expect("no item with a menu found");
    println!("testing menu for: {}", item.id);
    let nodes = host
        .get_menu(&item.id, None)
        .await
        .expect("get_menu failed");
    println!("menu nodes: {nodes:#?}");
    assert!(!nodes.is_empty(), "expected at least one menu item");
}

// ─── Pixmap cache ────────────────────────────────────────────────────────────────

#[test]
fn host_state_pixmap_cache_empty_initially() {
    let state = HostState::new();
    assert!(state.pixmap_cache.is_empty());
}

#[test]
fn host_state_pixmap_cache_insert_and_retrieve() {
    let mut state = HostState::new();
    let id = ItemId("org.example.App".to_owned());
    let bytes = vec![0xAA, 0xBB, 0xCC, 0xDD];
    state.pixmap_cache.insert((id.clone(), 22), bytes.clone());

    let cached = state.pixmap_cache.get(&(id, 22)).cloned();
    assert_eq!(cached, Some(bytes));
}

#[test]
fn host_state_pixmap_cache_invalidation() {
    let mut state = HostState::new();
    let id = ItemId("org.example.App".to_owned());
    state.pixmap_cache.insert((id.clone(), 16), vec![1, 2]);
    state.pixmap_cache.insert((id.clone(), 22), vec![3, 4]);

    // A different item's entry must survive.
    let other = ItemId("org.other.App".to_owned());
    state.pixmap_cache.insert((other.clone(), 22), vec![5, 6]);

    state.invalidate_pixmap_cache(&id);

    assert!(
        !state.pixmap_cache.contains_key(&(id.clone(), 16)),
        "16px entry must be cleared"
    );
    assert!(
        !state.pixmap_cache.contains_key(&(id.clone(), 22)),
        "22px entry must be cleared"
    );
    assert!(
        state.pixmap_cache.contains_key(&(other.clone(), 22)),
        "other item entry must survive"
    );
}

#[test]
fn tray_item_attention_icon_defaults_empty() {
    let item = dummy_item("App");
    assert!(
        item.attention_icon.is_empty(),
        "attention_icon defaults to empty IconData"
    );
}
