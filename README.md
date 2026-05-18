# trayd

Minimal Wayland-session system tray daemon (`zbus`) with a documented IPC socket for bars and other clients.

## Workspace

| Crate | Role |
| ----- | ---- |
| `libtrayd` | D-Bus tray host (library) |
| `trayd` | Daemon + CLI + IPC server |
| `trayd-client` | Terminal UI (socket IPC only) |

See [`docs/PLAN.md`](docs/PLAN.md) and [`docs/IPC.md`](docs/IPC.md).

```sh
cargo build -p trayd -p trayd-client -p libtrayd
cargo test --workspace
```
