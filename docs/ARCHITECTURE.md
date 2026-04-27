# Architecture

## Overview

Wayclick is a Rust workspace organized into six crates:

```
crates/
  wayclick-core/        Core library (config, engine, IPC, backends)
  wayclickd/            Daemon binary
  wayclickctl/          CLI control tool
  wayclick-tui/         Terminal UI dashboard
  wayclick-evdev-dump/  Device diagnostic utility
tests/                  Integration and E2E tests
```

## Data Flow

```
  Physical device            Lua config
        │                        │
        ▼                        ▼
   EvdevSource ──────┐     load_config()
        │            │          │
        ▼            │          ▼
   EvdevMonitor ─────┼───→  Engine ──→ InputBackend
        │            │       │  ▲          │
        │            │       │  │          ├─ UinputBackend ──→ /dev/uinput
        │            │       │  │          └─ LoggingBackend (dry-run)
        │            │       │  │
        ▼            │       ▼  │
  [button event] ────┘    IpcServer ◄──► wayclickctl / wayclick-tui
                               │
                           Unix socket
```

## Module Responsibilities

### wayclick-core

- **config.rs** — Data model: `Config`, `TriggerBinding`, `ActionConfig`,
  `DeviceMatch`, `GlobalOptions`. Validation logic (`validate_config`). Key code
  mapping. Constants (`MAX_INTERVAL_MS`, `MAX_ACTION_DEPTH`, etc.).
- **lua_api.rs** — Sandboxed Lua VM. Loads `init.lua`, registers the
  `wayclick.*` API table. Parses Lua tables into `Config`. Enforces the config
  sandbox (removes `os.execute`, `io.popen`, restricts `io.open`).
- **engine.rs** — Trigger state machine (Idle → Active → Cooldown). Worker
  thread management. Action execution loops (`auto_click`, `key_press`, scroll,
  mouse_move, composite). The engine mutex is intentionally fail-fast
  (`.lock().unwrap()`) — a corrupted engine is worse than a clean crash.
  All callers go through `with_engine_events()` which releases the lock before
  publishing events to the event bus, eliminating the ABBA deadlock class.
- **event_bus.rs** — Bounded pub/sub channels for push notifications. Each IPC
  subscriber gets a 64-item channel; slow consumers drop events rather than
  blocking the engine.
- **mutex_ext.rs** — `MutexExt` trait providing `lock_or_recover()` for
  peripheral mutexes (logger ring buffer, event bus subscribers, device tracker,
  uinput file handle). Unlike the engine mutex, these structures recover from
  poison rather than propagating the panic.
- **ipc.rs** — JSON-RPC 2.0 over Unix socket with 4-byte big-endian length
  framing. `IpcServer` accepts connections (max 32). Each connection thread
  handles commands and delivers push events from its subscription channel.
- **input_backend.rs** — `InputBackend` trait for emitting input events.
  `LoggingBackend` (dry run) and `MockBackend` (testing).
- **uinput_backend.rs** — Real backend: creates a `/dev/uinput` virtual device,
  emits `EV_KEY` / `EV_REL` / `EV_SYN` sequences.
- **evdev_source.rs** — `EvdevSource` trait. `EvdevSource` reads real devices
  with `poll(2)`. `MockSource` for testing.
- **evdev_monitor.rs** — Enumerates physical devices, matches against bindings,
  spawns per-device reader threads, handles hotplug (2-second scan interval).
  Fires triggers by calling the engine through `with_engine_events()`.
- **config_watcher.rs** — Filesystem polling watcher for `*.lua` files.
  Notifies the daemon on change.
- **logger.rs** — Thread-safe ring buffer logger. Log levels `trace`, `debug`,
  `info`, `warn`, `error`. Accessible via IPC `logs` method.

### wayclickd

Single-binary daemon. Startup sequence:
1. Parse CLI args (clap): `--config`, `--dry-run`, `--enable`, `--check-config`,
   `--check-permissions`
2. Load and validate Lua config
3. Initialize `InputBackend` (UinputBackend or LoggingBackend)
4. Create `Engine` with loaded triggers
5. Start `IpcServer` on the Unix socket
6. Start `EvdevMonitor` for device bindings
7. Start `ConfigWatcher` for hot-reload
8. Wait for `SIGINT` / `SIGTERM`; handle `SIGHUP` for reload

### wayclickctl

Thin CLI that sends JSON-RPC requests to the daemon socket. Subcommands:
`ping`, `status`, `toggle`, `enable`, `disable`, `trigger`, `list`, `reload`,
`logs`, `layer get/set`, `waybar`.

### wayclick-tui

Ratatui-based TUI. Polls the daemon via IPC. Four panels: trigger list, trigger
detail, device list, and log stream. Catppuccin Mocha color theme.

### wayclick-evdev-dump

Diagnostic tool. Three subcommands:
- `list` — enumerate all accessible input devices
- `monitor --device <path>` — print raw events from a device
- `identify` — press any button to identify the device and code

## IPC Protocol

JSON-RPC 2.0 over a Unix stream socket.

**Frame format:** `[4-byte big-endian uint32 length][UTF-8 JSON payload]`

**Methods:** `ping`, `status`, `toggle`, `enable`, `disable`, `set_layer`,
`trigger`, `list_triggers`, `list_layers`, `reload_config`, `logs`,
`check_config`, `subscribe`, `unsubscribe`, `register_trigger`,
`unregister_trigger`, `list_dynamic_triggers`

See [IPC.md](IPC.md) for the full protocol reference.

## Threading Model

- **Main thread** — signal handling (`SIGINT`/`SIGTERM`/`SIGHUP`), startup and
  shutdown coordination
- **IPC server thread** — non-blocking accept loop; spawns a connection thread
  per client
- **Per-connection threads** — handle JSON-RPC requests, deliver push events
  from the subscription channel
- **Per-trigger worker threads** — spawned/stopped by `Engine` on trigger
  activation/deactivation (one thread per active toggle/hold trigger)
- **Per-device reader threads** — spawned by `EvdevMonitor` for each matched
  physical device
- **Config watcher thread** — polls filesystem every 2 seconds for `*.lua`
  changes
- **Hotplug scan thread** — periodic device enumeration every 2 seconds

## Event Publishing

The `Engine` collects events into a local `Vec` while holding its mutex, then
releases the mutex before publishing to the `EventBus`. This prevents the ABBA
deadlock that would occur if a subscriber thread tried to re-acquire the engine
mutex from within a channel send.

```rust
// Pattern: collect → release lock → publish
fn with_engine_events<F>(engine: &Mutex<Engine>, f: F)
where F: FnOnce(&mut Engine)
{
    let events = {
        let mut eng = engine.lock().unwrap(); // fail-fast: engine corruption is unrecoverable
        f(&mut eng);
        eng.drain_pending_events()
    }; // lock released here
    for event in events {
        event_bus.publish(event); // safe: no lock held
    }
}
```

