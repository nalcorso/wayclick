# Wayclick — Complete AI Recreation Prompt

> **Purpose:** This document contains everything an AI coding agent needs to recreate the
> `wayclick` project from a blank slate. It includes a feature matrix against competing tools,
> detailed design requirements informed by lessons learned in the prototype codebase, full
> module specifications, Lua API surface, IPC protocol, DevOps/tooling strategy, and a
> complete test plan.

---

## 1. Project Summary

**wayclick** is a programmable mouse-automation daemon for Linux. It:

- Reads physical button presses from `/dev/input/event*` devices using the kernel evdev
  interface (no X11 or Wayland dependency — works at the input-subsystem level).
- Executes configurable *trigger actions* (auto-click loops, key-press loops, composite
  sequences/parallel batches) via the kernel `uinput` facility.
- Exposes a Lua configuration API (Neovim-style `init.lua` + `lua/` modules, hot-reloaded
  on save) so users can script arbitrarily complex automation.
- Ships a long-running daemon (`wayclickd`), a control CLI (`wayclickctl`), and a TUI
  (`wayclick-tui`), all communicating over a UNIX-domain socket.
- Identifies input devices by stable attributes (vendor/product ID, device name substring)
  rather than fragile `/dev/input/eventN` indices.
- Monitors for device hotplug events so configuration survives device reconnects.

**Target platform:** Linux kernel ≥ 5.15 (evdev + uinput), any compositor (Wayland or X11).

**Implementation language:** Rust (preferred for memory safety, strong error handling, and
excellent tooling) **or** C99 (acceptable if Rust toolchain unavailability is a hard
constraint). This document uses Rust terminology; C equivalents are noted where they diverge.

---

## 2. Feature Matrix — Competing Tools

The table below compares wayclick against the most relevant existing tools. Gaps identified
here directly inform design requirements in §3.

| Feature | **wayclick** (target) | xdotool | ydotool | AutoKey | evemu | xbindkeys | pynput | AutoHotkey (Win) |
|---|---|---|---|---|---|---|---|---|
| **Language/Scripting** | Lua 5.4 | CLI/shell | CLI/shell | Python 3 | CLI | S-expr config | Python API | AHK DSL |
| **Compositor dependency** | None (evdev) | X11 required | None (uinput) | X11/AT-SPI | None (kernel) | X11 | X11/uinput | Windows |
| **Wayland compatible** | ✅ Full | ❌ | ✅ Full | ⚠️ Partial (AT-SPI) | ✅ Full | ❌ | ⚠️ Partial | N/A |
| **Daemon mode** | ✅ | ❌ | ❌ | ✅ | ❌ | ✅ | ❌ | ✅ |
| **Per-device targeting** | ✅ By ID | ❌ Global | ❌ Global | ❌ Global | ⚠️ Path only | ❌ Global | ❌ Global | ⚠️ Driver-level |
| **Stable device identity** | ✅ Name/VID:PID | N/A | N/A | N/A | ❌ Path fragile | N/A | N/A | N/A |
| **Hotplug monitoring** | ✅ udev netlink | N/A | N/A | N/A | ❌ | N/A | N/A | N/A |
| **Auto-click loop** | ✅ | ⚠️ Shell loops | ⚠️ Shell loops | ✅ | ✅ Record/replay | ❌ | ✅ | ✅ |
| **Jitter/randomisation** | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ |
| **Composite actions** | ✅ Parallel+Seq | ❌ | ❌ | ✅ | ❌ | ❌ | ✅ | ✅ |
| **Key-press automation** | ✅ | ✅ | ✅ | ✅ | ✅ | ❌ | ✅ | ✅ |
| **Scroll wheel control** | ✅ (target) | ✅ | ✅ | ✅ | ✅ | ❌ | ✅ | ✅ |
| **Relative mouse move** | ✅ (target) | ✅ | ✅ | ✅ | ✅ | ❌ | ✅ | ✅ |
| **Toggle & Hold modes** | ✅ | ❌ | ❌ | ⚠️ | ❌ | ❌ | ❌ | ✅ |
| **Hot reload config** | ✅ | N/A | N/A | ⚠️ Restart | N/A | ❌ | N/A | ⚠️ |
| **IPC control** | ✅ UNIX socket | N/A | N/A | ✅ D-Bus | N/A | ❌ | N/A | COM |
| **CLI control** | ✅ | N/A | N/A | ✅ | N/A | ❌ | N/A | ✅ |
| **TUI** | ✅ ncurses | ❌ | ❌ | ✅ GTK GUI | ❌ | ❌ | ❌ | ❌ |
| **Per-app profiles** | ✅ (roadmap) | N/A | N/A | ✅ | N/A | N/A | N/A | ✅ |
| **udev integration** | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | N/A |
| **systemd service** | ✅ | ❌ | ❌ | ✅ | ❌ | ❌ | ❌ | N/A |
| **Dry-run/test mode** | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| **Security hardening** | ✅ privilege sep | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| **Actively maintained** | 🔨 This project | ✅ | ✅ | ⚠️ Slow | ✅ libevdev | ⚠️ Slow | ✅ | ✅ |

### Key Gaps Addressed by wayclick

1. **No existing Linux tool combines programmable Lua scripting with kernel-level (evdev)
   input reading that is also Wayland-native.** xdotool/AutoKey need X11. ydotool uses uinput
   for output but has no scripting. AutoKey has powerful Python scripting but is X11-bound.

2. **Device identity fragility.** Every evdev-based tool (evemu, custom scripts) hardcodes
   `/dev/input/eventN` paths which change when devices are reconnected or the kernel reorders
   them. wayclick must solve this with stable identity matching.

3. **No existing open-source Linux autoclicker has jitter/randomisation.** This is essential
   for realistic automation (game anti-cheat, accessibility use cases).

4. **No existing tool provides composite parallel/sequential action pipelines with a
   user-friendly scripting language.**

5. **Daemon lifecycle management** (hot reload, IPC, systemd integration, graceful shutdown)
   is missing from every evdev-class tool.

---

## 3. Design Requirements

### 3.1 Implementation Language

**Primary: Rust (stable toolchain, 2021 edition)**

Rationale:
- Memory safety without GC — critical for a daemon reading kernel events.
- `Result<T, E>` error handling is natural for the many I/O failure modes (device not found,
  permission denied, uinput full, etc.).
- First-class FFI for Lua C API via `mlua` crate (feature = `lua54`, `vendored` or system).
- `tokio` or `std::thread` for concurrent device monitoring — use `std::thread` to keep the
  binary small and avoid async complexity.
- `cargo clippy`, `cargo fmt`, `cargo audit`, `cargo-fuzz` integrate into CI naturally.
- Cross-compilation to aarch64 (Raspberry Pi, ARM SBCs) is straightforward.

**Alternative: C99 (if Rust is unavailable)**
- Use CMake + Ninja.
- Embed Lua 5.4 C API directly.
- Use `libevdev` for evdev parsing.
- Use `pthread` for threading.
- Must pass Clang Static Analyzer and `cppcheck`.

The rest of this document uses Rust but marks C deviations with `[C: ...]`.

### 3.2 Architectural Principles

1. **Privilege separation**: The daemon requires two privileges: `CAP_INPUT` (reading
   `/dev/input/event*`) and write access to `/dev/uinput`. These should be the *only*
   elevated permissions. All configuration parsing, IPC handling, and Lua execution runs
   at the user level.

2. **Modular InputBackend trait/interface**: All synthetic input emission goes through an
   abstract `InputBackend` trait. Concrete implementations:
   - `LoggingBackend` — dry-run, logs what it would emit (default, no privileges needed).
   - `UinputBackend` — real events via `/dev/uinput`.
   - `MockBackend` — records all calls, for tests.
   Future backends: Wayland virtual keyboard protocol, D-Bus portal.

3. **Modular InputSource trait/interface**: All physical input reading goes through an
   abstract `InputSource` trait. Concrete implementations:
   - `EvdevSource` — reads `/dev/input/event*`, identifies device by stable attributes.
   - `IpcSource` — trigger events injected via the IPC socket (no device needed).
   - `MockSource` — injects synthetic events for tests.

4. **Stable device identity**: Devices MUST be identified by one or more of:
   - Vendor ID + Product ID (from `EVIOCGID` ioctl).
   - Device name substring match (from `EVIOCGNAME`).
   - Physical location string (from `EVIOCGPHYS`).
   - udev symlink under `/dev/input/by-id/` or `/dev/input/by-path/`.
   Hard-coded `/dev/input/eventN` paths are **forbidden** in configuration — they are only
   permitted as an override for power users, with a prominent warning at startup.

5. **Hotplug monitoring**: Use udev netlink (via the `udev` crate or manual `PF_NETLINK`
   socket) to detect device add/remove events. When a configured device appears, open it and
   start monitoring. When it disappears, close it gracefully and wait.

6. **Lua as configuration DSL**: Lua is loaded with a restricted sandbox — no `os.execute`,
   `io.popen`, `require` outside the config directory, or network access. The `wayclick`
   global table exposes the full API. Config is hot-reloaded by polling file modification
   times on `init.lua` and all `lua/` subdirectory `*.lua` files.

7. **IPC protocol**: JSON-RPC 2.0 over a length-prefixed UNIX stream socket. Each message is
   `[4-byte big-endian length][JSON payload]`. This prevents framing bugs from the simple
   newline-delimited approach in the prototype.

8. **Thread model** (no async runtime):
   - Main thread: signal handler, shutdown coordination.
   - IPC server thread: accepts connections, one thread per connected client.
   - EvdevMonitor threads: one per open device, reads events in a poll loop.
   - Udev monitor thread: watches for hotplug events.
   - Engine worker threads: one per active toggle trigger (auto-click/key-press loops).
   - ConfigWatcher thread: polls file timestamps, triggers reload callback.

9. **Engine state machine**: Each `TriggerBinding` has runtime state:
   - `Idle` — not active.
   - `Active(JoinHandle)` — worker thread running, contains stop channel.
   - `Cooldown(Instant)` — debounce window, ignores duplicate presses.

### 3.3 Data Model

```
Config
├── GlobalOptions
│   ├── dry_run: bool
│   ├── socket_path: Option<String>
│   ├── log_capacity: usize
│   └── min_interval_ms: u32          (safety floor, default 1ms)
├── triggers: Vec<TriggerBinding>
│   ├── id: String                    (unique, snake_case)
│   ├── name: String
│   ├── description: String
│   ├── mode: TriggerMode             (Toggle | Hold | OneShot)
│   └── action: ActionConfig
│       ├── AutoClick { button, interval_ms, duration_ms, jitter_ms }
│       ├── KeyPress  { key_name, key_code, interval_ms, duration_ms, jitter_ms }
│       ├── ScrollWheel { direction, amount, interval_ms, duration_ms, jitter_ms }
│       ├── MouseMove { dx, dy, interval_ms, duration_ms, jitter_ms }
│       ├── Composite { mode: Parallel|Sequence, actions: Vec<ActionConfig> }
│       └── NoOp
└── device_bindings: Vec<DeviceBinding>
    ├── DeviceMatch                   (see §3.4)
    ├── button_bindings: Vec<ButtonBinding>
    │   ├── code: EventCode           (BTN_LEFT, BTN_SIDE, etc.)
    │   └── trigger_id: String
    └── exclusive: bool               (grab device, prevent events reaching other programs)
```

### 3.4 Device Matching (Critical Design Decision)

```
DeviceMatch (enum, at least one field required):
├── ByPath(String)             -- e.g. "/dev/input/by-id/usb-Logitech..."  WARN if used
├── ByName { contains: String }   -- e.g. "Logitech G Pro"
├── ByVidPid { vendor: u16, product: u16 }  -- e.g. 0x046d, 0xc08b
├── ByPhys { contains: String }   -- e.g. "usb-0000:00:14.0-1"
└── Any(Vec<DeviceMatch>)         -- logical OR across multiple matchers
```

At daemon startup and on hotplug, the EvdevMonitor enumerates all `/dev/input/event*`
devices, reads their attributes (name, VID/PID, phys), and matches them against configured
`DeviceMatch` rules. Multiple physical devices can match the same config binding (useful for
setups with two identical mice). Each matched device gets its own monitoring thread.

### 3.5 TriggerMode Semantics

- **Toggle**: First event starts the action worker; second event stops it. Supports optional
  cooldown (`cooldown_ms`) to prevent accidental double-toggles.
- **Hold**: Action starts on button-press event and stops on button-release. Requires
  press/release tracking in the EvdevMonitor.
- **OneShot**: Executes the action once synchronously (no worker thread), then returns.
  Useful for macros and single-click bursts with fixed `duration_ms`.

---

## 4. Module Breakdown

### 4.1 `wayclick-core` library crate

All shared types, config parsing, engine logic, input abstractions.

#### `core/config.rs`
- `struct Config`, `struct GlobalOptions`, `struct TriggerBinding`, `struct DeviceBinding`
- `enum ActionConfig`, `struct AutoClickConfig`, `struct KeyPressConfig`,
  `struct ScrollConfig`, `struct MouseMoveConfig`, `struct CompositeConfig`
- `enum TriggerMode { Toggle, Hold, OneShot }`
- `enum DeviceMatch { ByPath, ByName, ByVidPid, ByPhys, Any }`
- `fn load_config(path: &Path, logger: &Logger) -> Result<Config, ConfigError>`
  - Creates a Lua VM, registers the `wayclick` global table with all API functions
    as closures (upvalue = `&mut ConfigBuilder`).
  - Executes `init.lua`.
  - Validates: all trigger IDs unique, all `trigger_id` refs in device bindings exist,
    intervals ≥ `min_interval_ms`, no duplicate device bindings.
  - Returns `Err(ConfigError)` with file path + line number on validation failure.
- `fn validate_config(config: &Config) -> Result<(), Vec<ConfigError>>`
- Helper: `fn normalize_key_name(raw: &str) -> Result<(String, u32), ConfigError>`
  Uses `evdev::Key::from_str` (the `evdev` Rust crate wraps linux/input-event-codes.h).

#### `core/lua_api.rs`
The Lua API surface exposed to `init.lua`. All functions are registered as closures with a
`ConfigBuilder` upvalue. The Lua global `wayclick` is a table; all public functions are
fields of that table.

```lua
-- Global options
wayclick.set_options({
  dry_run = false,           -- bool, default true
  socket_path = "",          -- string, "" = auto-detect from XDG_RUNTIME_DIR
  log_capacity = 512,        -- integer
  min_interval_ms = 1,       -- integer, safety floor
})

-- Register a trigger
wayclick.register_trigger({
  id = "rapid_fire",         -- required, unique string
  name = "Rapid Fire",       -- optional display name
  description = "...",       -- optional
  mode = "toggle",           -- "toggle" | "hold" | "oneshot"  (default "toggle")
  action = <ActionTable>,    -- required, one of the action constructors below
  cooldown_ms = 200,         -- optional debounce (ms) for toggle mode
})

-- Action constructors (return opaque tables consumed by register_trigger)
wayclick.auto_click({
  button = "left",           -- "left"|"right"|"middle"|"button4"|"button5"
  interval_ms = 50,          -- milliseconds between clicks
  duration_ms = nil,         -- optional total duration (nil = run forever)
  jitter_ms = 0,             -- random +/- variation per interval
})

wayclick.key_press({
  key = "space",             -- key name: single char, "KEY_SPACE", "space", etc.
  interval_ms = 1000,
  duration_ms = nil,
  jitter_ms = 0,
})

wayclick.scroll({
  direction = "down",        -- "up" | "down" | "left" | "right"
  amount = 3,                -- scroll clicks per event
  interval_ms = 100,
  duration_ms = nil,
  jitter_ms = 0,
})

wayclick.mouse_move({
  dx = 0,                    -- relative X in pixels per event
  dy = 5,                    -- relative Y in pixels per event
  interval_ms = 16,          -- ~60fps
  duration_ms = nil,
  jitter_ms = 0,
})

wayclick.sequence({          -- run actions one after another (blocking)
  actions = { ... },
})

wayclick.parallel({          -- run actions concurrently
  actions = { ... },
})

wayclick.noop()              -- placeholder, logs but does nothing

-- Device binding: map a physical button press to a trigger
wayclick.bind_device({
  -- At least one match criterion required:
  name = "Logitech G Pro",   -- substring match on device name
  vid = 0x046d,              -- vendor ID (integer)
  pid = 0xc08b,              -- product ID (integer)
  phys = "usb-0000:00:14",   -- substring match on physical path
  path = "/dev/input/by-id/usb-...",  -- explicit path (discouraged, emits warning)

  -- Bindings on this device:
  bindings = {
    { code = "BTN_SIDE",  trigger = "rapid_fire" },
    { code = "BTN_EXTRA", trigger = "key_matrix" },
  },

  exclusive = false,         -- if true, grab device (events won't reach other programs)
})

-- Legacy alias kept for compatibility (deprecated, emits warning):
wayclick.bind_evdev({
  device = "/dev/input/event15",  -- DEPRECATED path-based
  code = "BTN_SIDE",
  trigger = "rapid_fire",
})
```

**Lua sandbox restrictions** (enforced after loading standard libs):
- `os.execute`, `os.exit`, `io.popen`, `io.open` (write mode) are `nil`-ed out.
- `require` is replaced with a sandboxed version that only resolves paths relative to the
  config `lua/` directory.
- `load`, `loadfile`, `dofile` are disabled.
- `debug` table is removed.

#### `core/engine.rs`
- `struct Engine { config: Config, state: HashMap<TriggerId, TriggerState>, backend: Arc<dyn InputBackend>, logger: Logger }`
- `fn apply_config(&mut self, config: Config)` — stops all workers, updates config.
- `fn set_enabled(&mut self, enabled: bool)`
- `fn trigger_event(&mut self, id: &str, press: bool) -> Result<(), EngineError>`
  - For Toggle: first call starts worker (sends `stop_tx: Sender<()>` to `TriggerState`),
    second call sends stop signal and joins.
  - For Hold: `press=true` starts worker, `press=false` stops it.
  - For OneShot: executes action synchronously (or in a short-lived thread), ignores `press`.
  - Applies `cooldown_ms` debounce logic.
- `fn describe_status(&self) -> StatusReport`
- `fn triggers_snapshot(&self) -> Vec<TriggerSnapshot>`

Worker thread pseudocode for auto-click loop:
```rust
loop {
    if stop_rx.try_recv().is_ok() { break; }
    backend.click(button)?;
    let sleep_ms = interval_ms + jitter(-jitter_ms..=jitter_ms);
    let start = Instant::now();
    // Sleep in 1ms chunks to allow responsive cancellation
    while start.elapsed().as_millis() < sleep_ms as u128 {
        if stop_rx.try_recv().is_ok() { return Ok(()); }
        thread::sleep(Duration::from_millis(1));
    }
    if let Some(dur) = duration_ms {
        if action_start.elapsed().as_millis() >= dur as u128 { break; }
    }
}
```

#### `core/input_backend.rs`
```rust
pub trait InputBackend: Send + Sync {
    fn init(&mut self) -> Result<(), BackendError>;
    fn click(&self, button: MouseButton) -> Result<(), BackendError>;
    fn key_press(&self, key_code: u32) -> Result<(), BackendError>;
    fn scroll(&self, direction: ScrollDirection, amount: i32) -> Result<(), BackendError>;
    fn move_relative(&self, dx: i32, dy: i32) -> Result<(), BackendError>;
    fn name(&self) -> &str;
}
```

- `LoggingBackend` — logs all calls via `Logger`, never fails, always returns `Ok(())`.
- `UinputBackend` — opens `/dev/uinput`, creates a virtual device named
  `"wayclick-virtual-pointer"` with `BUS_USB`, VID `0x1d6b`, PID `0x0001` ("Linux
  Foundation"). Supports EV_KEY (BTN_*), EV_REL (REL_X, REL_Y, REL_WHEEL, REL_HWHEEL),
  EV_SYN. Drops privileges after init if running as root (via `setuid`/`setgid` to a
  configured unprivileged user).
- `MockBackend` — records all calls in a `Vec<BackendCall>` for assertion in tests.

**UinputBackend** click sequence:
```
EV_KEY  BTN_LEFT  1   (press)
EV_SYN  SYN_REPORT 0
EV_KEY  BTN_LEFT  0   (release)
EV_SYN  SYN_REPORT 0
```
Note: Each button down/up must be followed by SYN_REPORT. Do NOT emit an extra SYN after
the full press+release sequence.

**UinputBackend** scroll:
```
EV_REL  REL_WHEEL  +amount   (positive = up/away from user)
EV_SYN  SYN_REPORT 0
```

#### `core/evdev_source.rs`

```rust
pub trait InputSource: Send {
    fn device_info(&self) -> DeviceInfo;
    fn poll_events(&mut self, timeout: Duration) -> Result<Vec<InputEvent>, SourceError>;
    fn close(self);
}

pub struct DeviceInfo {
    pub path: PathBuf,
    pub name: String,
    pub vendor_id: u16,
    pub product_id: u16,
    pub phys: String,
}
```

`EvdevSource`:
- Opens device with `O_RDONLY | O_NONBLOCK`.
- Optionally grabs device with `EVIOCGRAB` if `exclusive = true`.
- Poll loop with 250 ms timeout: reads `input_event` structs, filters `EV_KEY` events.
- Only fires trigger on `value == 1` (press), or tracks `value == 0` (release) for Hold mode.
- On `POLLHUP`/`POLLERR` or `read` returning `ENODEV`: device disconnected, send disconnect
  notification to the `EvdevMonitor`.

#### `core/evdev_monitor.rs`

Coordinates all `EvdevSource` threads and device hotplug.

```rust
pub struct EvdevMonitor {
    engine: Arc<Mutex<Engine>>,
    logger: Logger,
    config_bindings: Vec<DeviceBinding>,
    active_devices: HashMap<PathBuf, DeviceHandle>,
    udev_monitor: UdevMonitor,
}
```

- `fn configure(&mut self, bindings: Vec<DeviceBinding>)` — stop all, re-enumerate.
- `fn start(&mut self)` — scan `/dev/input/event*`, match devices, launch threads, start
  udev monitor thread.
- `fn stop(&mut self)` — graceful shutdown of all device threads.
- Internal: `fn match_device(info: &DeviceInfo, binding: &DeviceBinding) -> bool`
- Internal: `fn enumerate_devices() -> Vec<(PathBuf, DeviceInfo)>` — iterates
  `/dev/input/event*`, reads attrs via ioctls.
- Udev monitor thread: listens on `PF_NETLINK` `NETLINK_KOBJECT_UEVENT` socket for
  `add`/`remove` events with `SUBSYSTEM=input`. On `add`: try to match and open. On
  `remove`: stop matching thread.

#### `core/config_watcher.rs`

- Watches `init.lua` and all `lua/**/*.lua` in the config directory.
- Polls `std::fs::metadata(...).modified()` every 500ms in a background thread.
- On change: calls the `Callback: Fn() + Send` provided at `start()`.
- On SIGHUP (signal forwarded via `Arc<AtomicBool>`): triggers immediate reload.

#### `core/logger.rs`

- `struct Logger { capacity: usize, entries: Mutex<VecDeque<LogEntry>> }`
- `struct LogEntry { timestamp: SystemTime, level: LogLevel, message: String }`
- `enum LogLevel { Trace, Debug, Info, Warn, Error }`
- Methods: `trace/debug/info/warn/error(msg)`, `recent(n) -> Vec<LogEntry>`.
- Writes to `stdout` (Info/Debug/Trace) or `stderr` (Warn/Error) with ISO-8601 timestamp.
- Structured JSON log output mode enabled by `--log-json` flag (useful for systemd journal
  parsing).

#### `core/ipc.rs`

IPC protocol: JSON-RPC 2.0 over a length-prefixed UNIX stream socket.

Frame format: `[u32 big-endian length][UTF-8 JSON bytes]`

**Request methods (client → daemon):**

| Method | Params | Description |
|--------|--------|-------------|
| `status` | none | Returns daemon status |
| `status_json` | none | Returns full JSON status |
| `toggle` | none | Toggle enabled state |
| `enable` | none | Set enabled = true |
| `disable` | none | Set enabled = false |
| `trigger` | `{"id": "...", "press": true}` | Fire a trigger event |
| `list_triggers` | none | List all triggers |
| `reload_config` | none | Trigger hot reload |
| `logs_tail` | `{"n": 50}` | Return last N log entries |
| `ping` | none | Returns `pong` |

**Response envelope:**
```json
{"jsonrpc": "2.0", "id": 1, "result": {...}}
{"jsonrpc": "2.0", "id": 1, "error": {"code": -32000, "message": "..."}}
```

**Status result:**
```json
{
  "enabled": true,
  "dry_run": false,
  "trigger_count": 3,
  "active_triggers": ["rapid_fire"],
  "device_count": 2,
  "active_devices": ["/dev/input/event5"],
  "backend": "uinput",
  "config_path": "/home/user/.config/wayclick/init.lua",
  "uptime_secs": 3600
}
```

### 4.2 `wayclickd` binary

Entry point for the daemon.

```
wayclickd [OPTIONS]

Options:
  --config <path>          Path to init.lua (default: $WAYCLICK_CONFIG or
                           ~/.config/wayclick/init.lua)
  --check-config <path>    Validate config and exit (exit 0 = OK, 1 = error)
  --check-permissions      Check /dev/uinput and /dev/input access, then exit
  --dry-run                Override config: force dry_run = true
  --enable                 Start with automation enabled (default: disabled)
  --log-level <level>      trace|debug|info|warn|error (default: info)
  --log-json               Emit structured JSON log lines (for journald)
  --socket <path>          Override IPC socket path
  -h, --help               Show help
  -V, --version            Show version
```

Startup sequence:
1. Parse arguments.
2. Load and validate config. If `--check-config`: print result and exit.
3. If `--check-permissions`: print access status table and exit.
4. Create IPC socket directory with mode `0700`.
5. Initialize Logger.
6. Create Engine with LoggingBackend (dry run until uinput confirmed).
7. Start IPC server.
8. Create and init InputBackend (UinputBackend or LoggingBackend based on `dry_run`).
9. Configure EvdevMonitor with device bindings, start it.
10. Start ConfigWatcher.
11. Install signal handlers (SIGINT, SIGTERM → shutdown; SIGHUP → reload).
12. Main loop: `while !shutdown { sleep(100ms) }`.
13. Graceful shutdown: stop ConfigWatcher, EvdevMonitor, IPC server, Engine.

**Permission check output** (human-readable table):
```
Permission Check
────────────────────────────────
/dev/uinput          ✓ writable
/dev/input/event*    ✓ readable (member of 'input' group)
Lua config           ✓ found at /home/user/.config/wayclick/init.lua
IPC socket dir       ✓ /run/user/1000/ writable
────────────────────────────────
All checks passed. Start daemon with: wayclickd --enable
```

### 4.3 `wayclickctl` binary

```
wayclickctl [OPTIONS] <COMMAND>

Options:
  --socket <path>    IPC socket path (default: $XDG_RUNTIME_DIR/wayclick.sock)
  --json             Output raw JSON response
  --timeout <ms>     Connection timeout (default: 2000)
  -h, --help

Commands:
  status             Show daemon status
  toggle             Toggle automation on/off
  enable             Enable automation
  disable            Disable automation
  trigger <id>       Fire a named trigger
  list               List all triggers with current state
  reload             Reload configuration
  logs [--tail N]    Show recent log entries
  ping               Check if daemon is running
```

Exit codes: 0 = success, 1 = daemon not running / connection failed, 2 = command error.

### 4.4 `wayclick-tui` binary

A full-screen ncurses (or `crossterm`/`ratatui` in Rust) TUI.

Layout:
```
┌─ wayclick ──────── [enabled] ── config: ~/.config/wayclick/init.lua ──────────────┐
│ TRIGGERS                          │ TRIGGER DETAIL                                 │
│ ─────────────────────────────── │ ────────────────────────────────────────────── │
│ ● rapid_fire    [ACTIVE]  toggle │ id:          rapid_fire                        │
│ ○ key_matrix    [idle]    toggle │ mode:        toggle                            │
│ ○ key_repeat    [idle]    hold   │ action:      auto_click                        │
│                                   │ button:      left                              │
│                                   │ interval_ms: 10                               │
│                                   │ jitter_ms:   5                                │
│                                   │                                                │
│ DEVICES                           │ RECENT LOGS                                    │
│ ─────────────────────────────── │ ────────────────────────────────────────────── │
│ ✓ /dev/input/event5  G Pro Mouse │ [INFO] Config reloaded                         │
│ ✗ Logitech G502      (searching) │ [INFO] rapid_fire: started                     │
│                                   │ [DEBUG] uinput click BTN_LEFT                  │
├───────────────────────────────────────────────────────────────────────────────────┤
│ q:quit  t:toggle  r:reload  e:enable  d:disable  /:search  ↑↓:select  enter:fire │
└───────────────────────────────────────────────────────────────────────────────────┘
```

Navigation:
- `hjkl` / arrow keys to navigate.
- `t` to toggle overall enable/disable.
- `r` to reload config.
- `enter` or `space` to fire selected trigger.
- `q` / `Ctrl-C` to quit.
- `:` for command prompt with tab completion (`:toggle`, `:reload`, `:trigger <id>`,
  `:enable`, `:disable`).
- `Tab` to switch between panes

Style:
- `catpuccine` as default theme

**Permissions screen** (shown on startup if permissions are missing):
```
⚠ Permissions Required
──────────────────────────────────────────────────────────────────
/dev/uinput is not writable.

To fix this, run:
  sudo groupadd -f wayclick
  sudo usermod -aG wayclick $USER
  sudo cp udev/99-wayclick.rules /etc/udev/rules.d/
  sudo udevadm control --reload && sudo udevadm trigger
  newgrp wayclick    (or log out and back in)

Then set dry_run = false in your config.
──────────────────────────────────────────────────────────────────
[Press any key to continue in dry-run mode]
```

### 4.5 `wayclick-evdev-dump` utility

Diagnostic tool: prints all events from a specific `/dev/input/event*` device.

```
wayclick-evdev-dump /dev/input/event5
wayclick-evdev-dump --list         # List all input devices with VID:PID and name
wayclick-evdev-dump --identify     # Print the DeviceMatch config snippet for a device
```

The `--list` output should be:
```
/dev/input/event0  AT Translated Set 2 keyboard   [0001:0001] usb-0000:00:14.0-1
/dev/input/event5  Logitech G Pro Gaming Mouse     [046d:c08b] usb-0000:00:14.0-2
```

The `--identify` output should produce a ready-to-paste `bind_device` snippet:
```lua
wayclick.bind_device({
  name = "Logitech G Pro Gaming Mouse",  -- matches any device containing this string
  -- vid = 0x046d, pid = 0xc08b,        -- alternative: match by VID:PID
  bindings = {
    { code = "BTN_SIDE",  trigger = "TODO" },
    { code = "BTN_EXTRA", trigger = "TODO" },
  },
})
```

---

## 5. Lua Configuration Reference

### 5.1 Full Example — `~/.config/wayclick/init.lua`

```lua
local wc = wayclick

wc.set_options({
  dry_run = false,
  log_capacity = 512,
})

-- Trigger 1: Toggle rapid left-click loop on mouse button 4
wc.register_trigger({
  id = "rapid_fire",
  name = "Rapid Fire",
  mode = "toggle",
  cooldown_ms = 300,
  action = wc.auto_click({
    button = "left",
    interval_ms = 10,
    jitter_ms = 5,
  }),
})

-- Trigger 2: Hold mouse button 5 to send staggered key presses
wc.register_trigger({
  id = "key_matrix",
  name = "Key Matrix",
  mode = "toggle",
  action = wc.parallel({
    actions = {
      wc.key_press({ key = "1", interval_ms = 30000 }),
      wc.key_press({ key = "3", interval_ms = 59000 }),
    },
  }),
})

-- Trigger 3: One-shot burst (fires 5 clicks and stops)
wc.register_trigger({
  id = "burst_fire",
  name = "Burst Fire",
  mode = "oneshot",
  action = wc.auto_click({
    button = "left",
    interval_ms = 50,
    duration_ms = 250,   -- 5 clicks at 50ms = 250ms
  }),
})

-- Bind triggers to physical device buttons
wc.bind_device({
  name = "Logitech G Pro Gaming Mouse",
  bindings = {
    { code = "BTN_SIDE",  trigger = "rapid_fire" },
    { code = "BTN_EXTRA", trigger = "key_matrix" },
  },
})

-- Compositor integration: no bind_device needed, use wayclickctl from compositor binds
-- Hyprland example:
--   bind = ,mouse:276, exec, wayclickctl trigger rapid_fire
--   bind = $mod, M, exec, wayclickctl toggle
```

### 5.2 Module pattern (`lua/` directory)

```lua
-- ~/.config/wayclick/init.lua
local wc = wayclick
wc.set_options({ dry_run = false })
require("triggers.gaming")
require("triggers.productivity")
```

```lua
-- ~/.config/wayclick/lua/triggers/gaming.lua
local wc = wayclick
wc.register_trigger({ id = "rapid_fire", ... })
wc.bind_device({ name = "Logitech G Pro", bindings = {...} })
```

---

## 6. Build System and DevOps

### 6.1 Rust / Cargo

Workspace layout:
```
wayclick/
  Cargo.toml               (workspace)
  Cargo.lock
  crates/
    wayclick-core/          -- library: config, engine, backends, monitor
      Cargo.toml
      src/
        lib.rs
        config.rs
        lua_api.rs
        engine.rs
        input_backend.rs    (trait + logging + uinput + mock)
        evdev_source.rs
        evdev_monitor.rs
        config_watcher.rs
        logger.rs
        ipc.rs
    wayclickd/              -- daemon binary
      Cargo.toml
      src/main.rs
    wayclickctl/            -- control CLI
      Cargo.toml
      src/main.rs
    wayclick-tui/           -- TUI frontend
      Cargo.toml
      src/main.rs
    wayclick-evdev-dump/    -- diagnostic utility
      Cargo.toml
      src/main.rs
  config/                   -- example configs
  docs/
  systemd/
  udev/
  scripts/
  tests/                    -- integration tests (separate crate)
    Cargo.toml
    src/
```

**`Cargo.toml` (workspace):**
```toml
[workspace]
members = [
  "crates/wayclick-core",
  "crates/wayclickd",
  "crates/wayclickctl",
  "crates/wayclick-tui",
  "crates/wayclick-evdev-dump",
  "tests",
]
resolver = "2"

[workspace.dependencies]
mlua = { version = "0.9", features = ["lua54", "vendored"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "1"
log = "0.4"
env_logger = "0.11"
nix = { version = "0.27", features = ["ioctl", "signal", "socket", "uio"] }
```

**Key crate dependencies for `wayclick-core`:**
- `mlua` (lua54, vendored or system): Lua 5.4 embedding.
- `evdev` crate (or raw `nix` ioctl wrappers): reading `/dev/input/event*`.
- `udev` crate or raw netlink: hotplug monitoring.
- `serde` + `serde_json`: JSON serialisation/deserialisation.
- `thiserror`: ergonomic error types.
- `nix`: UNIX syscalls (ioctl, open, poll, socket).

**Key crate for `wayclick-tui`:**
- `ratatui` + `crossterm` (recommended over raw ncurses for Rust).

### 6.2 C / CMake alternative layout

If implementing in C:
```
wayclick/
  CMakeLists.txt
  cmake/
    FindLua.cmake
    FindLibevdev.cmake
    Sanitizers.cmake
  src/
    core/
      config.c / config.h       -- Lua API + config parsing
      engine.c / engine.h       -- trigger state machine
      input_backend.c / .h      -- abstract interface
      uinput_backend.c / .h
      logging_backend.c / .h
      evdev_source.c / .h
      evdev_monitor.c / .h
      config_watcher.c / .h
      logger.c / .h
      ipc.c / .h
      device_match.c / .h
    daemon/
      main.c
    cli/
      main.c
    tui/
      main.c
    tools/
      evdev_dump.c
  tests/
    CMakeLists.txt
    test_config.c
    test_engine.c
    test_device_match.c
    test_ipc_protocol.c
  ...
```

CMake options mirror the prototype: `WAYCLICK_ENABLE_SANITIZERS`, `WAYCLICK_ENABLE_LTO`,
`WAYCLICK_ENABLE_CLANG_TIDY`, `WAYCLICK_BUILD_TUI`, `WAYCLICK_BUILD_CLI`,
`WAYCLICK_VENDOR_DEPS`.

### 6.3 Build targets and scripts

```bash
# Release build (Rust)
cargo build --release --workspace

# Development build with all features
cargo build --workspace

# Run tests
cargo test --workspace

# Lint
cargo clippy --workspace -- -D warnings

# Format check
cargo fmt --check

# Security audit
cargo audit

# Address sanitizer (nightly)
RUSTFLAGS="-Z sanitizer=address" cargo +nightly test --target x86_64-unknown-linux-gnu

# Fuzz target (see §7.3)
cargo fuzz run fuzz_config_loader -- -max_total_time=60
```

Provide `scripts/dev.sh`: runs `cargo build`, `cargo test`, starts daemon in dry-run mode.
Provide `scripts/install.sh`: copies binaries, installs udev rules, creates systemd service.
Provide `scripts/check_permissions.sh`: checks group membership and device access.

### 6.4 CI/CD (GitHub Actions)

`.github/workflows/ci.yml` should include:

```yaml
jobs:
  build:
    runs-on: ubuntu-22.04
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with: { components: clippy, rustfmt }
      - run: cargo fmt --check
      - run: cargo clippy --workspace -- -D warnings
      - run: cargo build --workspace
      - run: cargo test --workspace
      - run: cargo audit

  sanitizers:
    runs-on: ubuntu-22.04
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@nightly
      - run: |
          RUSTFLAGS="-Z sanitizer=address,leak" \
          cargo +nightly test --target x86_64-unknown-linux-gnu \
          -p wayclick-core

  security:
    runs-on: ubuntu-22.04
    steps:
      - uses: actions/checkout@v4
      - run: cargo install cargo-audit
      - run: cargo audit
      - run: |
          pip install semgrep
          semgrep --config=auto src/
```

---

## 7. Test Suite

### 7.1 Unit Tests (in-module, `#[cfg(test)]`)

Each module in `wayclick-core` must have thorough unit tests.

#### `config.rs` tests
- `test_minimal_config`: `set_options` + one trigger, no device binding.
- `test_auto_click_defaults`: missing fields get correct defaults.
- `test_all_mouse_buttons`: parse `"left"/"right"/"middle"/"button4"/"button5"`.
- `test_all_trigger_modes`: `"toggle"/"hold"/"oneshot"`.
- `test_key_normalization`: `"space"` → `KEY_SPACE`, `"b"` → `KEY_B`, `"KEY_SPACE"` → pass-through, `"F1"` → `KEY_F1`.
- `test_composite_parallel` and `test_composite_sequence`.
- `test_evdev_bind_device_by_name`.
- `test_evdev_bind_device_by_vidpid`.
- `test_bind_evdev_legacy_with_warning`: deprecated `bind_evdev` still works.
- `test_config_error_duplicate_trigger_id`: returns `Err`.
- `test_config_error_unknown_trigger_ref`: device binding references unknown trigger ID.
- `test_config_error_invalid_key`: `"NOT_A_KEY_9999"` → `Err`.
- `test_sandbox_blocks_os_execute`: Lua `os.execute` returns nil or errors.
- `test_sandbox_blocks_io_popen`: Lua `io.popen` is nil.
- `test_sandbox_allows_require_local`: `require("utils")` resolves against config `lua/` dir.
- `test_lua_module_loading`: init.lua that `require`s a helper module, checks both files
  contribute to config.
- `test_socket_path_default`: empty socket_path → auto-detect from `XDG_RUNTIME_DIR`.

#### `device_match.rs` tests
- `test_match_by_name_exact`, `test_match_by_name_substring`, `test_match_by_name_case_insensitive`.
- `test_match_by_vidpid`.
- `test_match_by_phys_substring`.
- `test_match_any_first_wins`, `test_match_any_all_fail`.
- `test_no_match`.
- `test_path_match_emits_deprecation_warning`.

#### `engine.rs` tests
- Uses `MockBackend` and `MockSource`.
- `test_toggle_starts_worker`: trigger_event fires, worker records clicks.
- `test_toggle_stops_worker`: second trigger_event stops worker.
- `test_hold_press_release`: Hold mode starts on press, stops on release.
- `test_oneshot_executes_synchronously`.
- `test_disabled_engine_ignores_trigger`.
- `test_cooldown_debounce`: two rapid triggers within cooldown_ms → only one fires.
- `test_apply_config_stops_running_workers`.
- `test_jitter_range`: 1000 samples, all within `interval_ms ± jitter_ms`.
- `test_duration_limit`: action with `duration_ms = 100` stops after ≤ 110ms.

#### `ipc.rs` tests
- `test_frame_encode_decode`: roundtrip of length-prefixed frames.
- `test_status_response`: well-formed JSON-RPC response.
- `test_unknown_method`: returns JSON-RPC error `-32601`.
- `test_trigger_unknown_id`: returns error.
- `test_concurrent_clients`: 10 simultaneous connections, all get responses.

#### `uinput_backend.rs` tests
- These require `/dev/uinput` — gate with `#[cfg_attr(not(feature="integration"), ignore)]`
  and a `integration` feature flag.
- `test_uinput_click_left/right/middle`.
- `test_uinput_scroll_up/down`.
- `test_uinput_key_press`.
- `test_init_fails_gracefully_no_device`: mock the fd open to fail, check error message.

### 7.2 Integration Tests (`tests/` workspace member)

Integration tests run the full daemon in a subprocess and communicate over IPC.

```rust
// tests/src/lib.rs
fn spawn_daemon(config: &str) -> DaemonProcess { ... }
fn connect_ipc(socket_path: &str) -> IpcClient { ... }
```

Tests:
- `test_daemon_startup_and_ping`: daemon starts, `ping` returns `pong`.
- `test_status_disabled_on_start`: status shows `enabled: false`.
- `test_toggle_enable_disable`: toggle → enable → status shows enabled.
- `test_trigger_list_matches_config`: `list_triggers` returns configured triggers.
- `test_hot_reload`: write new config file, wait 600ms, verify new trigger appears in list.
- `test_dry_run_no_uinput`: start with `dry_run = true`, fire trigger, verify no real input.
- `test_check_config_exit_zero`: `wayclickd --check-config valid.lua` exits 0.
- `test_check_config_exit_one`: `wayclickd --check-config invalid.lua` exits 1.
- `test_graceful_shutdown`: SIGTERM → daemon exits cleanly, socket removed.
- `test_sighup_reload`: SIGHUP → config reloaded without restarting socket.

### 7.3 Fuzz Testing

Using `cargo-fuzz` (libFuzzer):

**`fuzz/fuzz_config_loader.rs`**: Feed arbitrary bytes as Lua source to the config loader.
Must not panic, must not crash, must not execute arbitrary code (sandbox test), must
return either `Ok(Config)` or `Err(ConfigError)`.

**`fuzz/fuzz_ipc_frame.rs`**: Feed arbitrary bytes to the IPC frame parser. Must not panic.

**`fuzz/fuzz_device_match.rs`**: Feed arbitrary device name strings and match patterns.

### 7.4 Security Analysis

**Static analysis:**
- `cargo clippy -- -D warnings`: catches unsafe patterns.
- `cargo audit`: checks dependencies against RustSec advisory database.
- Semgrep with `auto` ruleset in CI.
- C alternative: `cppcheck --enable=all`, Clang Static Analyzer (`scan-build`), `flawfinder`.

**Manual security checklist:**
1. Lua sandbox: verify `os.execute`, `io.popen`, `require` outside config dir, `load`,
   `loadfile`, `dofile`, `debug` are all blocked. Write explicit test cases.
2. IPC socket permissions: socket must be created with mode `0600` (owner-only).
3. Path traversal: config `require` path is resolved to an absolute path inside the config
   `lua/` directory; any `..` components or absolute paths are rejected.
4. Integer overflow: `interval_ms` and `jitter_ms` are validated ≥ 1 and ≤ 60000.
5. Buffer size: IPC frame length field is validated ≤ 64KB; larger frames are rejected.
6. `EVIOCGRAB` fallback: if grab fails, log warning and continue without exclusive mode.
7. Daemon privileges: document that the daemon should NOT run as root. Provide udev rules
   and group membership instructions so it can run as a normal user.
8. `uinput` device name: the virtual device name `"wayclick-virtual-pointer"` must not be
   spoofable by config (it is hardcoded, not configurable).

### 7.5 Performance Tests

Using `criterion` (Rust benchmarking):

- `bench_config_load`: parse a 50-trigger config 1000 times. Target: < 1ms per parse.
- `bench_engine_trigger_dispatch`: trigger event dispatch overhead (no I/O). Target: < 10μs.
- `bench_ipc_roundtrip`: status command latency (loopback socket). Target: < 1ms.

---

## 8. Systemd and udev Integration

### 8.1 `udev/99-wayclick.rules`
```
# Grant the 'wayclick' group write access to uinput
KERNEL=="uinput", SUBSYSTEM=="misc", MODE="0660", GROUP="wayclick", TAG+="uaccess"

# Grant members of 'input' group read access to all input devices
# (This rule is usually provided by systemd; included here for reference)
KERNEL=="event*", SUBSYSTEM=="input", MODE="0660", GROUP="input", TAG+="uaccess"
```

### 8.2 `systemd/wayclickd.service`
```ini
[Unit]
Description=Wayclick programmable mouse automation daemon
Documentation=https://github.com/yourusername/wayclick
After=network.target

[Service]
Type=simple
ExecStart=/usr/bin/wayclickd --config /etc/wayclick/init.lua --enable
ExecReload=/bin/kill -HUP $MAINPID
Restart=on-failure
RestartSec=5s

# Privilege restrictions
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ProtectHome=read-only
ReadWritePaths=/run/user /tmp

[Install]
WantedBy=default.target
```

For user-session usage (no root):
```ini
[Unit]
Description=Wayclick daemon (user session)
After=default.target

[Service]
ExecStart=/usr/bin/wayclickd --enable
Restart=on-failure

[Install]
WantedBy=default.target
```

Enable with: `systemctl --user enable --now wayclickd`

---

## 9. Documentation Requirements

The following documents must be included in the `docs/` directory:

- `ARCHITECTURE.md`: Module diagram (ASCII art), data flow, threading model.
- `BUILDING.md`: Dependency list (Arch, Debian, Fedora package names), build commands,
  CMake/Cargo options, cross-compilation notes.
- `CONFIG_SCHEMA.md`: Full Lua API reference with type annotations and all defaults.
- `PERMISSIONS.md`: Step-by-step permissions setup, group membership, udev rules, systemd.
- `DEVICE_MATCHING.md`: Explains stable device identity, `bind_device` vs `bind_evdev`,
  how to use `wayclick-evdev-dump --identify`, hotplug behaviour.
- `HYPRLAND_BINDINGS.md`: How to bind Hyprland keys/buttons to `wayclickctl trigger`.
- `SECURITY.md`: Threat model, sandbox, privilege model, IPC security.
- `THEMES.md` (TUI only): Theme configuration reference.
- `CONTRIBUTING.md`: Development setup, test commands, PR checklist.

---

## 10. Lessons Learned from Prototype

The following problems were encountered in the C++ prototype and **must be avoided**
in the new implementation:

### 10.1 Device Path Fragility (Critical)
The prototype used hardcoded `/dev/input/eventN` paths. These change every reboot when
devices are reconnected or other USB devices are added. Real users immediately hit this
problem.
**Fix:** Implement `DeviceMatch` as described in §3.4. Make path-based binding a deprecated
fallback that emits a startup warning.

### 10.2 No Hotplug Support
If the configured mouse was unplugged and replugged, the daemon continued running but no
longer monitored the device. Users had to restart the daemon.
**Fix:** Implement the udev netlink monitor in `EvdevMonitor` (§4.1).

### 10.3 Hold Trigger Blocked IPC Thread
The prototype's Hold mode ran the action synchronously in `trigger_event`, which was called
from the IPC server thread. A long-running Hold action would block all IPC operations.
**Fix:** All action execution (including Hold) runs in worker threads with a stop channel.

### 10.4 Double SYN in UinputBackend
The prototype's `click()` method called `button_event(true)` which internally called
`emit()` + `sync()`, then `button_event(false)` which also called `sync()`, then an
additional `sync()` at the end of `click()`. This results in two redundant SYN_REPORT events.
**Fix:** Only emit SYN_REPORT once per logical event group: after press, and after release.
The correct sequence is: `[press, SYN], [release, SYN]`.

### 10.5 Config Format Confusion
The prototype had both TOML example configs (`config/*.toml`) and a Lua `init.lua`, but the
TOML parser was removed. Leftover TOML files were confusing.
**Fix:** The new implementation uses Lua exclusively. Include TOML configs only in a
`docs/examples/` subdirectory with a note that they are illustrative only.

### 10.6 IPC Framing Bugs
The simple newline-terminated text IPC worked for short responses but broke if a JSON status
response contained newlines or was longer than 512 bytes (the fixed read buffer).
**Fix:** Use length-prefixed framing (§4.1 `core/ipc.rs`).

### 10.7 Minimal Tests
The prototype had one test file (`config_loader_tests.cpp`) using raw `assert()`. There were
no engine tests, no IPC tests, no device match tests.
**Fix:** Implement the full test suite in §7.

### 10.8 No Scroll or Mouse Move Support
The prototype's `InputBackend` only supported `click()` and `key_press()`. The
`UinputBackend` didn't configure `EV_REL` capabilities so scroll and move would silently fail.
**Fix:** `InputBackend` trait includes `scroll()` and `move_relative()`. `UinputBackend.init()`
registers `EV_REL` with `REL_X`, `REL_Y`, `REL_WHEEL`, `REL_HWHEEL`.

### 10.9 TUI Was Stub
The prototype's `wayclick-tui` printed one line and exited.
**Fix:** Implement the TUI as described in §4.4 using `ratatui`/`crossterm`.

### 10.10 Logger Wrote to stdout/stderr Only
No structured output, no log levels above INFO were adjustable at runtime, no way to tail
logs from the CLI.
**Fix:** Add `--log-level` flag, `--log-json` flag (structured JSON lines for journald), and
a `logs_tail` IPC command so `wayclickctl logs` works.

### 10.11 ConfigWatcher Only Scanned lua/ Subdirectory
The ConfigWatcher watched `init.lua` and `lua/**/*.lua` but missed files in other
subdirectories. If a user structured their config as `config/mouse.lua` (not under `lua/`),
changes were not detected.
**Fix:** Watch all `*.lua` files in the entire config directory tree, not just the `lua/`
subdirectory.

### 10.12 Engine Used Single Mutex for Everything
A single `Mutex<Engine>` meant all IPC commands, all trigger events, and all worker-thread
state changes were serialized. Fine for MVP but will cause latency under load.
**Fix:** Use a `RwLock` for config reads, finer-grained locking for trigger state, and
message-passing (`std::sync::mpsc`) between IPC threads and the engine rather than holding
locks across I/O operations.

---

## 11. Implementation Checklist

Implement in this order to deliver a functional MVP quickly, then layer on features:

### Phase 1 — Core (MVP)
- [ ] Workspace skeleton with all crates and empty `main.rs`/`lib.rs`.
- [ ] `Logger` with ring buffer and stdout/stderr output.
- [ ] `Config` data model (all structs/enums).
- [ ] `ConfigLoader` parsing Lua config with sandboxed VM.
- [ ] Unit tests for `ConfigLoader`.
- [ ] `InputBackend` trait + `LoggingBackend` + `MockBackend`.
- [ ] `Engine` with Toggle/Hold/OneShot, worker threads, stop channels.
- [ ] Unit tests for `Engine` using `MockBackend`.
- [ ] `wayclickd` startup: load config, init engine, signal handling, main loop.
- [ ] IPC server with length-prefixed JSON-RPC.
- [ ] `wayclickctl` with all subcommands.
- [ ] Integration test: daemon start, ping, toggle, trigger, shutdown.

### Phase 2 — Real Input
- [ ] `UinputBackend` with click, key_press, scroll, move_relative.
- [ ] Integration tests for `UinputBackend` (gated behind `integration` feature).
- [ ] `--check-permissions` command.
- [ ] udev rules, systemd service files.
- [ ] `PERMISSIONS.md`.

### Phase 3 — Physical Input
- [ ] `DeviceMatch` enum and matching logic.
- [ ] Unit tests for device matching.
- [ ] `EvdevSource` with press/release tracking.
- [ ] `EvdevMonitor` with device enumeration and thread management.
- [ ] Udev netlink monitor for hotplug.
- [ ] `wayclick-evdev-dump` with `--list` and `--identify`.
- [ ] Integration tests for evdev binding.
- [ ] `DEVICE_MATCHING.md`.

### Phase 4 — Polish
- [ ] `ConfigWatcher` with full config tree scanning.
- [ ] Hot reload integration tests (SIGHUP and file change).
- [ ] `wayclick-tui` with ratatui.
- [ ] TUI permissions screen.
- [ ] Structured JSON logging (`--log-json`).
- [ ] `logs_tail` IPC command + `wayclickctl logs`.
- [ ] All documentation files.

### Phase 5 — Security and DevOps
- [ ] Lua sandbox tests (all blocked functions verified).
- [ ] Fuzz targets for config loader, IPC framer, device match.
- [ ] `cargo audit` in CI.
- [ ] Semgrep in CI.
- [ ] Performance benchmarks with `criterion`.
- [ ] Security checklist (§7.4) verified.
- [ ] `SECURITY.md`.

### Phase 6 — Roadmap Features (post-MVP)
- [ ] Per-app profiles: integrate compositor event stream (Hyprland IPC, EWMH/NET_ACTIVE_WINDOW).
- [ ] Profile match rules: by app class, title, executable.
- [ ] Profile layering with momentary overlays.
- [ ] Live TUI editing of trigger parameters with IPC write-back.
- [ ] `wayclickctl profile set/next`.
- [ ] Plugin system for custom Lua action types.
- [ ] Absolute mouse movement (`EV_ABS`).
- [ ] Multi-axis scroll (`REL_HWHEEL`).

---

## 12. Quick-Start Verification

After implementing Phase 1, verify the MVP works:

```bash
# Build
cargo build --workspace

# Validate a config without running the daemon
./target/debug/wayclickd --check-config config/init.lua

# Start in dry-run mode (no privileges needed)
./target/debug/wayclickd --config config/init.lua --enable

# In another terminal:
./target/debug/wayclickctl status
# → {"enabled":true, "dry_run":true, ...}

./target/debug/wayclickctl list
# → [{"id":"rapid_fire","mode":"toggle","active":false}, ...]

./target/debug/wayclickctl trigger rapid_fire
# daemon logs: "DRY RUN click on button left"

./target/debug/wayclickctl trigger rapid_fire
# daemon logs: "rapid_fire: stopped"

./target/debug/wayclickctl toggle
# → disabled

# Run tests
cargo test --workspace
```

After implementing Phase 2 (uinput):

```bash
# Add user to wayclick group (see PERMISSIONS.md)
sudo groupadd -f wayclick
sudo usermod -aG wayclick $USER
sudo cp udev/99-wayclick.rules /etc/udev/rules.d/
sudo udevadm control --reload && sudo udevadm trigger
newgrp wayclick   # or re-login

# Edit config/init.lua: set dry_run = false
# Start daemon with real events
./target/debug/wayclickd --config config/init.lua --enable

# Fire trigger — should produce a real left click
./target/debug/wayclickctl trigger rapid_fire
```

After implementing Phase 3 (evdev):

```bash
# Find your mouse
./target/debug/wayclick-evdev-dump --list
./target/debug/wayclick-evdev-dump --identify /dev/input/event5

# Add user to input group
sudo usermod -aG input $USER

# Add bind_device to config/init.lua using the output of --identify
# Press the physical button → trigger fires automatically
```

---

*End of prompt. This document is self-contained and sufficient to recreate the wayclick project from a blank slate.*
