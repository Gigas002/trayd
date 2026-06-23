# Changelog

All notable changes to this project will be documented in this file.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

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
