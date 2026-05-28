use super::*;

#[test]
fn menu_node_leaf() {
    let node = MenuNode {
        id: 42,
        label: Some("Enable".to_owned()),
        enabled: true,
        visible: true,
        is_separator: false,
        children_display: None,
    };
    assert_eq!(node.id, 42);
    assert!(node.children_display.is_none());
}

#[test]
fn menu_node_submenu() {
    let node = MenuNode {
        id: 7,
        label: Some("More".to_owned()),
        enabled: true,
        visible: true,
        is_separator: false,
        children_display: Some("submenu".to_owned()),
    };
    assert_eq!(node.children_display.as_deref(), Some("submenu"));
}

#[test]
fn menu_node_separator() {
    let node = MenuNode {
        id: 0,
        label: None,
        enabled: false,
        visible: true,
        is_separator: true,
        children_display: None,
    };
    assert!(node.is_separator);
    assert!(node.label.is_none());
}

#[test]
fn host_event_menu_changed_carries_id() {
    let id = ItemId("org.kde.plasma.nm/StatusNotifierItem".to_owned());
    let ev = HostEvent::MenuChanged(id.clone());
    assert!(matches!(ev, HostEvent::MenuChanged(ref i) if *i == id));
}

#[test]
fn item_id_display() {
    let id = ItemId("org.kde.plasma.nm/StatusNotifierItem".to_owned());
    assert_eq!(id.to_string(), "org.kde.plasma.nm/StatusNotifierItem");
}

#[test]
fn item_status_from_sni_str() {
    assert_eq!(ItemStatus::from_sni_str("Passive"), ItemStatus::Passive);
    assert_eq!(ItemStatus::from_sni_str("Active"), ItemStatus::Active);
    assert_eq!(
        ItemStatus::from_sni_str("NeedsAttention"),
        ItemStatus::NeedsAttention
    );
    assert_eq!(ItemStatus::from_sni_str("unknown"), ItemStatus::Active);
}

#[test]
fn item_status_default_is_active() {
    assert_eq!(ItemStatus::default(), ItemStatus::Active);
}

#[test]
fn host_event_item_added_carries_item() {
    let item = Item {
        id: ItemId("test/StatusNotifierItem".to_owned()),
        title: Some("Test".to_owned()),
        status: ItemStatus::Active,
        has_attention_icon: false,
        tooltip: None,
        category: None,
        icon_name: None,
    };
    let ev = HostEvent::ItemAdded(item.clone());
    assert!(matches!(ev, HostEvent::ItemAdded(_)));
}

#[test]
fn host_event_item_removed_carries_id() {
    let id = ItemId("test/StatusNotifierItem".to_owned());
    let ev = HostEvent::ItemRemoved(id.clone());
    assert!(matches!(ev, HostEvent::ItemRemoved(ref i) if *i == id));
}
