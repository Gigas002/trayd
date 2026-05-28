# trayd IPC v1

## Transport

- **Socket:** Unix domain socket, default `$XDG_RUNTIME_DIR/trayd.sock` (overridable in `trayd.toml`)
- **Framing:** newline-delimited JSON (NDJSON) — one JSON object per line
- **Version field:** every request and response carries `"v": 1`

## Consumers

**abar**, **trayctl**, **tray-tui**, and shell scripts talk to trayd via this socket only. They do **not** link `libtrayd` or the `trayd` crate; they duplicate wire types locally per this document.

---

## Requests

Every request is a JSON object on one line:

```
{"v":1,"cmd":"<command>"[, ...args]}
```

| `cmd`        | Extra fields                                   | Callers                       |
| ------------ | ---------------------------------------------- | ----------------------------- |
| `ping`       | —                                              | any                           |
| `subscribe`  | —                                              | abar, tray-tui                |
| `get_items`  | —                                              | trayctl, scripts              |
| `get_menu`   | `"app_id": string`, `"submenu_id": int\|null`  | trayctl, tray-tui             |
| `activate`   | `"app_id": string`, `"item_id": int`           | trayctl, tray-tui             |
| `get_pixmap` | `"app_id": string`, `"size": int`              | abar                          |

---

## Responses

### Success

```
{"v":1,"type":"<type>"[, ...fields]}
```

| `type`    | Extra fields                                         | Sent in reply to          |
| --------- | ---------------------------------------------------- | ------------------------- |
| `pong`    | —                                                    | `ping`                    |
| `items`   | `"items": MinimalTrayItem[]`                         | `get_items`               |
| `event`   | `"event": TrayEvent`                                 | `subscribe` (stream)      |
| `menu`    | `"app_id": string`, `"items": MenuItem[]`            | `get_menu`                |
| `ack`     | —                                                    | `activate`                |
| `pixmap`  | `"app_id": string`, `"size": int`, `"data": string`  | `get_pixmap`              |

### Error

```
{"v":1,"error":{"code":"<CODE>","message":"..."}}
```

| `code`          | Meaning                                   |
| --------------- | ----------------------------------------- |
| `NOT_FOUND`     | `app_id` not registered                   |
| `BUS_FAILED`    | D-Bus communication failed                |
| `INVALID_APP_ID`| Malformed `app_id`                        |
| `NOT_IMPLEMENTED`| Feature not yet available               |

---

## Types

### `MinimalTrayItem`

```json
{
  "app_id": "org.example.App",
  "title": "Example App",
  "status": "active",
  "icon_handle": "example-app"
}
```

`title` and `icon_handle` are omitted when `null`.

| Field         | Type             | Notes                        |
| ------------- | ---------------- | ---------------------------- |
| `app_id`      | string           | stable SNI registration id   |
| `title`       | string \| absent | display name                 |
| `status`      | string           | `"active"`, `"passive"`, `"needs_attention"` |
| `icon_handle` | string \| absent | theme icon name or handle    |

### `MenuItem`

```json
{"item_id": 1, "label": "Action", "is_submenu": false}
```

| Field        | Type    | Notes                               |
| ------------ | ------- | ----------------------------------- |
| `item_id`    | integer | stable row id within this menu      |
| `label`      | string  | display text                        |
| `is_submenu` | bool    | `true` → has children; send `get_menu` with this `item_id` as `submenu_id` |

### `TrayEvent`

```json
{"kind": "update", "items": [ ...MinimalTrayItem ]}
```

Currently only `"kind": "update"` exists; carries the full current item list.

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
{"v":1,"type":"items","items":[{"app_id":"org.example.App","title":"Example App","status":"active","icon_handle":"example-app"}]}
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
{"v":1,"type":"pixmap","app_id":"org.example.App","size":22,"data":""}
```

### Error

```
{"v":1,"cmd":"get_menu","app_id":"unknown.App","submenu_id":null}
{"v":1,"error":{"code":"NOT_FOUND","message":"app_id not registered"}}
```
