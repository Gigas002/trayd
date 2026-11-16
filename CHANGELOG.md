# Changelog

All notable changes to this project will be documented in this file.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

Findings and fixes from the audit in [`docs/AUDIT.md`](docs/AUDIT.md).

**Breaking (`libtrayd`):** `MenuNode` gains public fields, so the next release must be 0.3.0.

### Fixed

- **`libtrayd`**: title, icon, tooltip, overlay and menu changes after the first were never seen: zbus cached the properties and SNI apps don't send `PropertiesChanged`.
- **`libtrayd`**: a burst of more than 64 change signals (e.g. 500 `NewIcon`) froze the whole D-Bus connection for good; signals are now coalesced and never block the reader.
- **`libtrayd`**: one app that never answered property reads blocked every other registration until it exited; registrations now run concurrently and every D-Bus call has a 10 s timeout.
- **`libtrayd`**: starting a second host (e.g. another trayd) took `org.kde.StatusNotifierWatcher` from the first and left it with no owner when it exited; it now fails with an error.
- **`libtrayd`**: registering a name nobody owns (or an empty / malformed string) created a permanent ghost item; such registrations are rejected with `InvalidArgs`.
- **`libtrayd`**: an app leaving the bus also unregistered apps whose name started with its name (`org.example.App` removed `org.example.App2`).
- **`libtrayd`**: items came out in hash order and reshuffled as apps were added; they now keep registration order.
- **`libtrayd`**: pixmaps with impossible sizes (negative, or not `width × height × 4` bytes) panicked debug builds and were served as 2147483648 × 4294967291 images; they are dropped.
- **`libtrayd`**: `get_pixmap` kept a copy of the icon per requested size (500 MB after 2000 sizes); the cache is gone.
- **`libtrayd`**: menu labels kept `_` mnemonic markers; submenus filled on `AboutToShow` came back empty.
- **`trayd`**: request lines were unbounded (300 MB without a newline grew the daemon to 375 MB); they are limited to 64 KiB.
- **`trayd`**: startup deleted any file at `socket_path`; only a stale socket is removed now. No `/tmp` fallback; socket mode is `0600`; the socket is removed on `SIGINT`/`SIGTERM`.
- **`trayd`**: `--socket` was documented but not accepted.
- **`trayd`/`trayctl`**: logs went to stdout (trayd) and always carried ANSI colour codes.
- **`trayctl`**: picking the second of two rows with the same label activated the first; disabled rows and separators were offered; control characters in labels reached the menu program; quoted `--dmenu-cmd` arguments were split apart.
- **`tray-tui`**: log lines were drawn over the UI; a disconnected daemon was not shown; the cursor jumped to another app when items were added; separators and disabled rows could be activated.

### Added

- **`libtrayd`**: `TrayHost::start_on(address)`, `TraydError::is_watcher_taken()`, `MenuNode::is_separator` and `MenuNode::toggle_state`.
- **IPC**: `INVALID_REQUEST` error code (malformed request, unknown command, unsupported `v`, over-long line); menu rows may carry `enabled`, `separator` and `checked`. Existing clients keep working.

### Changed

- **`libtrayd`**: new items' properties are read with one `GetAll` instead of 13 `Get` calls; the well-known name of path-registered apps is looked up once instead of on every `NewIcon`.
- **`trayctl`**: `--dmenu-cmd` runs through `sh -c`.
- Live tests run on a private `dbus-daemon` and are no longer `#[ignore]`d.

## [0.2.0] - 2026-09-04

### Fixed

- **`libtrayd`**: `StatusNotifierWatcher` now emits the `StatusNotifierItemUnregistered` signal
  (per the SNI watcher spec) for each service pruned from `WatcherInner::items` when its bus
  name disappears, instead of only updating internal state silently.
- **`tray-tui`**: dropped the explicit `crossterm` dependency in favor of `ratatui`'s bundled
  `crossterm` feature, avoiding a version mismatch between the two crates.
- **`libtrayd`**: resource leak causing `Too many open files (EMFILE)` crash after 15–45 minutes
  of runtime. Per-item `run_item_signal_watcher` tasks were never cancelled when the owning bus
  name disappeared: each task held a live `StatusNotifierItemProxy`, a property-cache background
  task, and 7 D-Bus signal subscriptions that accumulated without bound as services disconnected
  and reconnected (e.g. Steam cycling its tray icon). Fix:
  - `HostState` now stores a `JoinHandle` per item; the handle is aborted via
    `HostState::remove_item()` when `NameOwnerChanged` signals the bus name is gone. Aborting
    the task drops the proxy, which in turn aborts the property-cache task and queues
    `RemoveMatch` for all held signal subscriptions.
  - `StatusNotifierWatcher::new` returns the shared `Arc<Mutex<WatcherInner>>` so the host loop
    can also prune stale `RegisteredStatusNotifierItems` entries for the gone bus name, allowing
    services to re-register cleanly under a new unique name after a restart.

### Added

- **`libtrayd` / `trayd` / `trayctl`**: `TrayItem` now carries three new SNI fields read
  during item registration:
  - `category` — item category string per the SNI spec (`"ApplicationStatus"`,
    `"Communications"`, `"SystemServices"`, `"Hardware"`).
  - `item_is_menu` — `true` when the item is a pure menu with no application window.
    `TrayHost::activate` now returns `ActivationFailed` when called with `item_id = 0`
    on a menu-only item; use `get_menu` / `activate` with a non-zero `item_id` instead.
  - `tool_tip` — tooltip title, description, and icon data.
- **IPC `MinimalTrayItem`**: the wire type exposed by `get_items` / `subscribe` now
  includes `category` (string | absent), `item_is_menu` (bool, omitted when `false`),
  `tooltip_title` (string | absent), `tooltip_description` (string | absent), and
  `overlay_icon_handle` (string | absent).
  Existing clients are unaffected — the new fields default gracefully when absent.

- **`trayctl subscribe`**: built-in CLI alternative to subscribing via the socket directly. Sends
  a `subscribe` request and streams tray-state updates as NDJSON to stdout — one line per event,
  each line a JSON array of `MinimalTrayItem` objects. Exits cleanly when the daemon closes the
  connection. The raw socket `subscribe` command remains fully supported for custom clients.

### Docs

- `docs/ARCHITECTURE.md`: overview of the workspace crates and how the daemon, host, and clients
  fit together.

## [0.1.1] - 2026-06-02

### Fixed

- **`trayctl` / `libtrayd`**: icon handle now resolves correctly for apps (e.g. Telegram) that
  leave the SNI `IconName` property empty and register on the session bus under a well-known
  name different from their SNI connection. The lookup walks `org.freedesktop.DBus.ListNames`
  and matches by Unix PID, mapping e.g. `:1.201` → `org.telegram.desktop`. Fallback order:
  `IconName` → well-known bus name → SNI `Id`.
- **`trayctl`**: tracing logs now go to `stderr` instead of `stdout`, keeping JSON output
  on `stdout` clean for piping and parsing.

## [0.1.0] - 2026-05-31

### Added

#### Core daemon (`trayd`)

- Unix domain socket IPC server (NDJSON v1, default `$XDG_RUNTIME_DIR/trayd.sock`)
- Commands: `ping`, `subscribe`, `get_items`, `get_menu`, `activate`, `get_pixmap`
- Single-instance enforcement via socket lock
- `trayd run` and `trayd ping` CLI subcommands
- Config file support (`$XDG_CONFIG_HOME/trayd/trayd.toml`) — socket path, log filter

#### Library (`libtrayd`)

- `TrayHost`: registers `org.kde.StatusNotifierWatcher` on the session D-Bus, maintains an in-memory item cache, and fans out `HostEvent`s to subscribers
- `StatusNotifierItem` + `DBusMenu` zbus proxies for reading properties and invoking methods
- Per-item D-Bus signal watchers (`NewIcon`, `NewTitle`, `NewStatus`, `NewAttentionIcon`) to keep the cache fresh
- In-process pixmap cache keyed by `(app_id, requested_size)`; invalidated on icon/status signals and item removal
- `NeedsAttention` status: `attention_icon` field on `TrayItem`; `get_pixmap` prefers attention pixmaps when status warrants
- `PixmapData` return type carrying actual `width` and `height` alongside raw ARGB32 bytes

#### `trayctl`

- One-shot menu orchestrator: `trayctl menu --app-id <id>` drives a dmenu-compatible tool through nested submenus
- Default picker: `tofi --mode dmenu`; override with `--dmenu-cmd`
- `trayctl items` prints the current `get_items` snapshot as JSON
- Submenu loop: `get_menu` → pipe labels to dmenu stdin → read selection → recurse or `activate`

#### `tray-tui`

- ratatui full-screen terminal UI over the IPC socket
- Navigates items and nested menu levels entirely in-terminal — no external picker spawned
- Config: `$XDG_CONFIG_HOME/tray-tui/config.toml` (socket path override)

#### Protocol & docs

- `docs/IPC.md`: complete v1 wire protocol specification with type definitions and golden examples
- `examples/ipc-examples/`: golden NDJSON request/response fixtures for all commands
- `examples/trayd.toml`, `examples/tray-tui.toml`: annotated config examples
- `examples/abar.md`: bar-integration guide (IPC flow, startup sequence, systemd unit)
- Coalesced `subscribe` updates: rapid bursts of events are batched within a 50 ms window before a snapshot is sent
