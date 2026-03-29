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
tests/                  Integration tests
```

## Data Flow

```
  Physical Device           Lua Config
       │                        │
       ▼                        ▼
  EvdevSource ──────┐    load_config()
       │            │         │
       ▼            │         ▼
  EvdevMonitor ─────┼──→  Engine ──→ InputBackend
       │            │      │  ▲         │
       │            │      │  │         ├─ UinputBackend ──→ /dev/uinput
       │            │      │  │         └─ LoggingBackend (dry-run)
       │            │      │  │
       ▼            │      ▼  │
  [button event] ───┘   IpcServer ◄──► wayclickctl / wayclick-tui
                           │
                        Unix Socket
```

## Module Responsibilities

### wayclick-core

- **config.rs** — Data model: `Config`, `TriggerBinding`, `ActionConfig`,
  `DeviceMatch`, `GlobalOptions`. Validation logic. Key code mapping.
- **lua_api.rs** — Sandboxed Lua VM. Loads `init.lua`, registers the
  `wayclick.*` API table. Parses Lua tables into `Config`.
- **engine.rs** — Trigger state machine (Idle → Active → Cooldown). Worker
  thread management. Action execution loops (auto_click, key_sequence, scroll,
  mouse_move, composite).
- **ipc.rs** — JSON-RPC 2.0 over Unix socket with 4-byte BE length framing.
  `IpcServer` accepts connections, `ipc_request()` is the client helper.
- **input_backend.rs** — Trait for emitting input events. `LoggingBackend` (dry
  run) and `MockBackend` (testing).
- **uinput_backend.rs** — Real backend: creates `/dev/uinput` virtual device,
  emits EV_KEY/EV_REL/EV_SYN sequences.
- **evdev_source.rs** — Trait for reading input events. `EvdevSource` reads real
  devices with poll(2). `MockSource` for testing.
- **evdev_monitor.rs** — Enumerates devices, matches against bindings, spawns
  per-device reader threads, handles hotplug.
- **config_watcher.rs** — Filesystem polling watcher for `*.lua` files.
  Notifies callback on change.
- **logger.rs** — Ring-buffer logger with log levels, JSON output mode, and
  thread-safe `AtomicBool` quiet flag.

### wayclickd

Single-binary daemon. Startup sequence:
1. Parse CLI args (clap)
2. Load Lua config
3. Initialize backend (UinputBackend or LoggingBackend)
4. Create Engine with triggers
5. Start IPC server on Unix socket
6. Start EvdevMonitor for device bindings
7. Start ConfigWatcher for hot-reload
8. Wait for SIGINT/SIGTERM

### wayclickctl

Thin CLI that sends JSON-RPC requests to the daemon socket. Subcommands: ping,
status, toggle, enable, disable, trigger, list, reload, logs.

### wayclick-tui

Ratatui-based TUI that polls the daemon via IPC. Four panels: triggers, trigger
detail, devices, and logs. Catppuccin Mocha color theme.

### wayclick-evdev-dump

Diagnostic tool with three subcommands:
- `list` — enumerate all accessible input devices
- `monitor --device <path>` — print raw events from a device
- `identify` — press a button to identify which device it is

## IPC Protocol

JSON-RPC 2.0 over a UNIX stream socket.

**Frame format:** `[4-byte big-endian length][UTF-8 JSON payload]`

**Methods:** `ping`, `status`, `status_json`, `toggle`, `enable`, `disable`,
`trigger`, `list_triggers`, `reload_config`, `logs_tail`

## Threading Model

- **Main thread** — signal handling, shutdown coordination
- **IPC server thread** — non-blocking accept loop, per-connection handling
- **Per-trigger worker threads** — spawned/stopped by Engine on trigger events
- **Per-device reader threads** — spawned by EvdevMonitor
- **Config watcher thread** — polls filesystem for changes
- **Hotplug scan thread** — periodic device enumeration
