use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crate::ipc::server::StubHandler;

/// Helper: unique socket path per test.
fn test_socket(label: &str) -> PathBuf {
    PathBuf::from(format!(
        "/tmp/trayd-daemon-test-{}-{}.sock",
        std::process::id(),
        label
    ))
}

#[tokio::test]
async fn daemon_run_with_socket_binds_and_shuts_down() {
    let path = test_socket("run_with_socket");
    let (tx, rx) = tokio::sync::watch::channel(false);

    let path_clone = path.clone();
    let task = tokio::spawn(async move { super::run_with_socket(path_clone, rx).await });

    // Allow the server to start.
    tokio::time::sleep(Duration::from_millis(20)).await;

    // Signal shutdown and wait (with timeout).
    tx.send(true).unwrap();
    tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .expect("daemon did not stop within 1 s")
        .unwrap() // JoinHandle
        .expect("daemon returned an error");

    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn daemon_socket_accepts_ping_while_running() {
    use crate::ipc::client::Client;
    use crate::ipc::protocol::{Method, ResultPayload};

    let path = test_socket("accepts_ping");
    let handler = Arc::new(StubHandler::new());
    let server = crate::ipc::server::Server::bind(&path).expect("bind");
    let (tx, rx) = tokio::sync::watch::channel(false);
    let h = Arc::clone(&handler);
    tokio::spawn(async move { server.run(h, rx).await.ok() });
    tokio::time::sleep(Duration::from_millis(10)).await;

    let mut client = Client::connect(&path).await.expect("connect");
    let resp = client.send(Method::Ping).await.expect("send");
    assert!(matches!(resp.result, Some(ResultPayload::Pong { .. })));

    tx.send(true).ok();
    let _ = std::fs::remove_file(&path);
}
