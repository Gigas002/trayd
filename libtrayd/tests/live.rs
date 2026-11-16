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
