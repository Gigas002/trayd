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
