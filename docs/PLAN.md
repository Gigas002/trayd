# trayd — Rust architecture + implementation plan

This document is the **human roadmap** and **agent playbook** for **trayd**: a minimal **Wayland-session** system-tray **daemon** and **standardized IPC** surface for bars, launchers, and terminal clients.

It mirrors the execution discipline of [`docs/ABAR_PLAN.md`](ABAR_PLAN.md) (abar) and `wau/docs/WAU_RS_PLAN.md`:

- Library-first crate split, small verifiable phases, strict quality gates (fmt, clippy `-D warnings` with feature matrix, tests, `cargo doc`, `typos`, `cargo deny`).
- **Directory modules** with **sibling `tests.rs`** — tests never live in the same file as logic.
- **Per-integration Cargo features** where optional surfaces would otherwise bitrot CI.

**Design authority (confirmed):** [abar issue #7 — feature: tray](https://github.com/Gigas002/abar/issues/7) (open, assignee @Gigas002, 2026-05-18).

---

## 1. Goals, motivation, and constraints

### 1.1 Problem (from abar #7)

Tray support in minimal bars (`ashell`, `waybar`, **abar**) is hard to keep **small** and **correct**: StatusNotifierItem (SNI) hosting, D-Bus menu trees, pixmap lifecycles, and in-bar menus fight the “no heavyweight UI toolkit” rule.

**Decision:** extract tray into a dedicated daemon **`trayd`** so:

- One **D-Bus (`zbus`)** implementation is the **single source of truth** for tray state on the session.
- **abar** (and other bars) talk to **`trayd` via IPC only** — **no** `libtrayd` dependency in the bar (simpler bar crate graph; user can swap tray UX without recompiling abar).
- **Menus** are **not** drawn inside trayd or abar: spawn the user’s **dmenu-compatible launcher** (`rofi --dmenu`, `tofi --dmenu`, …) or a **TUI** (`trayd-client`), each speaking the **same IPC** to trayd.
- Optional full-screen tray UX: **`trayd-client`** (ratatui), inspired by [tray-tui](https://github.com/Levizor/tray-tui) — same role as “run `tray-tui`” in #7, but native to this repo.

**References (read-only, not dependencies):**

- [system-tray](https://github.com/JakeStanger/system-tray) — protocol/event shape reference; **do not** depend on the crate.
- [tray-tui](https://github.com/Levizor/tray-tui) — TUI interaction patterns for `trayd-client`.

### 1.2 Goals

- **Minimal surface area**: smallest useful SNI host + IPC; no GTK/Qt/iced/winit in this repo.
- **Session D-Bus host**: implement **StatusNotifierWatcher** + **StatusNotifierItem** + **DBusMenu** handling with **`zbus` only** (no `libdbus` / `dbus-glib`), same policy as abar §5.3.
- **Stable IPC**: documented, versioned protocol so **abar**, **trayd-client**, **dmenu** wrappers, and scripts interoperate via the **socket** (and optionally the **`trayd` CLI**) — **no** shared Rust crates between consumers.
- **Wayland-native ecosystem**: trayd does not paint a bar; it runs in the user’s Wayland session and feeds **pixmaps + metadata** to consumers. **abar** continues Cairo+Pango rendering (Phase 4 icon path reused for tray icons).
- **Tokio** for all async D-Bus and IPC I/O; no blocking the session bus thread pool on subprocesses.

### 1.3 Non-goals

- **No GUI** in trayd: no windows, popovers, GTK menus, or bar drawing.
- **No** dependency on [system-tray](https://github.com/JakeStanger/system-tray) (crate).
- **No** embedded dmenu/rofi/tofi UI — only spawn user-configured commands from **abar** config (see §6.2); trayd exposes data + actions over IPC.
- **No** promise of full freedesktop tray parity on day one; grow toward spec coverage in phases with explicit verify steps.
- **No** MPRIS, notifications, or power applets in trayd (abar defers MPRIS separately).

### 1.4 Relationship to abar Phase 7

[`ABAR_PLAN.md`](ABAR_PLAN.md) Phase 7 originally described an in-process **`libabar` `tray` feature** with `zbus`. With trayd, **abar’s tray module becomes an IPC client**:

| Layer                            | Responsibility                                                                   |
| -------------------------------- | -------------------------------------------------------------------------------- |
| **trayd** daemon (`trayd` crate) | Runs `libtrayd` host + **IPC server**; **CLI** for all consumers                 |
| **libtrayd**                     | D-Bus SNI/menu host only — **no** IPC, **no** CLI                                |
| **abar `libabar` tray**          | Poll/subscribe IPC → segments + pixmaps → Cairo paint; spawn `launcher` on click |
| **User launcher**                | dmenu line protocol or terminal + `trayd-client`                                 |

abar keeps compile-time `tray` feature for **wiring only** (IPC client + render), not for hosting D-Bus.

### 1.5 Definitions

- **Item**: one StatusNotifier registration (service + path identity stable in IPC).
- **Pixmap**: icon payload (format, size, ARGB/RGBA bytes) suitable for abar’s existing decode/blit path.
- **Menu session**: short-lived IPC context for navigating a DBusMenu tree (dmenu or TUI).
- **Consumer**: any process using the **IPC socket** or **`trayd` subcommands** (abar, `trayd-client`, launcher child, shell) — never `libtrayd` / `trayd` as libraries.

---

## 2. Repository layout (target)

Workspace root already lists `libtrayd`, `trayd`, `trayd-client` in [`Cargo.toml`](../Cargo.toml). **abar** lives under `abar/` as a **sibling tree** for integration testing only — not a workspace member of trayd.

```text
trayd/                           # workspace root
  Cargo.toml                     # members: libtrayd, trayd, trayd-client
  Cargo.lock
  deny.toml
  examples/
    trayd.toml                   # daemon: socket path, log level (optional)
    ipc-examples/                # request/response samples for docs/tests
  libtrayd/
    Cargo.toml                   # zbus, tokio, tracing, thiserror — tray host only
    src/
      lib.rs
      error.rs                   # thiserror; D-Bus / host errors only
      model/                     # ItemId, Pixmap, MenuNode, HostEvent — Rust API, not wire JSON
        mod.rs
        tests.rs
      dbus/                      # SNI watcher, item proxy, menu host
        mod.rs
        tests.rs
      host/                      # TrayHost: dbus + in-memory state; sync/async API for trayd
        mod.rs
        tests.rs
  trayd/
    Cargo.toml                   # [[bin]] only; clap, toml, serde_json, libtrayd
    src/
      main.rs                    # thin: parse CLI → run subcommand or daemon
      error.rs                   # config / CLI / IPC errors
      ipc/                       # wire protocol, codec, socket server + client
        protocol.rs
        codec.rs
        server.rs
        client.rs
        mod.rs
        tests.rs
      cli/
        mod.rs                   # subcommands call ipc::Client or start daemon
        tests.rs
      daemon/
        mod.rs                   # TrayHost (libtrayd) + ipc::Server run loop
        tests.rs
      config/                    # optional trayd.toml (XDG)
        mod.rs
        tests.rs
  trayd-client/
    Cargo.toml                   # ratatui, crossterm, tokio, serde_json — no libtrayd, no trayd
    src/
      main.rs
      ipc/                       # socket client + wire types (per docs/IPC.md; not shared with trayd crate)
        mod.rs
        tests.rs
      app/                       # tree UI, keybindings
        mod.rs
        tests.rs
      config/                    # XDG config (trayd-client.toml), tray-tui-inspired
        mod.rs
        tests.rs
  docs/
    PLAN.md                      # this file
    IPC.md                       # Phase 1: protocol spec (generated alongside codec)
  .github/workflows/             # already expect three crates
```

### 2.1 Crate boundary rules (match abar: lib = domain, bin crate = wiring + IPC + CLI)

Same split as **`libabar` / `abar`**: library holds protocol/domain logic; the **`trayd`** crate holds everything external consumers touch.

| Crate            | Responsibility                                                                    | Allowed deps                                                                               | Forbidden                                                                                                       |
| ---------------- | --------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------- |
| **libtrayd**     | `TrayHost`, SNI/DBusMenu, item/menu/pixmap **in-process API**                     | `zbus`, `tokio`, `tracing`, `thiserror` (add `serde` only if host types need it for tests) | **IPC** (socket, NDJSON, wire types), **clap**, **toml**, **ratatui**, `println!` / `eprintln!` — use `tracing` |
| **trayd**        | **IPC server**, **CLI**, daemon loop, config; wires `libtrayd::TrayHost` → socket | `libtrayd`, `clap`, `toml`, `serde`, `serde_json`, `tracing-subscriber`, `tokio`           | Reimplementing D-Bus tray protocol outside `libtrayd`                                                           |
| **trayd-client** | ratatui TUI; **IPC socket client** (own wire types per `docs/IPC.md`)             | `ratatui`, `crossterm`, `tokio`, `serde`, `serde_json`                                     | **`libtrayd`**, **`trayd` crate**, D-Bus, IPC **server**                                                        |

**`trayd` package:** **`[[bin]]` only** — `ipc/`, `cli/`, `daemon/` are private modules of the daemon binary (same crate, not a published library).

**All external consumers** (abar, **trayd-client**, shell, dmenu): talk to the running daemon via **Unix socket NDJSON** and/or **`trayd …` subprocess** — **no** `libtrayd`, **no** `trayd` library link (issue #7). Wire format is specified in **`docs/IPC.md`**; each consumer implements its side independently (acceptable duplication of serde structs).

**No `client` / `daemon` features on `libtrayd`** — optional feature splits belong on **`trayd`** if ever needed (e.g. CLI-only tools), not on the host library.

---

## 3. IPC protocol (standardized contract)

### 3.1 Transport

- **Primary:** Unix domain socket, default `$XDG_RUNTIME_DIR/trayd.sock` (overridable in `trayd.toml` and `--socket` CLI).
- **Framing:** **newline-delimited JSON** (NDJSON) for v1 — easy to debug with `socat`, scriptable from `sh`, usable from abar via `trayd …` subprocess without linking Rust.
- **Version field** on every request: `{ "v": 1, "method": "…", … }`.

Document fully in `docs/IPC.md` in Phase 1; keep `examples/ipc-examples/*.json` as golden fixtures.

### 3.2 Methods (v1 minimum)

| Method               | Purpose                                                                           | Typical caller              |
| -------------------- | --------------------------------------------------------------------------------- | --------------------------- |
| `ping`               | health / version                                                                  | systemd, abar startup       |
| `list`               | all items: id, title, status, attention icon flag                                 | abar render loop, CLI       |
| `subscribe`          | long-lived stream of `item_added`, `item_removed`, `item_updated`, `menu_changed` | abar (after initial `list`) |
| `get_pixmap`         | `{ item_id, size }` → PNG or raw ARGB + dimensions                                | abar icon cache             |
| `activate`           | primary activation (button 1)                                                     | abar click, CLI             |
| `secondary_activate` | middle click where supported                                                      | abar                        |
| `scroll`             | `{ item_id, direction, delta }`                                                   | abar scroll binding         |
| `menu_open`          | returns flat or tree snapshot for item                                            | dmenu / trayd-client entry  |
| `menu_select`        | `{ session_id, node_id }` → may return nested `menu_open` payload for submenu     | dmenu respawn, trayd-client |
| `menu_close`         | end session                                                                       | launcher exit               |

**Errors:** `{ "v":1, "error": { "code": "…", "message": "…" } }` — stable `code` enum (`NOT_FOUND`, `BUS_FAILED`, `INVALID_SESSION`, …).

### 3.3 dmenu integration (abar #7)

Launcher contract (implemented by **abar**, not trayd):

1. User sets `[tray] launcher = "tofi --dmenu"` and `dmenu = true`.
2. On tray button / item click needing a menu, abar runs:  
   `trayd menu-dmenu --item <id>` (or `trayd menu-dmenu --item <id> --session <id>` for nested level).
3. **trayd** CLI talks IPC to daemon, prints **one line per menu row** (label + hidden id), reads selected line from stdin, calls `menu_select`, exits; if submenu, **respawn** same launcher command with new `--session` (issue #7).

For `dmenu = false`, abar may spawn `trayd-client` in a terminal or rely on per-item `activate` only.

### 3.4 Why consumers use IPC / CLI only, not Rust libraries

Per [issue #7](https://github.com/Gigas002/abar/issues/7): avoids version lock-in, keeps dependents minimal, and allows swapping trayd without rebuilding bars or the TUI.

- **`libtrayd`**: tray **host** — linked only by the **`trayd` daemon** binary in this repo.
- **`trayd`**: daemon + **CLI** — defines the **public contract** (socket + subcommands).
- **`trayd-client`**, **abar**, launchers: same as any third-party app — **socket client** and/or spawn **`trayd`**; implement NDJSON against **`docs/IPC.md`**, no crate dependency on `trayd` or `libtrayd`.

---

## 4. D-Bus host design (libtrayd)

### 4.1 Protocols

- Register **org.kde.StatusNotifierWatcher** (or freedesktop equivalent used by target apps).
- Track **StatusNotifierItem** instances on the session bus.
- For each item: properties (IconName, IconPixmap, Attention\*, ToolTip, Category, Status), **`Activate`**, **`SecondaryActivate`**, **`Scroll`**.
- **DBusMenu** (`com.canonical.dbusmenu`): layout + property updates; expose as **`MenuNode` tree** in `libtrayd::model` with stable `node_id` (mapped to IPC ids in **`trayd::ipc`**, not in libtrayd).

### 4.2 Implementation notes

- **Tokio** + **zbus** connection on a dedicated task; channel into `TrayHost` state machine.
- **Semantic reference:** ashell tray sources (registration order, watcher protocol, edge cases) — **do not** copy iced/GTK UI.
- **Pixmap policy:** cache by `(item_id, size)`; invalidate on `NewIcon` / property change events.
- **Single daemon** per session: enforced in **`trayd::daemon`** (socket + PID file), not in libtrayd.

### 4.3 Headless / CI

- **libtrayd:** menu diff logic, host state machine, D-Bus fixtures (no sockets).
- **trayd:** IPC codec roundtrips, CLI parsing, daemon integration with mock `TrayHost`.
- Integration: `zbus` test bus or documented **manual** checklist; CI may `#[ignore]` live D-Bus tests (same spirit as abar Phase 7 verify).

---

## 5. Binaries

### 5.1 `trayd` (daemon + CLI)

**Default mode:** run daemon (foreground or `--detach` later).

**Subcommands (v1):**

| Command               | Role                             |
| --------------------- | -------------------------------- |
| `trayd` / `trayd run` | Start D-Bus host + IPC server    |
| `trayd ping`          | IPC health                       |
| `trayd list`          | Human or `--json` machine output |
| `trayd activate <id>` | Scripting                        |
| `trayd menu-dmenu …`  | Pipe-friendly menu for launchers |
| `trayd subscribe`     | Debug stream (optional)          |

**Config (`examples/trayd.toml`):** socket path, `RUST_LOG` default, optional max items (future).

**Systemd:** user unit `trayd.service` documented in Phase 8 (not required for first milestone).

### 5.2 `trayd-client` (TUI)

Inspired by [tray-tui](https://github.com/Levizor/tray-tui):

- Tree navigation of tray items + DBusMenu hierarchy.
- Keys: arrows/hjkl, Enter activate, open submenu, `q`/Ctrl-C quit.
- Config: `$XDG_CONFIG_HOME/trayd-client/config.toml` (bindings, colors, optional socket path).
- Connect to `$XDG_RUNTIME_DIR/trayd.sock` (default) with **`trayd-client/src/ipc/`** — same NDJSON methods as §3.2; **no** `trayd` / `libtrayd` in `Cargo.toml`.
- **No** dependency on tray-tui crate.

**Use cases:** `launcher = "trayd-client"` in abar #7 example; standalone terminal tray; debugging.

---

## 6. abar integration (consumer spec — implemented in abar repo later)

### 6.1 Config (from issue #7)

`config.toml`:

```toml
[tray]
dmenu = true
launcher = "tofi --dmenu"
# launcher = "trayd-client"   # TUI in terminal
```

`theme.toml`:

```toml
[tray]
# icons | submenu | simple
style = "icons"
```

| `style`   | abar behavior                                                                                |
| --------- | -------------------------------------------------------------------------------------------- |
| `icons`   | One segment per tray item; pixmap from `get_pixmap`; click → `activate` or menu via launcher |
| `submenu` | Single “tray” segment; click opens launcher listing items                                    |
| `simple`  | Label `tray: N`; click opens launcher                                                        |

### 6.2 abar runtime flow

1. **Startup:** ensure trayd running (`trayd ping` → else spawn `trayd` or warn per config).
2. **Background task (Tokio):** `trayd subscribe` subprocess or socket client → update shared tray snapshot → signal Wayland loop to repaint (same pattern as Hyprland keyboard socket).
3. **Render:** map items to `Segment`s + icon decode (reuse `libabar::icon`).
4. **Input:** hit-test tray segments → `trayd activate` / spawn `launcher` with `trayd menu-dmenu` for menus.
5. **No** `libtrayd` in `abar/Cargo.toml`.

### 6.3 Revised abar Phase 7 checklist (cross-repo)

- [ ] trayd daemon shipped (this repo).
- [ ] abar `tray` feature: IPC client + styles + launcher spawn only.
- [ ] Document dependency: trayd in PATH; optional systemd user service.

---

## 7. Quality gates (mirror abar §7)

When a phase is marked complete:

- `cargo fmt --check`
- `typos`
- `cargo deny check` (populate `deny.toml` allow list as crates land)
- `cargo clippy --workspace --all-targets --no-default-features -- -D warnings`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace --no-default-features`
- `cargo test --workspace --all-features`
- `cargo doc --workspace --no-deps`

**Test discipline:** `tests.rs` per directory module; integration tests under `libtrayd/tests/`, `trayd/tests/`, `trayd-client/tests/`.

**CI:** existing workflows (`build`, `fmt-clippy`, `test`, `doc`, `typos`, `deny`, `deploy` for `trayd` binary) — trayd crates need **no** Cairo/Pango in build matrix (remove from trayd jobs once abar is not built in same job; today workflows install Cairo for abar subtree — split or scope installs to `abar/` only when trayd crates exist).

---

## 8. Phased steps

### Phase 0 — Workspace scaffold + hygiene

- [x] Create `libtrayd`, `trayd`, `trayd-client` crates matching §2 (empty `lib.rs`, `pub fn stub()` if needed for CI).
- [x] Wire workspace `version`, `edition = "2024"`, shared `license`, `repository`.
- [x] Populate **`deny.toml`** allow list incrementally.
- [x] `tracing-subscriber` in **`trayd`** / **`trayd-client`** binaries only; **`libtrayd`** uses `tracing` macros only.
- [x] Add `docs/IPC.md` stub pointing to Phase 1.

**Verify:** all §7 gates green on empty crates; `cargo build -p trayd -p trayd-client -p libtrayd`.

### Phase 1 — IPC protocol + skeleton (**trayd** crate only)

- [x] `trayd::ipc::protocol` wire types (v1 methods in §3.2); map to/from `libtrayd` types at the daemon boundary only.
- [x] NDJSON codec + Unix socket **server** + **client** under `trayd/src/ipc/`.
- [x] `trayd ping` / `trayd list` against a **mock** `TrayHost` (test double in `trayd` tests, not D-Bus).
- [x] Golden tests from `examples/ipc-examples/`.

**Verify:** unit tests in `trayd/src/ipc/tests.rs`; integration test with temp socket; `docs/IPC.md` complete. **libtrayd** may still be empty or stub.

### Phase 2 — D-Bus SNI host (**libtrayd**) + daemon wiring (**trayd**)

- [x] `libtrayd::dbus/` watcher + item registration lifecycle.
- [x] `libtrayd::TrayHost`: list, pixmap, activate, scroll, host event stream.
- [x] `trayd::daemon`: run `TrayHost` + `ipc::Server`; single-instance socket policy.
- [x] `trayd` CLI: `list`, `activate`, `subscribe` via `ipc::Client`.

**Verify:** manual with `nm-applet`, `blueman`, or `telegram` tray; document apps used; CI ignores live bus or uses test bus if feasible.

### Phase 3 — DBusMenu (**libtrayd**) + menu IPC/CLI (**trayd**)

- [x] Menu tree snapshot + diff events on `TrayHost`.
- [x] `menu_open` / `menu_select` / `menu_close` in `trayd::ipc`.
- [x] `trayd menu-dmenu` CLI (§3.3).

**Verify:** manual menu on an app with tray menu; scripted test with recorded menu fixture.

### Phase 4 — `trayd-client` TUI

- [ ] `trayd-client/src/ipc/`: socket client + wire types (mirror `docs/IPC.md`; no `trayd` crate dep).
- [ ] ratatui tree UI: `list` / `subscribe`, `menu_open` / `menu_select`, `activate` over the socket.
- [ ] Default config + example `trayd-client.toml`.
- [ ] Parity with core tray-tui flows (navigate, activate, quit) — not pixel parity.

**Verify:** manual TUI session; unit tests for view model / key dispatch without terminal.

### Phase 5 — Pixmap hardening + performance

- [ ] Pixmap cache, size negotiation, attention icon swap.
- [ ] Rate-limit noisy updates; coalesce `item_updated` bursts.

**Verify:** stress with multiple items; memory stable in manual run.

### Phase 6 — abar integration support (trayd side complete)

- [ ] Stable `subscribe` stream for bar repaint driver.
- [ ] Document abar config (§6) in trayd README.
- [ ] `examples/trayd.toml` + install instructions.

**Verify:** dogfood with abar branch implementing §6 (cross-repo); checklist in README.

### Phase 7 — Polish + first release

- [ ] README: architecture diagram, IPC summary, systemd example, relation to abar #7.
- [ ] CHANGELOG; tag `v0.1.0`.
- [ ] crates.io publish `libtrayd`, `trayd` (trayd-client optional).

**Verify:** full §7 gates; release workflow (`deploy.yml`) produces stripped `trayd` tarball.

---

## 9. Definition of done (v0.1)

- [ ] **`trayd` daemon** hosts real StatusNotifier items via **zbus** on a Wayland session.
- [ ] **IPC v1** documented and used by **`trayd` CLI** without linking consumers to D-Bus.
- [ ] **`trayd menu-dmenu`** works with at least one external dmenu launcher (`tofi --dmenu` or `rofi -dmenu`).
- [ ] **`trayd-client`** provides terminal tray UX via IPC only.
- [ ] **No GUI** and **no system-tray crate** dependency.
- [ ] **abar** can integrate per §6 using subprocess IPC only (integration may land in abar repo after trayd v0.1).
- [ ] CI green: default / all-features / no-default-features; docs build.

---

## 10. Dependency policy

- **Edition:** `2024` (workspace).
- **Async:** `tokio` (`rt-multi-thread`, `net`, `io-util`, `process`, `sync`, `time`).
- **D-Bus:** `zbus` only for session bus.
- **CLI:** `clap` 4 derive in **trayd** only.
- **TUI:** `ratatui` + `crossterm` in **trayd-client** only.
- **Serialization:** `serde` + `serde_json` in **`trayd`** (daemon IPC) and **`trayd-client`** (socket client wire types per `docs/IPC.md`) — duplicated structs, not a shared crate.
- **Versions:** `x.y` in manifests; committed lockfile.
- Justify new deps in PR; keep **`libtrayd`** minimal (D-Bus host only).

---

## 11. Document maintenance

Update this plan when:

- IPC version or methods change (update `docs/IPC.md` + examples first).
- abar #7 or `ABAR_PLAN.md` tray sections change.
- crate split or feature policy changes.

---

## Revision history

| Date       | Change                                                                                                                                                                 |
| ---------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 2026-05-18 | Initial trayd plan: confirmed abar #7; three crates; IPC-first; abar consumer spec; phased roadmap                                                                     |
| 2026-05-18 | IPC + CLI live in **trayd** crate; **libtrayd** = D-Bus host only (abar/libabar split)                                                                                 |
| 2026-05-18 | **trayd-client** (and all consumers) use socket/CLI only — no link to `trayd` or `libtrayd` crates                                                                     |
| 2026-05-18 | **Phase 0** done: three crates scaffolded, CI without Cairo, `deny.toml` populated                                                                                     |
| 2026-05-18 | **Phase 1** done: IPC protocol wire types, NDJSON codec, Unix-socket server+client, `StubHandler`, `trayd ping/list/activate`, golden fixtures, `docs/IPC.md` complete |
| 2026-05-18 | **Phase 2** done: `libtrayd` SNI watcher + item proxy (`zbus`), `TrayHost` (list/pixmap/activate/scroll/subscribe), `Handler` trait async, `TrayHostHandler`, daemon D-Bus wiring with stub fallback, single-instance check, `trayd subscribe` CLI |
| 2026-05-19 | **Phase 3** done: `libtrayd` DBusMenu proxy (`com.canonical.dbusmenu`), `MenuNode` model, `HostEvent::MenuChanged`, `TrayHost::get_menu/menu_event/about_to_show`, session management in `TrayHostHandler`, `menu_open/select/close` IPC, `trayd menu-dmenu` CLI with optional external launcher, `LayoutUpdated` background monitor per item |
