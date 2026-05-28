use super::*;

#[test]
fn item_id_display() {
    let id = ItemId("org.example.App".to_owned());
    assert_eq!(id.to_string(), "org.example.App");
}

#[test]
fn item_id_from_str() {
    let id = ItemId::from("org.example.App");
    assert_eq!(id.0, "org.example.App");
}

#[test]
fn item_id_from_string() {
    let id = ItemId::from("test".to_owned());
    assert_eq!(id.0, "test");
}

#[test]
fn tray_status_from_dbus_passive() {
    assert_eq!(TrayStatus::from_dbus("Passive"), TrayStatus::Passive);
    assert_eq!(TrayStatus::from_dbus("anything_else"), TrayStatus::Passive);
    assert_eq!(TrayStatus::from_dbus(""), TrayStatus::Passive);
}

#[test]
fn tray_status_from_dbus_active() {
    assert_eq!(TrayStatus::from_dbus("Active"), TrayStatus::Active);
}

#[test]
fn tray_status_from_dbus_needs_attention() {
    assert_eq!(
        TrayStatus::from_dbus("NeedsAttention"),
        TrayStatus::NeedsAttention
    );
}

#[test]
fn tray_status_round_trip() {
    for status in [
        TrayStatus::Passive,
        TrayStatus::Active,
        TrayStatus::NeedsAttention,
    ] {
        assert_eq!(TrayStatus::from_dbus(status.as_str()), status);
    }
}

#[test]
fn tray_status_display() {
    assert_eq!(TrayStatus::Active.to_string(), "Active");
    assert_eq!(TrayStatus::NeedsAttention.to_string(), "NeedsAttention");
}

#[test]
fn icon_data_default_is_empty() {
    assert!(IconData::default().is_empty());
}

#[test]
fn icon_data_not_empty_with_name() {
    let icon = IconData {
        name: "network-wireless".to_owned(),
        pixmaps: vec![],
    };
    assert!(!icon.is_empty());
}

#[test]
fn icon_data_not_empty_with_pixmap() {
    let icon = IconData {
        name: String::new(),
        pixmaps: vec![IconPixmap {
            width: 22,
            height: 22,
            data: vec![0u8; 22 * 22 * 4],
        }],
    };
    assert!(!icon.is_empty());
}

#[test]
fn icon_data_handle_from_name() {
    let icon = IconData {
        name: "network-wireless".to_owned(),
        pixmaps: vec![],
    };
    assert_eq!(icon.as_handle(), Some("network-wireless".to_owned()));
}

#[test]
fn icon_data_handle_none_when_empty() {
    assert_eq!(IconData::default().as_handle(), None);
}

#[test]
fn icon_data_handle_none_when_only_pixmaps() {
    let icon = IconData {
        name: String::new(),
        pixmaps: vec![IconPixmap {
            width: 16,
            height: 16,
            data: vec![],
        }],
    };
    assert_eq!(icon.as_handle(), None);
}
