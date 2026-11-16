//! End-to-end tests of the tray host against fake apps on a private D-Bus.

mod common;

use common::{FakeApp, FakeState, wait_for};
use libtrayd::ItemId;

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
