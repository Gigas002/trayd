# trayd IPC (v1)

> **Status:** specification complete; implementation tracked by phase (see [§ Implementation status](#implementation-status)).

---

## Table of contents

1. [Transport](#transport)
2. [Consumers](#consumers)
3. [Protocol overview](#protocol-overview)
4. [Methods](#methods)
   - [ping](#ping)
   - [list](#list)
   - [subscribe](#subscribe)
   - [get_pixmap](#get_pixmap)
   - [activate](#activate)
   - [secondary_activate](#secondary_activate)
   - [scroll](#scroll)
   - [menu_open](#menu_open)
   - [menu_select](#menu_select)
   - [menu_close](#menu_close)
5. [Schemas](#schemas)
   - [ItemInfo](#iteminfo)
   - [MenuNodeInfo](#menunodeinfo)
6. [Error codes](#error-codes)
7. [Subscribe events](#subscribe-events)
8. [Implementation status](#implementation-status)
9. [Examples](#examples)

---

## Transport

| Property          | Value                                                             |
| ----------------- | ----------------------------------------------------------------- |
| Socket type       | Unix domain socket (SOCK_STREAM)                                  |
| Default path      | `$XDG_RUNTIME_DIR/trayd.sock`                                     |
| Override (config) | `socket` key in `trayd.toml`                                      |
| Override (CLI)    | `--socket <path>` flag                                            |
| Framing           | **NDJSON** — one JSON object per line (`\n`-terminated)           |
| Encoding          | UTF-8                                                             |
| Direction         | Client → daemon: request line. Daemon → client: response line(s). |

The socket path is resolved at daemon startup. A running daemon owns the socket; a second `trayd run` will fail if the socket file already exists and is live. Stale (dead) socket files are removed on startup.

### Framing rules

- Every message is a single JSON object on a single line, terminated by `\n`.
- Lines must not exceed a reasonable implementation limit (suggested: 4 MiB for pixmap payloads).
- Clients must handle unexpected fields gracefully (forward-compatibility).
- After sending a request the client waits for exactly one response line, except for `subscribe` which streams events indefinitely until the connection closes.

---

## Consumers

External programs use this protocol directly against the socket or via **`trayd` CLI subcommands**. They do **not** link `libtrayd` or the `trayd` crate.

| Consumer           | Mode                                                                             |
| ------------------ | -------------------------------------------------------------------------------- |
| **abar**           | Spawns `trayd list` / `trayd subscribe`; talks socket for pixmaps and activation |
| **trayd-client**   | Connects to socket directly; implements NDJSON client in its own crate           |
| **Shell scripts**  | `socat`, `nc`, or any line-oriented tool; human-readable with `jq`               |
| **dmenu wrappers** | Invoke `trayd menu-dmenu --item <id>` CLI helper (see `docs/PLAN.md` §3.3)       |

> **Why not link `libtrayd`?** Per [abar issue #7](https://github.com/Gigas002/abar/issues/7): avoids version lock-in, keeps dependents minimal, and allows swapping `trayd` without rebuilding bars or TUIs.

---

## Protocol overview

Every message carries a version field `"v": 1`.

### Request (client → daemon)

```examples/ipc-examples/ping-request.json#L1
{"v":1,"method":"ping"}
```

Minimum required fields: `"v"` and `"method"`. Methods may require additional fields documented below.

### Success response (daemon → client)

```examples/ipc-examples/ping-response.json#L1
{"v":1,"result":{"type":"pong","version":"0.1.0"}}
```

Always an object with `"v"` and `"result"`. The `"result"` object always has a `"type"` discriminant.

### Error response (daemon → client)

```examples/ipc-examples/error-not-found.json#L1
{"v":1,"error":{"code":"NOT_FOUND","message":"item not found: org.example.missing"}}
```

Always an object with `"v"` and `"error"`. The `"error"` object has a stable `"code"` string and a human-readable `"message"`. See [§ Error codes](#error-codes).

### Subscribe event (daemon → client, streaming)

```examples/ipc-examples/event-item-added.json#L1
{"v":1,"event":{"type":"item_added","item":{"id":"org.kde.plasma.nm","title":"Network Manager","status":"active","has_attention_icon":false}}}
```

Events are sent asynchronously after a successful `subscribe` response. The connection stays open until the client disconnects.

---

## Methods

### `ping`

Health-check and version probe.

**Request**

```examples/ipc-examples/ping-request.json#L1
{"v":1,"method":"ping"}
```

**Success response**

```examples/ipc-examples/ping-response.json#L1
{"v":1,"result":{"type":"pong","version":"0.1.0"}}
```

| Field            | Type     | Description                    |
| ---------------- | -------- | ------------------------------ |
| `result.type`    | `"pong"` | Fixed discriminant             |
| `result.version` | string   | Daemon semver (e.g. `"0.1.0"`) |

**Error codes:** none expected; any socket-level failure means the daemon is not running.

---

### `list`

Returns a snapshot of all currently registered tray items.

**Request**

```examples/ipc-examples/list-request.json#L1
{"v":1,"method":"list"}
```

**Success response (empty)**

```examples/ipc-examples/list-response-empty.json#L1
{"v":1,"result":{"type":"list","items":[]}}
```

**Success response (with items)**

```examples/ipc-examples/list-response.json#L1
{"v":1,"result":{"type":"list","items":[{"id":"org.kde.plasma.nm","title":"Network Manager","status":"active","has_attention_icon":false}]}}
```

| Field          | Type         | Description                 |
| -------------- | ------------ | --------------------------- |
| `result.type`  | `"list"`     | Fixed discriminant          |
| `result.items` | `ItemInfo[]` | Ordered array; may be empty |

See [§ ItemInfo](#iteminfo) for the item schema.

**Error codes:** `INTERNAL_ERROR` if the host is unavailable.

---

### `subscribe`

Begins a long-lived event stream. The daemon sends one acknowledgement response, then streams `event` objects until the client disconnects.

**Request**

```examples/ipc-examples/subscribe-request.json#L1
{"v":1,"method":"subscribe"}
```

**Acknowledgement response**

```examples/ipc-examples/subscribed-response.json#L1
{"v":1,"result":{"type":"subscribed"}}
```

**Subsequent stream** — see [§ Subscribe events](#subscribe-events) for event shapes.

```examples/ipc-examples/event-item-added.json#L1
{"v":1,"event":{"type":"item_added","item":{"id":"org.kde.plasma.nm","title":"Network Manager","status":"active","has_attention_icon":false}}}
```

```examples/ipc-examples/event-item-removed.json#L1
{"v":1,"event":{"type":"item_removed","id":"org.kde.plasma.nm"}}
```

| Field         | Type           | Description                  |
| ------------- | -------------- | ---------------------------- |
| `result.type` | `"subscribed"` | Acknowledgement discriminant |

After the acknowledgement, the client should not send further requests on the same connection. Close the connection to end the subscription.

**Error codes:** `INTERNAL_ERROR` if the daemon cannot attach a subscriber.

---

### `get_pixmap`

Fetches a rendered icon for a tray item at a requested size.

**Request**

| Field     | Type           | Required | Description                                       |
| --------- | -------------- | -------- | ------------------------------------------------- |
| `method`  | `"get_pixmap"` | yes      |                                                   |
| `item_id` | string         | yes      | Stable item identifier                            |
| `size`    | integer        | yes      | Requested icon size in logical pixels (e.g. `22`) |

**Example request**

```/dev/null/get_pixmap-request.json#L1
{"v":1,"method":"get_pixmap","item_id":"org.kde.plasma.nm","size":22}
```

**Success response**

| Field            | Type                  | Description               |
| ---------------- | --------------------- | ------------------------- |
| `result.type`    | `"pixmap"`            | Fixed discriminant        |
| `result.item_id` | string                | Echoed item identifier    |
| `result.format`  | `"argb32"` \| `"png"` | Pixel data encoding       |
| `result.width`   | integer               | Actual pixel width        |
| `result.height`  | integer               | Actual pixel height       |
| `result.data`    | string                | Base64-encoded pixel data |

**Example success response**

```/dev/null/get_pixmap-response.json#L1
{"v":1,"result":{"type":"pixmap","item_id":"org.kde.plasma.nm","format":"png","width":22,"height":22,"data":"<base64>"}}
```

**Error codes:** `NOT_FOUND`, `BUS_FAILED`, `INTERNAL_ERROR`.

---

### `activate`

Triggers the primary activation action on a tray item (equivalent to left-click).

**Request**

| Field     | Type         | Required | Description            |
| --------- | ------------ | -------- | ---------------------- |
| `method`  | `"activate"` | yes      |                        |
| `item_id` | string       | yes      | Stable item identifier |

**Example request**

```examples/ipc-examples/activate-request.json#L1
{"v":1,"method":"activate","item_id":"org.kde.plasma.nm"}
```

**Success response**

```examples/ipc-examples/ok-response.json#L1
{"v":1,"result":{"type":"ok"}}
```

**Error codes:** `NOT_FOUND`, `BUS_FAILED`, `INTERNAL_ERROR`.

---

### `secondary_activate`

Triggers the secondary activation action (equivalent to middle-click). Not all items support this; the daemon forwards the call regardless and returns `ok` if the D-Bus call succeeded.

**Request**

| Field     | Type                   | Required | Description            |
| --------- | ---------------------- | -------- | ---------------------- |
| `method`  | `"secondary_activate"` | yes      |                        |
| `item_id` | string                 | yes      | Stable item identifier |

**Example request**

```/dev/null/secondary_activate-request.json#L1
{"v":1,"method":"secondary_activate","item_id":"org.kde.plasma.nm"}
```

**Success response** — same shape as `activate`:

```examples/ipc-examples/ok-response.json#L1
{"v":1,"result":{"type":"ok"}}
```

**Error codes:** `NOT_FOUND`, `BUS_FAILED`, `INTERNAL_ERROR`.

---

### `scroll`

Sends a scroll event to a tray item.

**Request**

| Field       | Type                                        | Required | Description                               |
| ----------- | ------------------------------------------- | -------- | ----------------------------------------- |
| `method`    | `"scroll"`                                  | yes      |                                           |
| `item_id`   | string                                      | yes      | Stable item identifier                    |
| `direction` | `"up"` \| `"down"` \| `"left"` \| `"right"` | yes      | Scroll axis and sign                      |
| `delta`     | integer                                     | yes      | Number of steps (positive, typically `1`) |

**Example request**

```/dev/null/scroll-request.json#L1
{"v":1,"method":"scroll","item_id":"org.kde.plasma.nm","direction":"up","delta":1}
```

**Success response**

```examples/ipc-examples/ok-response.json#L1
{"v":1,"result":{"type":"ok"}}
```

**Error codes:** `NOT_FOUND`, `BUS_FAILED`, `INVALID_REQUEST` (unknown direction), `INTERNAL_ERROR`.

---

### `menu_open`

Opens a menu session for a tray item and returns a snapshot of the top-level menu nodes.

**Request**

| Field     | Type          | Required | Description            |
| --------- | ------------- | -------- | ---------------------- |
| `method`  | `"menu_open"` | yes      |                        |
| `item_id` | string        | yes      | Stable item identifier |

**Example request**

```/dev/null/menu_open-request.json#L1
{"v":1,"method":"menu_open","item_id":"org.kde.plasma.nm"}
```

**Success response**

| Field               | Type             | Description                                                 |
| ------------------- | ---------------- | ----------------------------------------------------------- |
| `result.type`       | `"menu_opened"`  | Fixed discriminant                                          |
| `result.session_id` | string           | Opaque session handle; pass to `menu_select` / `menu_close` |
| `result.item_id`    | string           | Echoed item identifier                                      |
| `result.nodes`      | `MenuNodeInfo[]` | Top-level menu entries                                      |

**Example success response**

```/dev/null/menu_open-response.json#L1
{"v":1,"result":{"type":"menu_opened","session_id":"s-1","item_id":"org.kde.plasma.nm","nodes":[{"id":"n-1","label":"Enable Networking","enabled":true,"visible":true,"is_separator":false},{"id":"n-2","label":"Connection Editor","enabled":true,"visible":true,"is_separator":false},{"id":"n-sep","enabled":false,"visible":true,"is_separator":true}]}}
```

See [§ MenuNodeInfo](#menunodeinfo) for the node schema.

**Error codes:** `NOT_FOUND`, `BUS_FAILED`, `INTERNAL_ERROR`.

---

### `menu_select`

Selects a node in an open menu session. For leaf nodes the action is triggered and `ok` is returned. For nodes with `children_display: "submenu"` a new `menu_opened` payload is returned describing the submenu.

**Request**

| Field        | Type            | Required | Description                                              |
| ------------ | --------------- | -------- | -------------------------------------------------------- |
| `method`     | `"menu_select"` | yes      |                                                          |
| `session_id` | string          | yes      | Session handle from `menu_open` or a prior `menu_select` |
| `node_id`    | string          | yes      | Node identifier from the `nodes` array                   |

**Example request**

```/dev/null/menu_select-request.json#L1
{"v":1,"method":"menu_select","session_id":"s-1","node_id":"n-1"}
```

**Success response — leaf node**

```examples/ipc-examples/ok-response.json#L1
{"v":1,"result":{"type":"ok"}}
```

**Success response — submenu node**

```/dev/null/menu_select-submenu-response.json#L1
{"v":1,"result":{"type":"menu_opened","session_id":"s-1","item_id":"org.kde.plasma.nm","nodes":[{"id":"n-3","label":"VPN: office","enabled":true,"visible":true,"is_separator":false}]}}
```

The `session_id` is reused for the nested level; pass it unchanged to subsequent `menu_select` or `menu_close` calls.

**Error codes:** `INVALID_SESSION`, `NOT_FOUND`, `BUS_FAILED`, `INTERNAL_ERROR`.

---

### `menu_close`

Ends a menu session and releases any associated resources.

**Request**

| Field        | Type           | Required | Description             |
| ------------ | -------------- | -------- | ----------------------- |
| `method`     | `"menu_close"` | yes      |                         |
| `session_id` | string         | yes      | Session handle to close |

**Example request**

```/dev/null/menu_close-request.json#L1
{"v":1,"method":"menu_close","session_id":"s-1"}
```

**Success response**

```examples/ipc-examples/ok-response.json#L1
{"v":1,"result":{"type":"ok"}}
```

**Error codes:** `INVALID_SESSION`, `INTERNAL_ERROR`.

---

## Schemas

### ItemInfo

Represents a registered tray item. Present in `list` responses, `item_added` events, and `item_updated` events.

| Field                | Type                                             | Required | Description                                                                                             |
| -------------------- | ------------------------------------------------ | -------- | ------------------------------------------------------------------------------------------------------- |
| `id`                 | string                                           | yes      | Stable, unique item identifier (typically the D-Bus service name, e.g. `"org.kde.plasma.nm"`)           |
| `title`              | string                                           | no       | Human-readable display name; may be absent if the application did not set one                           |
| `status`             | `"passive"` \| `"active"` \| `"needs_attention"` | yes      | Item visibility hint: `passive` = hidden by default, `active` = normal, `needs_attention` = urgent      |
| `has_attention_icon` | bool                                             | yes      | `true` if the item has a separate attention icon to display in `needs_attention` state                  |
| `tooltip`            | string                                           | no       | Tooltip text; may be absent                                                                             |
| `category`           | string                                           | no       | Application category hint (e.g. `"ApplicationStatus"`, `"SystemServices"`, `"Hardware"`); may be absent |

**Example**

```/dev/null/item-info-example.json#L1
{"id":"org.kde.plasma.nm","title":"Network Manager","status":"active","has_attention_icon":false}
```

---

### MenuNodeInfo

Represents one node in a menu tree snapshot. Present in `menu_opened` results.

| Field              | Type   | Required | Description                                                                                                       |
| ------------------ | ------ | -------- | ----------------------------------------------------------------------------------------------------------------- |
| `id`               | string | yes      | Node identifier, unique within the session snapshot                                                               |
| `label`            | string | no       | Display text; absent for separators                                                                               |
| `enabled`          | bool   | yes      | `false` if the item is greyed out and cannot be selected                                                          |
| `visible`          | bool   | yes      | `false` if the item is hidden (should not be shown in the UI)                                                     |
| `is_separator`     | bool   | yes      | `true` if this is a visual separator rather than an actionable item                                               |
| `children_display` | string | no       | When present and equal to `"submenu"`, selecting this node returns a nested `menu_opened` payload instead of `ok` |

**Example (normal leaf)**

```/dev/null/menu-node-leaf.json#L1
{"id":"n-1","label":"Enable Networking","enabled":true,"visible":true,"is_separator":false}
```

**Example (submenu parent)**

```/dev/null/menu-node-submenu.json#L1
{"id":"n-vpn","label":"VPN Connections","enabled":true,"visible":true,"is_separator":false,"children_display":"submenu"}
```

**Example (separator)**

```/dev/null/menu-node-sep.json#L1
{"id":"n-sep","enabled":false,"visible":true,"is_separator":true}
```

---

## Error codes

All error responses have the shape `{ "v": 1, "error": { "code": "<CODE>", "message": "<human text>" } }`.

The `code` field is a stable, machine-readable string. The `message` field is informational only and may change between versions.

| Code              | Meaning                                                                         |
| ----------------- | ------------------------------------------------------------------------------- |
| `NOT_FOUND`       | The requested `item_id` is not registered (item may have disappeared)           |
| `BUS_FAILED`      | The D-Bus call to the tray application failed or timed out                      |
| `INVALID_SESSION` | The `session_id` is unknown or has already been closed                          |
| `INVALID_REQUEST` | The request is malformed, missing required fields, or contains an invalid value |
| `INTERNAL_ERROR`  | An unexpected internal error occurred in the daemon                             |

**Example**

```examples/ipc-examples/error-not-found.json#L1
{"v":1,"error":{"code":"NOT_FOUND","message":"item not found: org.example.missing"}}
```

---

## Subscribe events

After a successful `subscribe` acknowledgement the daemon pushes events as they occur. Each event is a JSON object with `"v": 1` and an `"event"` object containing a `"type"` discriminant.

### `item_added`

A new tray item has registered.

| Field        | Type           | Description                                    |
| ------------ | -------------- | ---------------------------------------------- |
| `event.type` | `"item_added"` | Discriminant                                   |
| `event.item` | `ItemInfo`     | Full item snapshot at the time of registration |

```examples/ipc-examples/event-item-added.json#L1
{"v":1,"event":{"type":"item_added","item":{"id":"org.kde.plasma.nm","title":"Network Manager","status":"active","has_attention_icon":false}}}
```

---

### `item_removed`

A tray item has unregistered.

| Field        | Type             | Description                         |
| ------------ | ---------------- | ----------------------------------- |
| `event.type` | `"item_removed"` | Discriminant                        |
| `event.id`   | string           | The stable `id` of the removed item |

```examples/ipc-examples/event-item-removed.json#L1
{"v":1,"event":{"type":"item_removed","id":"org.kde.plasma.nm"}}
```

---

### `item_updated`

A tray item's properties have changed (title, status, icon, tooltip, etc.).

| Field        | Type             | Description                  |
| ------------ | ---------------- | ---------------------------- |
| `event.type` | `"item_updated"` | Discriminant                 |
| `event.item` | `ItemInfo`       | Full refreshed item snapshot |

**Example**

```/dev/null/event-item-updated.json#L1
{"v":1,"event":{"type":"item_updated","item":{"id":"org.kde.plasma.nm","title":"Network Manager","status":"needs_attention","has_attention_icon":true}}}
```

---

### `menu_changed`

The menu tree for an item has changed. Clients that have an open session for this item should call `menu_open` again to get a fresh snapshot, or close the session.

| Field           | Type             | Description                                    |
| --------------- | ---------------- | ---------------------------------------------- |
| `event.type`    | `"menu_changed"` | Discriminant                                   |
| `event.item_id` | string           | The stable `id` of the item whose menu changed |

**Example**

```/dev/null/event-menu-changed.json#L1
{"v":1,"event":{"type":"menu_changed","item_id":"org.kde.plasma.nm"}}
```

---

## Implementation status

| Phase       | Scope                                                                                                                        | Status      |
| ----------- | ---------------------------------------------------------------------------------------------------------------------------- | ----------- |
| **Phase 0** | Workspace scaffold; `docs/IPC.md` stub                                                                                       | ✅ complete |
| **Phase 1** | IPC skeleton: `trayd::ipc::protocol` wire types, NDJSON codec, Unix socket server/client, mock handler, golden fixture tests | 🔲 planned  |
| **Phase 2** | Real D-Bus SNI host (`libtrayd`), daemon wiring, `trayd list` / `trayd subscribe` / `trayd activate`                         | 🔲 planned  |
| **Phase 3** | DBusMenu integration (`libtrayd`), `menu_open` / `menu_select` / `menu_close` IPC, `trayd menu-dmenu` CLI                    | 🔲 planned  |

See `docs/PLAN.md` §8 for the full phase breakdown.

---

## Examples

Golden request/response fixtures live under `examples/ipc-examples/`. They are used by integration tests in Phase 1 to verify serialisation round-trips.

| File                       | Description                   |
| -------------------------- | ----------------------------- |
| `ping-request.json`        | `ping` request                |
| `ping-response.json`       | `ping` success response       |
| `list-request.json`        | `list` request                |
| `list-response-empty.json` | `list` response with no items |
| `list-response.json`       | `list` response with one item |
| `activate-request.json`    | `activate` request            |
| `ok-response.json`         | Generic `ok` success response |
| `error-not-found.json`     | `NOT_FOUND` error response    |
| `subscribe-request.json`   | `subscribe` request           |
| `subscribed-response.json` | `subscribe` acknowledgement   |
| `event-item-added.json`    | `item_added` event envelope   |
| `event-item-removed.json`  | `item_removed` event envelope |

### Quick test with `socat`

```/dev/null/socat-example.sh#L1-3
# Health-check
echo '{"v":1,"method":"ping"}' | socat - UNIX-CONNECT:/run/user/1000/trayd.sock
# List items
echo '{"v":1,"method":"list"}' | socat - UNIX-CONNECT:/run/user/1000/trayd.sock
```
