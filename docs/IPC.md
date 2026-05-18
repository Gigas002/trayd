# trayd IPC (v1) — draft

> **Status:** stub for Phase 0. The full protocol is implemented in **Phase 1** (`docs/PLAN.md` §8).

## Transport

- Unix domain socket, default: `$XDG_RUNTIME_DIR/trayd.sock`
- Framing: newline-delimited JSON (NDJSON)
- Every request includes `"v": 1`

## Consumers

External programs (**abar**, **trayd-client**, shell, dmenu wrappers) implement this protocol against the socket and/or invoke **`trayd` CLI** subcommands. They do **not** link `libtrayd` or the `trayd` crate.

## Methods (planned)

See `docs/PLAN.md` §3.2: `ping`, `list`, `subscribe`, `get_pixmap`, `activate`, `secondary_activate`, `scroll`, `menu_open`, `menu_select`, `menu_close`.

## Examples

Golden request/response fixtures will live under `examples/ipc-examples/` in Phase 1.
