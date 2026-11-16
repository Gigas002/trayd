//! Shared fixtures for the live integration tests.
//!
//! Every test gets its own private `dbus-daemon`, so the tests never touch the
//! desktop session bus (where they would compete with the user's real tray for
//! `org.kde.StatusNotifierWatcher`). When `dbus-daemon` is not installed the
//! tests print a notice and pass vacuously.

#![allow(dead_code)] // each test binary uses a different subset

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use libtrayd::{TrayHost, TrayItem};
use zbus::object_server::SignalEmitter;
use zbus::{OwnedVariant, Variant, interface};

// ─── Private bus ─────────────────────────────────────────────────────────────

/// A `dbus-daemon` that lives as long as this value.
pub struct TestBus {
    child: Child,
    pub address: String,
}

impl TestBus {
    /// Start a private session bus, or `None` when `dbus-daemon` is missing.
    pub fn start() -> Option<Self> {
        let mut child = match Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address=1"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                eprintln!("skipping: cannot run dbus-daemon: {e}");
                return None;
            }
        };
        let mut line = String::new();
        BufReader::new(child.stdout.take().expect("piped"))
            .read_line(&mut line)
            .expect("read dbus-daemon address");
        Some(Self {
            child,
            address: line.trim().to_owned(),
        })
    }

    /// A fresh client connection to this bus.
    pub async fn connect(&self) -> zbus::Connection {
        zbus::connection::Builder::address(self.address.as_str())
            .build()
            .await
            .expect("connect to test bus")
    }

    /// Start a tray host on this bus.
    pub async fn host(&self) -> TrayHost {
        TrayHost::start_on(&self.address)
            .await
            .expect("start host on test bus")
    }
}

impl Drop for TestBus {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// `let Some(bus) = TestBus::start() else { return };` in one line.
#[macro_export]
macro_rules! test_bus {
    () => {
        match common::TestBus::start() {
            Some(bus) => bus,
            None => return,
        }
    };
}

/// Poll `host` until `pred` holds for its item list, or panic after 5 s.
pub async fn wait_for(
    host: &TrayHost,
    what: &str,
    pred: impl Fn(&[TrayItem]) -> bool,
) -> Vec<TrayItem> {
    for _ in 0..500 {
        let items = host.items().await;
        if pred(&items) {
            return items;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!(
        "timed out waiting for: {what}; items = {:#?}",
        host.items().await
    );
}

// ─── Fake tray app ───────────────────────────────────────────────────────────

/// One menu row served by [`FakeMenu`].
#[derive(Clone, Debug)]
pub struct Row {
    pub id: i32,
    pub props: Vec<(&'static str, Prop)>,
    /// `Some` marks a submenu; its rows are served on `GetLayout(id)`.
    pub children: Option<Vec<Row>>,
}

#[derive(Clone, Debug)]
pub enum Prop {
    Str(&'static str),
    Bool(bool),
    Int(i32),
}

impl Row {
    pub fn label(id: i32, label: &'static str) -> Self {
        Self {
            id,
            props: vec![("label", Prop::Str(label))],
            children: None,
        }
    }

    pub fn with(mut self, key: &'static str, value: Prop) -> Self {
        self.props.push((key, value));
        self
    }

    pub fn submenu(mut self, rows: Vec<Row>) -> Self {
        self.props.push(("children-display", Prop::Str("submenu")));
        self.children = Some(rows);
        self
    }
}

/// Mutable state of a fake app, shared with the test for scripting and
/// assertions.
#[derive(Debug)]
pub struct FakeState {
    pub id: String,
    pub title: String,
    pub status: String,
    pub icon_name: String,
    pub pixmaps: Vec<(i32, i32, Vec<u8>)>,
    pub item_is_menu: bool,
    pub menu: Vec<Row>,
    /// Submenus with an id listed here stay empty until `AboutToShow(id)`.
    pub lazy_submenus: Vec<i32>,
    /// Never answer property reads (simulates a hung app).
    pub hang: bool,
    // Recorded calls:
    pub activations: Vec<(i32, i32)>,
    pub events: Vec<(i32, String)>,
    pub about_to_show: Vec<i32>,
    pub property_reads: usize,
}

impl Default for FakeState {
    fn default() -> Self {
        Self {
            id: "fake-app".into(),
            title: "Fake App".into(),
            status: "Active".into(),
            icon_name: "fake-icon".into(),
            pixmaps: Vec::new(),
            item_is_menu: false,
            menu: vec![Row::label(1, "Open"), Row::label(2, "Quit")],
            lazy_submenus: Vec::new(),
            hang: false,
            activations: Vec::new(),
            events: Vec::new(),
            about_to_show: Vec::new(),
            property_reads: 0,
        }
    }
}

pub type Shared = Arc<Mutex<FakeState>>;

pub struct FakeItem {
    state: Shared,
}

impl FakeItem {
    async fn read<T>(&self, f: impl FnOnce(&FakeState) -> T) -> T {
        let hang = {
            let mut s = self.state.lock().unwrap();
            s.property_reads += 1;
            s.hang
        };
        if hang {
            std::future::pending::<()>().await;
        }
        f(&self.state.lock().unwrap())
    }
}

#[interface(name = "org.kde.StatusNotifierItem")]
impl FakeItem {
    fn activate(&self, x: i32, y: i32) {
        self.state.lock().unwrap().activations.push((x, y));
    }

    #[zbus(property)]
    async fn id(&self) -> String {
        self.read(|s| s.id.clone()).await
    }
    #[zbus(property)]
    async fn category(&self) -> String {
        self.read(|_| "ApplicationStatus".to_owned()).await
    }
    #[zbus(property)]
    async fn title(&self) -> String {
        self.read(|s| s.title.clone()).await
    }
    #[zbus(property)]
    async fn status(&self) -> String {
        self.read(|s| s.status.clone()).await
    }
    #[zbus(property)]
    async fn icon_name(&self) -> String {
        self.read(|s| s.icon_name.clone()).await
    }
    #[zbus(property)]
    async fn icon_pixmap(&self) -> Vec<(i32, i32, Vec<u8>)> {
        self.read(|s| s.pixmaps.clone()).await
    }
    #[zbus(property)]
    async fn item_is_menu(&self) -> bool {
        self.read(|s| s.item_is_menu).await
    }
    #[zbus(property)]
    async fn menu(&self) -> zbus::OwnedObjectPath {
        self.read(|_| zbus::OwnedObjectPath::try_from("/Menu").unwrap())
            .await
    }
    #[zbus(property)]
    async fn tool_tip(&self) -> (String, Vec<(i32, i32, Vec<u8>)>, String, String) {
        self.read(|s| (String::new(), Vec::new(), s.title.clone(), String::new()))
            .await
    }

    #[zbus(signal)]
    pub async fn new_title(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
    #[zbus(signal)]
    pub async fn new_icon(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
    #[zbus(signal)]
    pub async fn new_status(emitter: &SignalEmitter<'_>, status: &str) -> zbus::Result<()>;
}

/// `GetLayout` node: `(id, properties, children)`.
type Layout = (i32, HashMap<String, OwnedVariant>, Vec<OwnedVariant>);

pub struct FakeMenu {
    state: Shared,
}

fn row_variant(row: &Row, populate: bool) -> Variant<'static> {
    let props: HashMap<String, Variant<'static>> = row
        .props
        .iter()
        .map(|(k, v)| {
            let v = match v {
                Prop::Str(s) => Variant::from(*s),
                Prop::Bool(b) => Variant::from(*b),
                Prop::Int(i) => Variant::from(*i),
            };
            ((*k).to_owned(), v)
        })
        .collect();
    let kids: Vec<Variant<'static>> = match (&row.children, populate) {
        (Some(rows), true) => rows.iter().map(|r| row_variant(r, false)).collect(),
        _ => Vec::new(),
    };
    Variant::from((row.id, props, kids))
}

fn find_row(rows: &[Row], id: i32) -> Option<&Row> {
    rows.iter().find_map(|r| {
        if r.id == id {
            Some(r)
        } else {
            r.children.as_deref().and_then(|c| find_row(c, id))
        }
    })
}

#[interface(name = "com.canonical.dbusmenu")]
impl FakeMenu {
    fn get_layout(&self, parent_id: i32, _depth: i32, _props: Vec<String>) -> (u32, Layout) {
        let s = self.state.lock().unwrap();
        let rows: Vec<Row> = if parent_id == 0 {
            s.menu.clone()
        } else if s.lazy_submenus.contains(&parent_id) {
            Vec::new()
        } else {
            find_row(&s.menu, parent_id)
                .and_then(|r| r.children.clone())
                .unwrap_or_default()
        };
        let kids = rows
            .iter()
            .map(|r| OwnedVariant::try_from(row_variant(r, false)).unwrap())
            .collect();
        (1, (parent_id, HashMap::new(), kids))
    }

    fn about_to_show(&self, id: i32) -> bool {
        let mut s = self.state.lock().unwrap();
        s.about_to_show.push(id);
        let was_lazy = s.lazy_submenus.contains(&id);
        s.lazy_submenus.retain(|&l| l != id);
        was_lazy
    }

    fn event(&self, id: i32, event_id: &str, _data: Variant<'_>, _timestamp: u32) {
        self.state
            .lock()
            .unwrap()
            .events
            .push((id, event_id.to_owned()));
    }
}

/// A running fake tray app with its own bus connection.
pub struct FakeApp {
    pub conn: zbus::Connection,
    pub state: Shared,
}

impl FakeApp {
    /// Serve the item and menu objects; does not register yet.
    pub async fn new(bus: &TestBus, state: FakeState) -> Self {
        let state = Arc::new(Mutex::new(state));
        let conn = bus.connect().await;
        conn.object_server()
            .at(
                "/StatusNotifierItem",
                FakeItem {
                    state: Arc::clone(&state),
                },
            )
            .await
            .unwrap();
        conn.object_server()
            .at(
                "/Menu",
                FakeMenu {
                    state: Arc::clone(&state),
                },
            )
            .await
            .unwrap();
        Self { conn, state }
    }

    /// Claim `name` and register it with the watcher.
    pub async fn register_as(&self, name: &str) {
        self.conn.request_name(name).await.unwrap();
        self.register(name).await.unwrap();
    }

    /// Call `RegisterStatusNotifierItem(service)` from this connection.
    pub async fn register(&self, service: &str) -> zbus::Result<()> {
        self.conn
            .call_method(
                Some("org.kde.StatusNotifierWatcher"),
                "/StatusNotifierWatcher",
                Some("org.kde.StatusNotifierWatcher"),
                "RegisterStatusNotifierItem",
                &(service,),
            )
            .await
            .map(|_| ())
    }

    /// Disconnect from the bus, even while a method call is still pending.
    pub async fn leave(self) {
        self.conn.close().await.unwrap();
    }

    /// The unique bus name of this app.
    pub fn unique_name(&self) -> String {
        self.conn.unique_name().unwrap().to_string()
    }

    /// Change the state and emit the matching SNI change signal.
    pub async fn set_title(&self, title: &str) {
        self.state.lock().unwrap().title = title.to_owned();
        let emitter = SignalEmitter::new(&self.conn, "/StatusNotifierItem").unwrap();
        FakeItem::new_title(&emitter).await.unwrap();
    }

    pub async fn set_icon_name(&self, name: &str) {
        self.state.lock().unwrap().icon_name = name.to_owned();
        let emitter = SignalEmitter::new(&self.conn, "/StatusNotifierItem").unwrap();
        FakeItem::new_icon(&emitter).await.unwrap();
    }

    pub async fn emit_new_icon(&self) {
        let emitter = SignalEmitter::new(&self.conn, "/StatusNotifierItem").unwrap();
        FakeItem::new_icon(&emitter).await.unwrap();
    }

    /// Change the icon name, then emit `n` `NewIcon` signals back to back.
    pub async fn flood_new_icon(&self, final_icon: &str, n: usize) {
        self.state.lock().unwrap().icon_name = final_icon.to_owned();
        let emitter = SignalEmitter::new(&self.conn, "/StatusNotifierItem").unwrap();
        for _ in 0..n {
            FakeItem::new_icon(&emitter).await.unwrap();
        }
    }

    pub fn property_reads(&self) -> usize {
        self.state.lock().unwrap().property_reads
    }

    pub fn events(&self) -> Vec<(i32, String)> {
        self.state.lock().unwrap().events.clone()
    }

    pub fn activations(&self) -> Vec<(i32, i32)> {
        self.state.lock().unwrap().activations.clone()
    }
}

/// Wait until `cond` holds for the fake app's state, or panic after 5 s.
pub async fn wait_app(app: &FakeApp, what: &str, cond: impl Fn(&FakeState) -> bool) {
    for _ in 0..500 {
        if cond(&app.state.lock().unwrap()) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("timed out waiting for: {what}");
}

/// The watcher's `RegisteredStatusNotifierItems` property.
pub async fn registered_items(bus: &TestBus) -> Vec<String> {
    let conn = bus.connect().await;
    let reply = conn
        .call_method(
            Some("org.kde.StatusNotifierWatcher"),
            "/StatusNotifierWatcher",
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &(
                "org.kde.StatusNotifierWatcher",
                "RegisteredStatusNotifierItems",
            ),
        )
        .await
        .unwrap();
    let v: OwnedVariant = reply.body().deserialize().unwrap();
    Vec::<String>::try_from(v).unwrap()
}
