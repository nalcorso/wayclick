# wayclick IPC Protocol

wayclick exposes a Unix domain socket that accepts JSON-RPC 2.0 framed messages.
External programs (game plugins, scripts, GUI frontends, etc.) can use it to:

- Query status and trigger lists
- Enable/disable and reload config at runtime
- **Subscribe to push events** (trigger fired, layer changed, enabled/disabled, config reloaded)
- **Register and unregister triggers dynamically** — owned by the connection, cleaned up automatically on disconnect

---

## Connection

The socket path is configured in `wayclickd` (default `${XDG_RUNTIME_DIR}/wayclick.sock` or `/run/wayclick.sock`).

```sh
# Quick test
echo '{"jsonrpc":"2.0","id":1,"method":"ping","params":{}}' | \
  wayclick-ctl raw -
```

---

## Framing

Each message is length-prefixed:

```
[ 4 bytes big-endian uint32 ] [ N bytes UTF-8 JSON ]
```

Python helper:

```python
import socket, struct, json

def send_frame(sock, obj):
    body = json.dumps(obj).encode()
    sock.sendall(struct.pack(">I", len(body)) + body)

def recv_frame(sock):
    length = struct.unpack(">I", sock.recv(4, socket.MSG_WAITALL))[0]
    return json.loads(sock.recv(length, socket.MSG_WAITALL))
```

---

## Request / Response

Standard JSON-RPC 2.0 — requests carry a numeric `id`, responses echo it:

```json
→ {"jsonrpc":"2.0","id":1,"method":"ping","params":{}}
← {"jsonrpc":"2.0","id":1,"result":"pong"}
```

Errors use the standard error object:

```json
← {"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"reason"}}
```

---

## Push Notifications (Events)

After calling `subscribe`, the server sends unsolicited **notification frames** on the
same connection alongside normal responses.  Notifications have `"id": null` and
`"method": "event"`:

```json
{"jsonrpc":"2.0","id":null,"method":"event","params":{"type":"trigger_activated","trigger_id":"rapid_fire","timestamp_ms":1745760000000}}
{"jsonrpc":"2.0","id":null,"method":"event","params":{"type":"layer_changed","from":"base","to":"combat","timestamp_ms":1745760001000}}
{"jsonrpc":"2.0","id":null,"method":"event","params":{"type":"enabled_changed","enabled":false,"timestamp_ms":1745760002000}}
{"jsonrpc":"2.0","id":null,"method":"event","params":{"type":"config_reloaded","timestamp_ms":1745760003000}}
```

Clients distinguish notifications from responses by checking `msg.get("id") == null`.

### Event types

| `type`                 | Extra fields |
|------------------------|-------------|
| `trigger_activated`    | `trigger_id`, `timestamp_ms` |
| `trigger_deactivated`  | `trigger_id`, `timestamp_ms` |
| `layer_changed`        | `from`, `to`, `timestamp_ms` |
| `enabled_changed`      | `enabled`, `timestamp_ms` |
| `config_reloaded`      | `timestamp_ms` |

---

## Methods

### ping

```json
→ {"jsonrpc":"2.0","id":1,"method":"ping","params":{}}
← {"jsonrpc":"2.0","id":1,"result":"pong"}
```

### status

Returns daemon state, active layer, trigger count, and version.

```json
→ {"jsonrpc":"2.0","id":2,"method":"status","params":{}}
← {"jsonrpc":"2.0","id":2,"result":{
    "enabled": true,
    "layer": "base",
    "trigger_count": 3,
    "active_trigger_count": 1,
    "version": "0.1.0"
  }}
```

### enable / disable / toggle

```json
→ {"jsonrpc":"2.0","id":3,"method":"enable","params":{}}
← {"jsonrpc":"2.0","id":3,"result":{"enabled":true}}
```

### set_layer

```json
→ {"jsonrpc":"2.0","id":4,"method":"set_layer","params":{"layer":"combat"}}
← {"jsonrpc":"2.0","id":4,"result":{"layer":"combat"}}
```

### list_triggers

Returns all triggers (static + dynamic from all connections).
Each entry now includes `activate_count`, `last_activated_ms`, and `user_enabled`.

```json
→ {"jsonrpc":"2.0","id":5,"method":"list_triggers","params":{}}
← {"jsonrpc":"2.0","id":5,"result":[
    {"id":"rapid_fire","name":"Rapid Fire","mode":"toggle","action_type":"auto_click","active":false,"dynamic":false,"activate_count":47,"last_activated_ms":1745760005000,"user_enabled":true},
    {"id":"plugin_trigger","name":"Plugin Trigger","mode":"oneshot","action_type":"keystroke","active":false,"dynamic":true,"activate_count":0,"last_activated_ms":null,"user_enabled":true}
  ]}
```

### list_layers

Returns all layer names defined in the current config.

```json
→ {"jsonrpc":"2.0","id":6,"method":"list_layers","params":{}}
← {"jsonrpc":"2.0","id":6,"result":{"layers":["base","combat","menu"],"current":"base"}}
```

### trigger

Fire a trigger by ID. For toggle triggers, `press: true` activates and
`press: false` deactivates.

```json
→ {"jsonrpc":"2.0","id":7,"method":"trigger","params":{"id":"rapid_fire","press":true}}
← {"jsonrpc":"2.0","id":7,"result":{"triggered":"rapid_fire"}}
```

### reload_config

Reload the Lua config from disk. All running trigger workers are stopped first.
The active layer is preserved.

```json
→ {"jsonrpc":"2.0","id":8,"method":"reload_config","params":{}}
← {"jsonrpc":"2.0","id":8,"result":{"reloaded":true}}
```

### check_config

Validate a Lua config file without applying it. Returns the list of validation
errors, or an empty list if the config is valid.

```json
→ {"jsonrpc":"2.0","id":9,"method":"check_config","params":{"path":"/home/user/.config/wayclick/new.lua"}}
← {"jsonrpc":"2.0","id":9,"result":{"valid":true,"errors":[]}}
```

```json
← {"jsonrpc":"2.0","id":9,"result":{"valid":false,"errors":["interval_ms 0 is below minimum 1"]}}
```


### enable_trigger / disable_trigger

Enable or disable an individual trigger without affecting others.
The state survives config reload but is reset on daemon restart.

```json
→ {"jsonrpc":"2.0","id":10,"method":"enable_trigger","params":{"id":"rapid_fire"}}
← {"jsonrpc":"2.0","id":10,"result":{"enabled":"rapid_fire"}}

→ {"jsonrpc":"2.0","id":11,"method":"disable_trigger","params":{"id":"rapid_fire"}}
← {"jsonrpc":"2.0","id":11,"result":{"disabled":"rapid_fire"}}
```

The `user_enabled` field in `list_triggers` responses reflects the current state.
A disabled trigger shows `"user_enabled": false` and will not activate even if its
button is pressed.

### logs

Tail recent log entries from the ring buffer.

```json
→ {"jsonrpc":"2.0","id":10,"method":"logs","params":{"tail":20}}
← {"jsonrpc":"2.0","id":10,"result":[
    {"level":"info","message":"trigger rapid_fire activated","timestamp_ms":1745760000000},
    {"level":"info","message":"trigger rapid_fire deactivated","timestamp_ms":1745760005000}
  ]}
```

---

## Error codes

| Code | Meaning |
|---|---|
| `-32600` | Invalid JSON-RPC request (malformed frame, missing fields) |
| `-32601` | Method not found |
| `-32602` | Invalid parameters (validation error, duplicate trigger ID, unknown trigger ID) |
| `-32000` | Internal engine error (unexpected failure) |

---



Start receiving push events on this connection.  Omit `events` (or pass `null`) to
receive all event types.

```json
→ {"jsonrpc":"2.0","id":1,"method":"subscribe","params":{}}
← {"jsonrpc":"2.0","id":1,"result":{"subscribed":true,"events":"all"}}

→ {"jsonrpc":"2.0","id":2,"method":"subscribe","params":{"events":["trigger_activated","trigger_deactivated"]}}
← {"jsonrpc":"2.0","id":2,"result":{"subscribed":true,"events":["trigger_activated","trigger_deactivated"]}}
```

Calling `subscribe` again replaces the existing subscription.

### unsubscribe

Stop receiving push events while keeping the connection open.

```json
→ {"jsonrpc":"2.0","id":3,"method":"unsubscribe","params":{}}
← {"jsonrpc":"2.0","id":3,"result":{"subscribed":false}}
```

### register_trigger

Register a dynamic trigger owned by this connection.  The trigger is automatically
removed when the connection closes (cleanly or abruptly).

The `action` object uses the same schema as `ActionConfig` in Lua config files.
Optional numeric fields (`jitter_ms`, `hold_ms`, `duration_ms`) default to `0`/`null`
if omitted.

```json
→ {"jsonrpc":"2.0","id":4,"method":"register_trigger","params":{
    "id": "rapid_fire",
    "name": "Rapid Fire",
    "mode": "toggle",
    "action": {
      "type": "auto_click",
      "button": "left",
      "interval_ms": 50
    }
  }}
← {"jsonrpc":"2.0","id":4,"result":{"registered":"rapid_fire"}}
```

Sequence action example (hideout macro):

```json
→ {"jsonrpc":"2.0","id":5,"method":"register_trigger","params":{
    "id": "hideout_macro",
    "mode": "oneshot",
    "action": {
      "type": "composite",
      "mode": "sequence",
      "actions": [
        {"type": "keystroke", "key_name": "KEY_ENTER", "key_code": 28},
        {"type": "delay", "duration_ms": 100},
        {"type": "keystroke", "key_name": "KEY_SLASH", "key_code": 53},
        {"type": "keystroke", "key_name": "KEY_H",     "key_code": 35},
        {"type": "delay", "duration_ms": 100},
        {"type": "keystroke", "key_name": "KEY_ENTER", "key_code": 28}
      ]
    }
  }}
← {"jsonrpc":"2.0","id":5,"result":{"registered":"hideout_macro"}}
```

**Errors:**

```json
← {"jsonrpc":"2.0","id":4,"error":{"code":-32602,"message":"Trigger ID 'rapid_fire' already exists"}}
← {"jsonrpc":"2.0","id":5,"error":{"code":-32602,"message":"Validation error: interval_ms 0 is below minimum 1"}}
```

Trigger IDs must not conflict with static (Lua-defined) triggers or dynamic triggers
owned by other connections.

### unregister_trigger

Remove a dynamic trigger.  Only the owning connection may unregister a trigger.

```json
→ {"jsonrpc":"2.0","id":6,"method":"unregister_trigger","params":{"id":"rapid_fire"}}
← {"jsonrpc":"2.0","id":6,"result":{"unregistered":"rapid_fire"}}
```

```json
← {"jsonrpc":"2.0","id":6,"error":{"code":-32000,"message":"Trigger 'rapid_fire' not found or not owned by this connection"}}
```

### list_dynamic_triggers

List only the dynamic triggers registered by this connection.

```json
→ {"jsonrpc":"2.0","id":7,"method":"list_dynamic_triggers","params":{}}
← {"jsonrpc":"2.0","id":7,"result":[
    {"id":"rapid_fire","name":"Rapid Fire","mode":"toggle","action_type":"auto_click","active":true,"dynamic":true},
    {"id":"hideout_macro","name":"hideout_macro","mode":"oneshot","action_type":"sequence","active":false,"dynamic":true}
  ]}
```

---

## Lifecycle Safety

Dynamic triggers are **owned by the connection** that registered them.  No matter how
a client disconnects (clean close, crash, network drop), wayclick will:

1. Stop any active workers for those triggers
2. Remove them from the trigger table
3. Unsubscribe them from the event bus

This prevents a misbehaving client from leaving wayclick in a polluted state.

---

## Full Example — Game Plugin

```python
import socket, struct, json, threading

SOCK_PATH = "/run/user/1000/wayclick.sock"

def send_frame(sock, obj):
    body = json.dumps(obj).encode()
    sock.sendall(struct.pack(">I", len(body)) + body)

def recv_frame(sock):
    n = struct.unpack(">I", sock.recv(4, socket.MSG_WAITALL))[0]
    return json.loads(sock.recv(n, socket.MSG_WAITALL))

sock = socket.socket(socket.AF_UNIX)
sock.connect(SOCK_PATH)

# Register a dynamic trigger owned by this connection
send_frame(sock, {"jsonrpc":"2.0","id":1,"method":"register_trigger","params":{
    "id": "rapid_fire", "mode": "toggle",
    "action": {"type": "auto_click", "button": "left", "interval_ms": 50}
}})
print(recv_frame(sock))  # → {"result": {"registered": "rapid_fire"}}

# Subscribe to events on the same connection
send_frame(sock, {"jsonrpc":"2.0","id":2,"method":"subscribe","params":{}})
print(recv_frame(sock))  # → {"result": {"subscribed": true, "events": "all"}}

# Background thread reads both push events and command responses
def reader():
    while True:
        msg = recv_frame(sock)
        if msg.get("method") == "event":
            evt = msg["params"]
            if evt["type"] == "trigger_activated":
                print(f"[EVENT] {evt['trigger_id']} fired")
        elif "result" in msg:
            print(f"[RESP] id={msg['id']} → {msg['result']}")

threading.Thread(target=reader, daemon=True).start()

# Fire the dynamic trigger manually
send_frame(sock, {"jsonrpc":"2.0","id":3,"method":"trigger","params":{"id":"rapid_fire","press":True}})

# On sock.close() (or process exit), rapid_fire is automatically cleaned up
```

---

## Concurrency Notes

- A single connection handles both commands and push events concurrently.  There is no
  ordering guarantee between a response and a notification triggered by the same
  command on a different connection.
- The event bus channel is bounded (64 items per subscriber).  A slow client that
  cannot keep up with events will have excess events silently dropped rather than
  blocking the engine.
- IPC connections are limited to `MAX_IPC_CONNECTIONS` (default 32).
