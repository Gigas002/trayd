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
