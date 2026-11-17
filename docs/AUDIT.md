# Audit

A whole-application audit of libtrayd, trayd, trayctl and tray-tui
(2026-11-16) in six parts: security (§2), bugs (§3), performance (§4), UI
usability and design (§5), code quality, tests and project hygiene (§6), and
features compared with the StatusNotifier specification and other hosts (§7).
§8 lists what was left open, and §9 is the checklist to re-run the audit.

Each finding says what was found, how serious it is, and what was done:
**fixed** (with the commit), **accepted** (with the reason), or **OK** (checked,
nothing to do). Every fix is its own commit, with a regression test where one
can be written; the tests were checked to fail before the fix.

Severity: **high** (the tray stops working or shows wrong state in normal use,
data loss), **medium** (a crash, hang or unbounded growth reachable by an app or
local client, behaviour that contradicts what the user picked), **low**
(hardening, polish, or needs an unusual setup).

## 1. Method

Every source file was read in full. Each finding below was then reproduced
against a running daemon. The user's own tray (a trayd serving their bar) was
never touched: every live test ran on a **private `dbus-daemon`** (1.16.4) with
its own trayd and its own socket under `$XDG_RUNTIME_DIR`. Tray apps were
simulated by a scriptable fake StatusNotifierItem + DBusMenu (Python,
`dbus-python`) that could:

- change its title or icon and emit `NewTitle` / `NewIcon`, once, in a burst
  of 500, or as a 30 Hz animation;
- register under a well-known name, a bare object path, or a name it doesn't
  own;
- never answer property reads (a hung app);
- serve hostile pixmaps (`width = i32::MIN`) and a menu with mnemonics,
  separators, disabled, check and duplicate rows, an escape sequence, and a
  submenu that fills only on `AboutToShow`.

The IPC socket was probed with `socat` and Python (malformed lines, wrong
versions, 300 MB without a newline, 2000 pixmap sizes). `tray-tui` ran in a
`tmux` pane driven by `send-keys` and read back with `capture-pane`; `trayctl`
used a scripted dmenu that records its input. Method calls were counted with
`dbus-monitor`, CPU from `/proc/<pid>/stat` over 10 s. A background
connection owned 60 extra names, so the bus had about 170 names as on a real
desktop. Static checks: the gates, `cargo deny check`, `typos`,
`cargo llvm-cov`, a `cargo publish --dry-run` of libtrayd.

All processes, sockets and buses created for the audit were removed
afterwards.

## 2. Security

trayd is a per-user daemon. Untrusted input comes from two sides: any process
on the session bus can register as a tray app and controls every property,
pixmap and menu row it reports; any process of the same user can connect to
the IPC socket.

| Area                    | Finding                                                                                                                                                                                                                                                                                                                               | Status                  |
| ----------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------- |
| Socket path deletion    | Startup called `remove_file(socket_path)` unconditionally. A `trayd.toml` pointing at a real file deleted it (reproduced with a scratch file). Only a stale socket is removed now; anything else stops the daemon before it touches the bus.                                                                                          | Fixed, medium (c9f1526) |
| `/tmp` fallback         | Without `XDG_RUNTIME_DIR` the daemon listened on `/tmp/trayd.sock`, where any local user can create the path first; the clients never looked there. It now refuses to start unless `XDG_RUNTIME_DIR` or `socket_path` is set.                                                                                                         | Fixed, low (c9f1526)    |
| Socket permissions      | The socket was `srwxr-xr-x` (umask). Inside the `0700` runtime directory that was safe, but not for a custom path. It is now `0600`.                                                                                                                                                                                                  | Fixed, low (c9f1526)    |
| Unbounded request lines | A client sending data without a newline made the daemon buffer all of it: 300 MB grew it from 50 to 375 MB. Lines are limited to 64 KiB; the same client is now disconnected and RSS stays at 7 MB.                                                                                                                                   | Fixed, medium (1bc22a3) |
| Pixmap cache growth     | `get_pixmap` kept a copy of the icon per requested size (0–65535). 2000 sizes of a 256 px icon kept 500 MB (up to ~16 GB per item). The cache is gone; RSS now moves by 2 MB.                                                                                                                                                         | Fixed, medium (e1ccf44) |
| Hostile pixmaps         | A pixmap with `width = i32::MIN` panicked a debug build on overflow (no reply) and, in release, was served as 2147483648 × 4294967291 backed by 4 bytes: bars that size buffers from width × height would crash or allocate gigabytes. Pixmaps whose sides aren't positive or whose data isn't exactly `w × h × 4` bytes are dropped. | Fixed, medium (91e391c) |
| Signal flood            | 500 `NewIcon` signals in a burst froze the daemon's D-Bus connection for good (zbus backpressure, see §3). Any process on the bus could take the tray down this way.                                                                                                                                                                  | Fixed, high (eb8c806)   |
| Hung app                | An app that registered and never answered blocked every other registration (§3).                                                                                                                                                                                                                                                      | Fixed, high (80ac405)   |
| Ghost registrations     | Any process could register a name nobody owns, or `""`, and leave a permanent item in every bar. Rejected with `InvalidArgs` now.                                                                                                                                                                                                     | Fixed, medium (3c48831) |
| Watcher takeover        | zbus requests names with `REPLACE_EXISTING`/`ALLOW_REPLACEMENT` by default: any second host took the watcher name and, on exit, left it unowned (§3).                                                                                                                                                                                 | Fixed, high (830ca7e)   |
| Escape sequences        | A menu label with `ESC[31m` and a newline: tray-tui renders it inert (ratatui drops control characters); trayctl passed it raw to the menu program, which matters for terminal pickers such as `fzf`. It now replaces control characters. `trayctl items` / `subscribe` print JSON, where serde escapes them.                         | Fixed, low (74809ac)    |
| `--dmenu-cmd`           | Now runs through `sh -c`. The string is the user's own argument; labels only ever travel over stdin, so app-controlled text cannot reach the shell.                                                                                                                                                                                   | OK                      |
| Impersonation           | Any app can register any well-known name it owns and choose any title and icon. This is how the SNI protocol works (KDE's watcher behaves the same); names are checked to exist and to have an owner.                                                                                                                                 | Accepted                |
| IPC peer checks         | No credential check on connecting clients: the socket is `0600` in a `0700` directory, so only the same user can connect, who could equally talk to D-Bus directly.                                                                                                                                                                   | Accepted                |
| Clients trusting trayd  | trayctl and tray-tui read daemon replies without a line limit. The daemon runs as the same user.                                                                                                                                                                                                                                      | Accepted                |
| `unsafe`                | None in any crate; all four now `#![forbid(unsafe_code)]`.                                                                                                                                                                                                                                                                            | Fixed (1dbd954)         |
| Dependencies            | `cargo deny check`: advisories, bans, licenses, sources OK; one warn-only duplicate (`syn`). CI ran only the licence check; it now runs all four.                                                                                                                                                                                     | Fixed (171af76)         |

## 3. Bugs

| Area                     | Finding                                                                                                                                                                                                                                                                                                                                                                              | Severity | Status                                     |
| ------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | -------- | ------------------------------------------ |
| Stale properties         | Item proxies used zbus's property cache, which only `PropertiesChanged` refreshes; SNI apps send `NewTitle`, `NewIcon` and so on instead. After the first read, every change was ignored: a second `SetTitle` left the old title in `get_items` (and the fake app saw only 2 property reads in total). Titles, icons, tooltips, overlays and menu paths never updated.               | high     | Fixed (c7783e0)                            |
| Signal-burst deadlock    | The per-item task awaited property reads inside its signal loop. zbus never drops messages: with 64 unread signals the connection's reader waits, and so does the reply the loop waits for. After 500 `NewIcon`s, the original release could not register a new app and did not answer even a property read of the watcher. The loop now only records changes and wakes a refresher. | high     | Fixed (eb8c806)                            |
| Hung app blocks the tray | Registrations were handled one at a time and zbus has no default timeout. An app that never answered `GetAll` blocked every later registration and every removal: 227 s in the test, ended only by killing it. Registrations now run concurrently with a 10 s call timeout; a registration in flight is cancelled if its app leaves (no ghost from the race).                        | high     | Fixed (80ac405)                            |
| Watcher name stolen      | A second trayd (e.g. with another `--socket`) took `org.kde.StatusNotifierWatcher`; when it exited, nobody owned the name and the first trayd silently stopped getting apps. A second instance now exits with "another tray already owns org.kde.StatusNotifierWatcher". The previously `#[ignore]`d live tests would have done the same to the user's tray when run.                | high     | Fixed (830ca7e, 71b700b)                   |
| Ghost items              | A registration for a name with no owner stayed listed forever (removal waits for its owner to leave). `""` stayed in `RegisteredStatusNotifierItems`.                                                                                                                                                                                                                                | medium   | Fixed (3c48831)                            |
| Prefix unregistering     | When `org.audit.P` left, `org.audit.P2` was also removed from the watcher list and got a `StatusNotifierItemUnregistered` signal while still running (also `:1.2` vs `:1.20/…`).                                                                                                                                                                                                     | medium   | Fixed (f3c7b6a)                            |
| Leaked item task         | Replacing an item's watcher task dropped the old `JoinHandle`, which detaches instead of stopping it; reproduced as two "signal watcher started" for one app after the prefix bug.                                                                                                                                                                                                   | low      | Fixed (760bf62)                            |
| Item order               | `items()` returned hash order; adding the 4th and 8th app reshuffled the existing icons. Items now keep registration order.                                                                                                                                                                                                                                                          | medium   | Fixed (ff169b3)                            |
| Wrong menu row activated | trayctl matched the picked line by label: picking the second "Settings" clicked the first (`EVENT 6` instead of `7`). Lines are now unique and map back by position.                                                                                                                                                                                                                 | medium   | Fixed (74809ac)                            |
| Separators and disabled  | Separators came over IPC as blank rows; tray-tui made them selectable and Enter clicked them in the app (`EVENT 2 clicked`). Greyed-out rows were offered and activated by both clients. IPC rows now carry `separator` and `enabled`; both clients honour them.                                                                                                                     | medium   | Fixed (be04869, f4bf8ed, 74809ac, da28b99) |
| Lazy submenus            | `GetLayout` was called without `AboutToShow`, so submenus that apps fill on demand (common with Qt, Electron and libdbusmenu) came back empty.                                                                                                                                                                                                                                       | medium   | Fixed (be04869)                            |
| `--socket` for trayd     | Documented in the README, rejected by the CLI.                                                                                                                                                                                                                                                                                                                                       | low      | Fixed (43564af)                            |
| Pixmap size truncation   | `size` was cast to `u16`: 65558 was served as size 22. Clamped now.                                                                                                                                                                                                                                                                                                                  | low      | Fixed (1bc22a3)                            |
| Error codes              | Malformed JSON, unknown commands and bad versions were answered `NOT_IMPLEMENTED` and the connection was dropped; `v` was never checked. New `INVALID_REQUEST`; the connection stays usable after a well-framed bad line.                                                                                                                                                            | low      | Fixed (1bc22a3)                            |
| Pixmap tie               | The comment promised "prefer larger" on equal distance; the code kept the first. Fixed with the validation rewrite.                                                                                                                                                                                                                                                                  | low      | Fixed (91e391c)                            |
| Unquoted `--dmenu-cmd`   | The command was split on whitespace: `-p 'Tray menu'` became three arguments. A missing program was treated as a cancelled menu.                                                                                                                                                                                                                                                     | low      | Fixed (74809ac)                            |
| Stale subscriber tasks   | A `subscribe` client that disconnects was noticed at the next tray event, when the write failed; until then its task and broadcast receiver stayed (bounded, no growth). The server now watches the read side and drops it at once. | low      | Fixed (fda0a3f)                            |
| DBusMenu id cast         | `item_id` is sent as `u32` and cast back to `i32`; the cast round-trips, so negative DBusMenu ids still work.                                                                                                                                                                                                                                                                        | —        | OK                                         |

## 4. Performance

Release builds, bus with ~170 names, numbers from the runs above.

| Measurement                                     | Before                        | After                                       |
| ----------------------------------------------- | ----------------------------- | ------------------------------------------- |
| 60 `NewIcon` (30 Hz) from a path-registered app | 1442 calls (1380 PID lookups) | —                                           |
| 500-signal burst + 60 frames                    | connection frozen for good    | 186 `Get` calls, 10 ms CPU                  |
| Calls to register an app                        | 13 `Get` + name lookups       | 1 `GetAll` (+ subscriptions)                |
| Time until a hung app's item appears            | never (blocks everyone)       | 11 s (one timeout), others at once (122 ms) |
| RSS after 2000 `get_pixmap` sizes               | +500 MB, never freed          | +2 MB                                       |
| RSS under a 300 MB newline-less request         | 375 MB                        | 7 MB, client disconnected                   |
| trayd idle, 10 s, 3 items + 1 subscriber        | —                             | 0 ms CPU, 8.4 MB RSS                        |
| tray-tui idle, 10 s                             | —                             | 0 ms CPU, 5.0 MB RSS                        |

The PID-based well-known-name lookup (one `GetConnectionUnixProcessID` per
name on the bus) now runs once per registration instead of on every
`NewIcon` (eb8c806); the burst of 500 is coalesced into a handful of re-reads.
Subscribers still get one snapshot per 50 ms window of changes (unchanged).

## 5. UI usability and design

| Area             | Finding                                                                                                                                                                                          | Status                   |
| ---------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------ |
| tray-tui logs    | Reconnect warnings were printed to stdout across the UI. Logs go to stderr, are held back while the UI is up and printed on exit.                                                                | Fixed (642a661)          |
| Daemon gone      | tray-tui kept showing the last list with no hint. The status bar says "trayd unreachable, retrying…" until it is back.                                                                           | Fixed (642a661)          |
| Cursor           | The selection followed list positions and jumped to another app when one appeared above it; it now stays on the same app. An open menu closes, with a note, when its app leaves.                 | Fixed (642a661)          |
| Panic            | No panic hook: a panic left the terminal in raw mode. `ratatui::init` installs one.                                                                                                              | Fixed (642a661)          |
| Mnemonics        | Labels showed DBusMenu markers ("\_Open window", "Save \_\_as"). Stripped in libtrayd.                                                                                                           | Fixed (be04869)          |
| Menu rows        | tray-tui: separators are divider lines the cursor skips, disabled rows are dimmed and say so on Enter, toggles show `[x]` / `[ ]`. trayctl: the same rows hidden or marked, submenus end in `›`. | Fixed (da28b99, 74809ac) |
| Menu title       | tray-tui titled menus with the raw bus name (`:1.202/StatusNotifierItem`) and "(submenu)" for every level; it now shows the item title and a breadcrumb ("Fake App › More").                     | Fixed (da28b99)          |
| Log noise        | trayd logged to stdout and both trayd and trayctl wrote ANSI colours into files and journald. stderr, colour only on a terminal.                                                                 | Fixed (650616a)          |
| Error messages   | "another tray already owns …", "… exists and is not a socket", "XDG_RUNTIME_DIR is not set; set it or socket_path", "… command not found" replace silent failures.                               | Fixed                    |
| Narrow terminals | tray-tui at 70×10 and 60×14 renders correctly (pinned by `TestBackend` tests); long labels are cut by the list widget.                                                                           | OK                       |

## 6. Code quality, tests and project hygiene

| Area       | Finding                                                                                                                                                                                                                                                                  | Status          |
| ---------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | --------------- |
| Live tests | All D-Bus tests were `#[ignore]`d, and running them took the user's watcher name. They now each start a private `dbus-daemon` with a fake app (`libtrayd/tests/common`), run by default, and skip when `dbus-daemon` is missing; CI installs it.                         | Fixed (71b700b) |
| Coverage   | 10.6% → 56.8% lines (host 4.7% → 74.7%, IPC server 0% → 51.8%, codec 0% → 95.9%). 108 passing + 9 ignored tests → 146 passing, none ignored.                                                                                                                             | Fixed           |
| Gates      | fmt, typos, clippy `-D warnings` at three feature levels, tests, docs: all pass on every audit commit (checked one by one, §9). Two commits each failed once under parallel build load and passed on retry; the live tests' 5 s wait limit was raised to 30 s (b3c4c75). | OK              |
| Manifests  | Cargo warned about `homepage` repeating `repository` and an unused `workspace.package.description`; both removed.                                                                                                                                                        | Fixed (171af76) |
| Publishing | `cargo publish -p libtrayd --dry-run` packages fine; libtrayd 0.2.0 is already on crates.io. The binaries depend on libtrayd by path only, which is fine while only libtrayd is published.                                                                               | OK              |
| Semver     | `MenuNode` gained public fields (be04869): so the crates are bumped to 0.3.0. Other additions (`start_on`, `is_watcher_taken`, IPC fields) are additive.                                                                                                                 | Fixed           |
| Docs       | README, IPC.md, ARCHITECTURE and CHANGELOG updated for every behaviour change.                                                                                                                                                                                           | Fixed (b43b286) |

## 7. Features compared with the SNI specification and other hosts

| Feature                                          | trayd                                                           |
| ------------------------------------------------ | --------------------------------------------------------------- |
| Watcher + host, `RegisteredStatusNotifierItems`  | Yes                                                             |
| Registration by name, path, or `name/path`       | Yes                                                             |
| Title, status, category, tooltip text            | Yes (now kept up to date, c7783e0)                              |
| Icon name, pixmaps, attention icon, overlay name | Yes; overlay pixmaps over IPC since 6b45ff7                     |
| `IconThemePath` (apps shipping their own icons)  | Yes (115a9cb): sent as `icon_theme_path`; clients decide whether to search it |
| Activate                                         | Yes                                                             |
| SecondaryActivate (middle click), Scroll         | Yes (b7dfe6c)                                                   |
| ContextMenu for items without DBusMenu           | Yes (b7dfe6c)                                                   |
| DBusMenu: rows, submenus, `AboutToShow`          | Yes (`AboutToShow` since be04869)                               |
| DBusMenu: separators, disabled, check/radio      | Yes (since be04869, f4bf8ed)                                    |
| DBusMenu: row icons, shortcuts                   | Yes (bb83ea4)                                                   |
| DBusMenu: `LayoutUpdated`                        | No; clients re-fetch the menu each time it opens                |
| XEmbed (legacy X11 tray icons)                   | No (Wayland-only by design)                                     |

## 8. Left open

- **Release**: all crates are now 0.3.0 (libtrayd's `MenuNode` change is
  breaking); set the date in CHANGELOG.md when tagging. Nothing was published.
- **CI**: the `dbus-daemon` install step and the full `cargo deny check` job
  were not run on GitHub here; watch their first run.
- ~~**Features** (§7): `IconThemePath`, secondary activation and scroll over
  IPC, menu row icons, overlay pixmaps.~~ Done (b7dfe6c, 115a9cb, 6b45ff7,
  bb83ea4), along with `ContextMenu` and menu shortcuts. XEmbed stays out
  (Wayland-only by design), and so does `LayoutUpdated`: clients fetch a
  menu each time it opens.
- ~~**Subscriber cleanup** (§3)~~: done (fda0a3f).
- ~~**Cosmetic**: `RegisterStatusNotifierHost`'s argument shows up as
  `_service` in introspection.~~ Done (1e8e787).

## 9. Re-running the audit

```sh
# Gates (docs/ARCHITECTURE.md §8); live tests need dbus-daemon installed
cargo fmt --all -- --check
typos
cargo deny check
for f in --no-default-features "" --all-features; do
  cargo clippy --workspace --all-targets $f -- -D warnings
  cargo test --workspace $f
done
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features

# Coverage (system LLVM tools on Arch: LLVM_COV / LLVM_PROFDATA)
cargo llvm-cov --workspace --all-features --summary-only
```

Each audit commit on its own (from a scratch worktree):

```sh
for c in $(git rev-list --reverse d67b3ee..HEAD); do
  git checkout -q "$c" && cargo fmt --all -- --check &&
    cargo clippy --workspace --all-targets --all-features -- -D warnings &&
    cargo test --workspace --all-features || echo "FAILED: $c"
done
```

Manual checks, **always on a private bus** so the desktop's tray is never
touched:

```sh
dbus-daemon --session --fork --print-address=1 --print-pid=1   # export the address
DBUS_SESSION_BUS_ADDRESS=… trayd --socket "$XDG_RUNTIME_DIR/trayd-test.sock"
```

- [ ] A second `trayd --socket …-2.sock` on the same bus exits with "another
      tray already owns …", and the first still accepts registrations.
- [ ] An app that changes its title three times shows each title in
      `trayctl items`.
- [ ] An app emitting 500 `NewIcon` in a burst doesn't stop a new app from
      appearing within a second.
- [ ] An app that never answers property reads doesn't delay another app.
- [ ] `RegisterStatusNotifierItem "org.nobody.Here"` fails with `InvalidArgs`.
- [ ] Killing `org.example.P` leaves `org.example.P2` registered.
- [ ] Adding ten apps keeps their order in `trayctl items`.
- [ ] 2000 `get_pixmap` sizes and a 300 MB line without a newline leave RSS
      flat.
- [ ] `socket_path` pointing at a regular file stops trayd and keeps the file;
      the socket is `0600` and gone after `SIGTERM`.
- [ ] `trayctl menu` with two equal labels activates the one picked; separators
      and disabled rows aren't offered; `--dmenu-cmd "rofi -dmenu -p 'Tray'"`
      works.
- [ ] `RUST_LOG=debug tray-tui`: killing trayd shows "trayd unreachable" with
      nothing drawn over the UI; the held lines print on exit.
- [ ] Idle CPU over 10 s is ~0 ms for trayd and tray-tui.
