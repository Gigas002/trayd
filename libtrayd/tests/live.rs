//! End-to-end tests of the tray host against fake apps on a private D-Bus.

mod common;

use common::{FakeApp, FakeState, Prop, Row, wait_for};
use libtrayd::{ItemId, ScrollOrientation, TraydError};

#[tokio::test]
async fn registered_item_is_listed_with_its_properties() {
    let bus = test_bus!();
    let host = bus.host().await;
    let app = FakeApp::new(&bus, FakeState::default()).await;
    app.register_as("org.example.Fake").await;

    let items = wait_for(&host, "item listed", |i| i.len() == 1).await;
    let item = &items[0];
    assert_eq!(item.id, ItemId::from("org.example.Fake".to_owned()));
    assert_eq!(item.title, "Fake App");
    assert_eq!(item.icon.name, "fake-icon");
    assert_eq!(item.menu_path, "/Menu");
}

#[tokio::test]
async fn path_only_registration_uses_the_sender() {
    let bus = test_bus!();
    let host = bus.host().await;
    let app = FakeApp::new(&bus, FakeState::default()).await;
    app.register("/StatusNotifierItem").await.unwrap();

    let expected = format!("{}/StatusNotifierItem", app.unique_name());
    wait_for(&host, "item listed under sender", |i| {
        i.iter().any(|it| it.id.to_string() == expected)
    })
    .await;
}

#[tokio::test]
async fn item_is_removed_when_the_app_leaves_the_bus() {
    let bus = test_bus!();
    let host = bus.host().await;
    let app = FakeApp::new(&bus, FakeState::default()).await;
    app.register_as("org.example.Fake").await;
    wait_for(&host, "item listed", |i| i.len() == 1).await;

    drop(app);
    wait_for(&host, "item removed", |i| i.is_empty()).await;
}

#[tokio::test]
async fn primary_activation_reaches_the_app() {
    let bus = test_bus!();
    let host = bus.host().await;
    let app = FakeApp::new(&bus, FakeState::default()).await;
    app.register_as("org.example.Fake").await;
    let id = wait_for(&host, "item listed", |i| i.len() == 1).await[0]
        .id
        .clone();

    host.activate(&id, 0).await.unwrap();
    assert_eq!(app.activations(), vec![(0, 0)]);
}

#[tokio::test]
async fn menu_is_listed_and_clicks_reach_the_app() {
    let bus = test_bus!();
    let host = bus.host().await;
    let app = FakeApp::new(&bus, FakeState::default()).await;
    app.register_as("org.example.Fake").await;
    let id = wait_for(&host, "item listed", |i| i.len() == 1).await[0]
        .id
        .clone();

    let menu = host.get_menu(&id, None).await.unwrap();
    let labels: Vec<_> = menu.iter().map(|n| n.label.as_str()).collect();
    assert_eq!(labels, ["Open", "Quit"]);

    host.activate(&id, 2).await.unwrap();
    assert_eq!(app.events(), vec![(2, "clicked".to_owned())]);
}

#[tokio::test]
async fn unknown_item_is_not_found() {
    let bus = test_bus!();
    let host = bus.host().await;
    let id = ItemId::from("org.example.Nobody".to_owned());
    assert!(matches!(
        host.activate(&id, 0).await,
        Err(libtrayd::TraydError::NotFound(_))
    ));
    assert!(matches!(
        host.get_menu(&id, None).await,
        Err(libtrayd::TraydError::NotFound(_))
    ));
}

#[tokio::test]
async fn second_host_does_not_take_over_the_watcher() {
    let bus = test_bus!();
    let first = bus.host().await;

    let second = libtrayd::TrayHost::start_on(&bus.address).await;
    assert!(
        matches!(&second, Err(e) if e.is_watcher_taken()),
        "second host must fail, got {:?}",
        second.as_ref().map(|_| "a running host")
    );
    drop(second);

    // The first host still owns the name: a new app reaches it.
    let app = FakeApp::new(&bus, FakeState::default()).await;
    app.register_as("org.example.Fake").await;
    wait_for(&first, "first host still gets registrations", |i| {
        i.len() == 1
    })
    .await;
}

#[tokio::test]
async fn every_change_signal_refreshes_the_item() {
    let bus = test_bus!();
    let host = bus.host().await;
    let app = FakeApp::new(&bus, FakeState::default()).await;
    app.register_as("org.example.Fake").await;
    wait_for(&host, "item listed", |i| i.len() == 1).await;

    // Several rounds: a property cache would serve the first value forever.
    for round in 1..=3 {
        let title = format!("Title {round}");
        app.set_title(&title).await;
        wait_for(&host, &title, |i| i[0].title == title).await;

        let icon = format!("icon-{round}");
        app.set_icon_name(&icon).await;
        wait_for(&host, &icon, |i| i[0].icon.name == icon).await;
    }
}

#[tokio::test]
async fn registrations_without_a_live_owner_are_rejected() {
    let bus = test_bus!();
    let host = bus.host().await;
    let app = FakeApp::new(&bus, FakeState::default()).await;

    for bogus in [
        "org.example.Nobody",
        "",
        "not a bus name",
        "org.example.X/bad path",
    ] {
        assert!(
            app.register(bogus).await.is_err(),
            "{bogus:?} must be rejected"
        );
    }
    // A real registration afterwards still works and is the only entry.
    app.register_as("org.example.Fake").await;
    wait_for(&host, "real item listed", |i| i.len() == 1).await;
    assert_eq!(common::registered_items(&bus).await, ["org.example.Fake"]);
}

#[tokio::test]
async fn removing_an_app_keeps_apps_whose_name_extends_it() {
    let bus = test_bus!();
    let host = bus.host().await;
    let short = FakeApp::new(&bus, FakeState::default()).await;
    short.register_as("org.example.P").await;
    let long = FakeApp::new(&bus, FakeState::default()).await;
    long.register_as("org.example.P2").await;
    wait_for(&host, "both listed", |i| i.len() == 2).await;

    drop(short);
    wait_for(&host, "only P2 left", |i| {
        i.len() == 1 && i[0].id.to_string() == "org.example.P2"
    })
    .await;
    assert_eq!(common::registered_items(&bus).await, ["org.example.P2"]);
}

#[tokio::test]
async fn a_hung_app_does_not_block_other_registrations() {
    let bus = test_bus!();
    let host = bus.host().await;
    let hung = FakeApp::new(
        &bus,
        FakeState {
            hang: true,
            ..FakeState::default()
        },
    )
    .await;
    hung.register_as("org.example.Hung").await;
    common::wait_app(&hung, "host started reading", |s| s.property_reads > 0).await;

    let app = FakeApp::new(&bus, FakeState::default()).await;
    app.register_as("org.example.Fine").await;
    wait_for(&host, "fine app listed despite the hung one", |i| {
        i.iter().any(|it| it.id.to_string() == "org.example.Fine")
    })
    .await;
}

#[tokio::test]
async fn an_app_leaving_during_registration_leaves_no_ghost() {
    let bus = test_bus!();
    let host = bus.host().await;
    let hung = FakeApp::new(
        &bus,
        FakeState {
            hang: true,
            ..FakeState::default()
        },
    )
    .await;
    hung.register_as("org.example.Gone").await;
    common::wait_app(&hung, "host started reading", |s| s.property_reads > 0).await;
    hung.leave().await;

    let app = FakeApp::new(&bus, FakeState::default()).await;
    app.register_as("org.example.Fine").await;
    wait_for(&host, "fine app listed", |i| !i.is_empty()).await;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let ids: Vec<_> = host
        .items()
        .await
        .iter()
        .map(|i| i.id.to_string())
        .collect();
    assert_eq!(ids, ["org.example.Fine"]);
    assert_eq!(common::registered_items(&bus).await, ["org.example.Fine"]);
}

#[tokio::test]
async fn items_are_listed_in_registration_order() {
    let bus = test_bus!();
    let host = bus.host().await;
    let mut apps = Vec::new();
    let names: Vec<String> = (0..10).map(|n| format!("org.example.App{n}")).collect();
    for (n, name) in names.iter().enumerate() {
        let app = FakeApp::new(&bus, FakeState::default()).await;
        app.register_as(name).await;
        wait_for(&host, name, |i| i.len() == n + 1).await;
        apps.push(app);
    }
    let ids: Vec<String> = host
        .items()
        .await
        .iter()
        .map(|i| i.id.to_string())
        .collect();
    assert_eq!(ids, names);
}

#[tokio::test]
async fn malformed_pixmaps_are_never_served() {
    let bus = test_bus!();
    let host = bus.host().await;
    let good = (2, 2, vec![0xffu8; 2 * 2 * 4]);
    let app = FakeApp::new(
        &bus,
        FakeState {
            pixmaps: vec![
                (i32::MIN, -5, vec![0; 4]),         // hostile dimensions
                (22, 22, vec![0; 10]),              // too little data
                (0, 0, Vec::new()),                 // empty
                (64, 64, vec![0; 64 * 64 * 4 + 1]), // too much data
                good.clone(),
            ],
            ..FakeState::default()
        },
    )
    .await;
    app.register_as("org.example.Fake").await;
    let id = wait_for(&host, "item listed", |i| i.len() == 1).await[0]
        .id
        .clone();

    for size in [1, 22, 64, u16::MAX] {
        let px = host.get_pixmap(&id, size).await.unwrap();
        assert_eq!(
            (px.width, px.height, px.data),
            (2, 2, good.2.clone()),
            "size {size}"
        );
    }
}

#[tokio::test]
async fn a_burst_of_change_signals_neither_freezes_nor_floods_the_host() {
    let bus = test_bus!();
    let host = bus.host().await;
    let app = FakeApp::new(&bus, FakeState::default()).await;
    // Path-only registration: the case where the host also looks for a
    // well-known name of the same process.
    app.register("/StatusNotifierItem").await.unwrap();
    wait_for(&host, "item listed", |i| i.len() == 1).await;
    let reads_before = app.property_reads();

    app.flood_new_icon("final-icon", 500).await;

    // The host still serves other apps during and after the burst...
    let other = FakeApp::new(&bus, FakeState::default()).await;
    other.register_as("org.example.Other").await;
    wait_for(&host, "other app listed after the burst", |i| i.len() == 2).await;
    // ...ends up with the final icon...
    wait_for(&host, "final icon", |i| {
        i.iter().any(|it| it.icon.name == "final-icon")
    })
    .await;
    // ...and coalesced the burst instead of re-reading per signal.
    let reads = app.property_reads() - reads_before;
    assert!(reads < 100, "500 signals caused {reads} property reads");
}

#[tokio::test]
async fn menus_report_what_a_user_sees() {
    let bus = test_bus!();
    let host = bus.host().await;
    let app = FakeApp::new(
        &bus,
        FakeState {
            menu: vec![
                Row::label(1, "_Open window"),
                Row {
                    id: 2,
                    props: vec![("type", Prop::Str("separator"))],
                    children: None,
                },
                Row::label(3, "Disabled").with("enabled", Prop::Bool(false)),
                Row::label(4, "Do not disturb")
                    .with("toggle-type", Prop::Str("checkmark"))
                    .with("toggle-state", Prop::Int(1)),
                Row::label(5, "Save __as").with("toggle-type", Prop::Str("radio")),
                Row::label(6, "More").submenu(vec![Row::label(61, "Inner")]),
            ],
            lazy_submenus: vec![6],
            ..FakeState::default()
        },
    )
    .await;
    app.register_as("org.example.Fake").await;
    let id = wait_for(&host, "item listed", |i| i.len() == 1).await[0]
        .id
        .clone();

    let menu = host.get_menu(&id, None).await.unwrap();
    let rows: Vec<_> = menu
        .iter()
        .map(|n| {
            (
                n.id,
                n.label.as_str(),
                n.enabled,
                n.is_separator,
                n.toggle_state,
            )
        })
        .collect();
    assert_eq!(
        rows,
        [
            (1, "Open window", true, false, None),
            (2, "", true, true, None),
            (3, "Disabled", false, false, None),
            (4, "Do not disturb", true, false, Some(true)),
            (5, "Save _as", true, false, Some(false)),
            (6, "More", true, false, None),
        ]
    );

    // "More" fills its children only once it is about to be shown.
    let sub = host.get_menu(&id, Some(6)).await.unwrap();
    let labels: Vec<_> = sub.iter().map(|n| n.label.as_str()).collect();
    assert_eq!(labels, ["Inner"]);
    assert!(app.state.lock().unwrap().about_to_show.contains(&6));
}

#[tokio::test]
async fn watcher_introspection_names_every_argument() {
    let bus = test_bus!();
    let _host = bus.host().await;
    let conn = bus.connect().await;
    let reply = conn
        .call_method(
            Some("org.kde.StatusNotifierWatcher"),
            "/StatusNotifierWatcher",
            Some("org.freedesktop.DBus.Introspectable"),
            "Introspect",
            &(),
        )
        .await
        .unwrap();
    let xml: String = reply.body().deserialize().unwrap();
    let host_method = xml
        .split("<method ")
        .find(|m| m.starts_with("name=\"RegisterStatusNotifierHost\""))
        .expect("RegisterStatusNotifierHost is introspectable");
    assert!(host_method.contains("name=\"service\""), "{host_method}");
    assert!(!xml.contains("name=\"_"), "{xml}");
}

#[tokio::test]
async fn secondary_activation_scroll_and_context_menu_reach_the_app() {
    let bus = test_bus!();
    let host = bus.host().await;
    let app = FakeApp::new(&bus, FakeState::default()).await;
    app.register_as("org.example.Fake").await;
    let id = wait_for(&host, "item listed", |i| i.len() == 1).await[0]
        .id
        .clone();

    host.secondary_activate(&id, 10, 20).await.unwrap();
    host.scroll(&id, -120, ScrollOrientation::Vertical)
        .await
        .unwrap();
    host.scroll(&id, 1, ScrollOrientation::Horizontal)
        .await
        .unwrap();
    host.context_menu(&id, 30, 40).await.unwrap();

    let s = app.state.lock().unwrap();
    assert_eq!(s.secondary_activations, vec![(10, 20)]);
    assert_eq!(
        s.scrolls,
        vec![(-120, "vertical".to_owned()), (1, "horizontal".to_owned())]
    );
    assert_eq!(s.context_menus, vec![(30, 40)]);
    assert!(s.activations.is_empty());
}

#[tokio::test]
async fn item_methods_on_an_unknown_item_are_not_found() {
    let bus = test_bus!();
    let host = bus.host().await;
    let id = ItemId::from("org.example.Missing");
    assert!(matches!(
        host.secondary_activate(&id, 0, 0).await,
        Err(TraydError::NotFound(_))
    ));
    assert!(matches!(
        host.scroll(&id, 1, ScrollOrientation::Vertical).await,
        Err(TraydError::NotFound(_))
    ));
    assert!(matches!(
        host.context_menu(&id, 0, 0).await,
        Err(TraydError::NotFound(_))
    ));
}

#[tokio::test]
async fn icon_theme_path_is_read_and_kept_up_to_date() {
    let bus = test_bus!();
    let host = bus.host().await;
    let state = FakeState {
        icon_theme_path: "/opt/fake/icons".into(),
        ..FakeState::default()
    };
    let app = FakeApp::new(&bus, state).await;
    app.register_as("org.example.Fake").await;
    wait_for(&host, "theme path read at registration", |i| {
        i.len() == 1 && i[0].icon_theme_path == "/opt/fake/icons"
    })
    .await;

    // libappindicator announces it with NewIconThemePath(path)...
    app.set_icon_theme_path("/opt/fake/icons2").await;
    wait_for(&host, "theme path from NewIconThemePath", |i| {
        i[0].icon_theme_path == "/opt/fake/icons2"
    })
    .await;

    // ...KDE apps with NewIcon.
    app.state.lock().unwrap().icon_theme_path = "/opt/fake/icons3".into();
    app.emit_new_icon().await;
    wait_for(&host, "theme path re-read on NewIcon", |i| {
        i[0].icon_theme_path == "/opt/fake/icons3"
    })
    .await;
}

#[tokio::test]
async fn apps_without_an_icon_theme_path_have_an_empty_one() {
    let bus = test_bus!();
    let host = bus.host().await;
    let app = FakeApp::new(&bus, FakeState::default()).await;
    app.register_as("org.example.Fake").await;
    let items = wait_for(&host, "item listed", |i| i.len() == 1).await;
    assert_eq!(items[0].icon_theme_path, "");
}

#[tokio::test]
async fn overlay_pixmaps_are_served_apart_from_the_icon() {
    let bus = test_bus!();
    let host = bus.host().await;
    let pixmap = |n: i32, byte: u8| (n, n, vec![byte; (n * n * 4) as usize]);
    let state = FakeState {
        pixmaps: vec![pixmap(22, 1)],
        overlay_pixmaps: vec![pixmap(8, 2), pixmap(16, 3)],
        ..FakeState::default()
    };
    let app = FakeApp::new(&bus, state).await;
    app.register_as("org.example.Fake").await;
    let id = wait_for(&host, "item listed", |i| i.len() == 1).await[0]
        .id
        .clone();

    let icon = host.get_pixmap(&id, 22).await.unwrap();
    assert_eq!((icon.width, icon.data[0]), (22, 1));
    let overlay = host.get_overlay_pixmap(&id, 14).await.unwrap();
    assert_eq!((overlay.width, overlay.data[0]), (16, 3));
}

#[tokio::test]
async fn an_item_without_an_overlay_pixmap_says_so() {
    let bus = test_bus!();
    let host = bus.host().await;
    let app = FakeApp::new(&bus, FakeState::default()).await;
    app.register_as("org.example.Fake").await;
    let id = wait_for(&host, "item listed", |i| i.len() == 1).await[0]
        .id
        .clone();
    assert!(matches!(
        host.get_overlay_pixmap(&id, 16).await,
        Err(TraydError::NotFound(_))
    ));
}

#[tokio::test]
async fn menu_rows_carry_their_icons_and_shortcuts() {
    let bus = test_bus!();
    let host = bus.host().await;
    let state = FakeState {
        menu: vec![
            Row::label(1, "Save")
                .with("icon-name", Prop::Str("document-save"))
                .with("icon-data", Prop::Bytes(vec![0x89, b'P', b'N', b'G']))
                .with("shortcut", Prop::Keys(vec![vec!["Control", "S"]])),
            Row::label(2, "Quit"),
        ],
        ..FakeState::default()
    };
    let app = FakeApp::new(&bus, state).await;
    app.register_as("org.example.Fake").await;
    let id = wait_for(&host, "item listed", |i| i.len() == 1).await[0]
        .id
        .clone();

    let menu = host.get_menu(&id, None).await.unwrap();
    assert_eq!(menu[0].icon_name, "document-save");
    assert_eq!(menu[0].icon_data, vec![0x89, b'P', b'N', b'G']);
    assert_eq!(menu[0].shortcuts, vec![vec!["Control", "S"]]);
    assert!(menu[1].icon_data.is_empty() && menu[1].shortcuts.is_empty());
}
