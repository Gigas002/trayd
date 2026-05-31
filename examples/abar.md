# abar integration

[abar](https://github.com/Gigas002/abar) is the primary bar consumer for trayd.
It connects to the IPC socket only — it does not link `libtrayd`.

For abar-side configuration see the [abar repository](https://github.com/Gigas002/abar).

---

## IPC flow

```
client startup
  │── ping ──────────────────────────────────────► trayd
  │◄── pong ──────────────────────────────────────
  │
  │── subscribe ──────────────────────────────────► trayd
  │◄── event{update, items=[...]} ────────────────   (initial snapshot)
  │◄── event{update, items=[...]} ────────────────   (on any change)
  │   ...persistent stream...
  │
  │  (for each MinimalTrayItem, render icon segment)
  │── get_pixmap{app_id, size} ───────────────────► trayd
  │◄── pixmap{width, height, data(base64 ARGB32)} ─
  │
  │  (on item click — primary activation)
  │── activate{app_id, item_id=0} ───────────────► trayd
  │◄── ack ────────────────────────────────────────
  │
  │  (on item click — open menu)
  │  spawn: trayctl menu --app-id <app_id>
```

---

## Startup sequence

1. Send `ping` to `$XDG_RUNTIME_DIR/trayd.sock`.
2. If no response, start the daemon: `trayd run &` (or via systemd user unit).
3. Open a persistent `subscribe` connection; repaint on every `event`.

---

## Optional systemd user unit

```ini
[Unit]
Description=trayd system tray daemon
PartOf=graphical-session.target

[Service]
ExecStart=%h/.cargo/bin/trayd run
Restart=on-failure

[Install]
WantedBy=graphical-session.target
```

Install to `~/.config/systemd/user/trayd.service`, then:

```sh
systemctl --user enable --now trayd
```
