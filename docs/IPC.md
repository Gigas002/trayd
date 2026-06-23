# trayd IPC v1

## Transport

- **Socket:** Unix domain socket, default `$XDG_RUNTIME_DIR/trayd.sock` (overridable in `trayd.toml`)
- **Framing:** newline-delimited JSON (NDJSON) — one JSON object per line
- **Version field:** every request and response carries `"v": 1`

## Consumers

Any process can connect to `$XDG_RUNTIME_DIR/trayd.sock` and speak this protocol — bars, TUIs, scripts, or custom tools. No dependency on `libtrayd` is required; the wire types are small enough to duplicate locally. `libtrayd` is also available as a standalone library if you want to embed the SNI host directly rather than talk to a running daemon.

---

## Requests

Every request is a JSON object on one line:

```
{"v":1,"cmd":"<command>"[, ...args]}
```

| `cmd`        | Extra fields                                  | Typical callers                   |
| ------------ | --------------------------------------------- | --------------------------------- |
| `ping`       | —                                             | any                               |
| `subscribe`  | —                                             | persistent consumers (bars, TUIs) |
| `get_items`  | —                                             | one-shot consumers, scripts       |
| `get_menu`   | `"app_id": string`, `"submenu_id": int\|null` | menu consumers                    |
| `activate`   | `"app_id": string`, `"item_id": int`          | menu consumers                    |
| `get_pixmap` | `"app_id": string`, `"size": int`             | icon-rendering consumers          |

---

## Responses

### Success

```
{"v":1,"type":"<type>"[, ...fields]}
```

| `type`   | Extra fields                                                                         | Sent in reply to     |
| -------- | ------------------------------------------------------------------------------------ | -------------------- |
| `pong`   | —                                                                                    | `ping`               |
| `items`  | `"items": MinimalTrayItem[]`                                                         | `get_items`          |
| `event`  | `"event": TrayEvent`                                                                 | `subscribe` (stream) |
| `menu`   | `"app_id": string`, `"items": MenuItem[]`                                            | `get_menu`           |
| `ack`    | —                                                                                    | `activate`           |
| `pixmap` | `"app_id": string`, `"size": int`, `"width": int`, `"height": int`, `"data": string` | `get_pixmap`         |

### Error

```
{"v":1,"error":{"code":"<CODE>","message":"..."}}
```

| `code`            | Meaning                    |
| ----------------- | -------------------------- |
| `NOT_FOUND`       | `app_id` not registered    |
| `BUS_FAILED`      | D-Bus communication failed |
| `INVALID_APP_ID`  | Malformed `app_id`         |
| `NOT_IMPLEMENTED` | Feature not yet available  |

---

## Types

### `MinimalTrayItem`

```json
{
  "app_id": "org.example.App",
  "title": "Example App",
  "status": "Active",
  "icon_handle": "example-app",
  "category": "ApplicationStatus",
  "tooltip_title": "Example App"
}
```

`title`, `icon_handle`, `category`, `tooltip_title`, `tooltip_description`, and `overlay_icon_handle` are omitted when absent.
`item_is_menu` is omitted when `false`.

| Field                 | Type             | Notes                                                                                     |
| --------------------- | ---------------- | ----------------------------------------------------------------------------------------- |
| `app_id`              | string           | stable SNI registration id                                                                |
| `title`               | string \| absent | display name                                                                              |
| `status`              | string           | `"Active"`, `"Passive"`, `"NeedsAttention"`                                               |
| `icon_handle`         | string \| absent | theme icon name or handle                                                                 |
| `category`            | string \| absent | SNI category: `"ApplicationStatus"`, `"Communications"`, `"SystemServices"`, `"Hardware"` |
| `item_is_menu`        | bool \| absent   | `true` when the item is a pure menu — **do not call `activate` with `item_id=0`**         |
| `tooltip_title`       | string \| absent | tooltip title text                                                                        |
| `tooltip_description` | string \| absent | tooltip description text                                                                  |
| `overlay_icon_handle` | string \| absent | overlay icon theme name (badge shown on top of the normal icon)                           |

### `MenuItem`

```json
{ "item_id": 1, "label": "Action", "is_submenu": false }
```

| Field        | Type    | Notes                                                                      |
| ------------ | ------- | -------------------------------------------------------------------------- |
| `item_id`    | integer | stable row id within this menu                                             |
| `label`      | string  | display text                                                               |
| `is_submenu` | bool    | `true` → has children; send `get_menu` with this `item_id` as `submenu_id` |

### `TrayEvent`

```json
{"kind": "update", "items": [ ...MinimalTrayItem ]}
```

Currently only `"kind": "update"` exists; carries the full current item list.

### `PixmapResponse`

Returned by `get_pixmap`.

```json
{
  "app_id": "org.example.App",
  "size": 22,
  "width": 22,
  "height": 22,
  "data": "<base64>"
}
```

| Field    | Type    | Notes                                                                                      |
| -------- | ------- | ------------------------------------------------------------------------------------------ |
| `app_id` | string  | echoed from the request                                                                    |
| `size`   | integer | requested size (pixels)                                                                    |
| `width`  | integer | actual pixel width of the returned surface                                                 |
| `height` | integer | actual pixel height of the returned surface                                                |
| `data`   | string  | base64-encoded ARGB32 bytes (`width × height × 4`) in big-endian byte order (per SNI spec) |

---

## `subscribe` stream

`subscribe` keeps the connection open. After the initial `event` response (full snapshot), the daemon pushes subsequent `event` lines whenever the tray state changes. The consumer reads until EOF.

```
→ {"v":1,"cmd":"subscribe"}
← {"v":1,"type":"event","event":{"kind":"update","items":[...]}}
← {"v":1,"type":"event","event":{"kind":"update","items":[...]}}
   ... (daemon pushes on every change)
```

---

## Examples

Golden request/response pairs live under `examples/ipc-examples/*.jsonl` — first line is request, second is response.

### `ping`

```
{"v":1,"cmd":"ping"}
{"v":1,"type":"pong"}
```

### `get_items`

```
{"v":1,"cmd":"get_items"}
{"v":1,"type":"items","items":[{"app_id":"org.example.App","title":"Example App","status":"Active","icon_handle":"example-app","category":"ApplicationStatus","tooltip_title":"Example App"}]}
```

### `get_menu` (top-level)

```
{"v":1,"cmd":"get_menu","app_id":"org.example.App","submenu_id":null}
{"v":1,"type":"menu","app_id":"org.example.App","items":[{"item_id":1,"label":"Action","is_submenu":false},{"item_id":2,"label":"Submenu","is_submenu":true}]}
```

### `get_menu` (submenu)

```
{"v":1,"cmd":"get_menu","app_id":"org.example.App","submenu_id":2}
{"v":1,"type":"menu","app_id":"org.example.App","items":[{"item_id":10,"label":"Sub Item 1","is_submenu":false}]}
```

### `activate`

```
{"v":1,"cmd":"activate","app_id":"org.example.App","item_id":1}
{"v":1,"type":"ack"}
```

### `get_pixmap`

```
{"v":1,"cmd":"get_pixmap","app_id":"org.example.App","size":22}
{"v":1,"type":"pixmap","app_id":"org.example.App","size":22,"width":22,"height":22,"data":""}
```

### Error

```
{"v":1,"cmd":"get_menu","app_id":"unknown.App","submenu_id":null}
{"v":1,"error":{"code":"NOT_FOUND","message":"app_id not registered"}}
```
