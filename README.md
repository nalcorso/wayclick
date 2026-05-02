# wayclick

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Rust: 1.85+](https://img.shields.io/badge/rust-1.85%2B-orange.svg)](https://www.rust-lang.org/)

**Kernel-level input automation for Linux.**

Wayclick reads physical button presses directly from the kernel's input layer
(evdev) and fires configurable actions through a virtual device (uinput). It
doesn't care which display server you're running — Wayland, X11, or none at
all. Write your config in Lua. Run it as a systemd service. Automate everything.

> **AI Disclosure** — Developed with AI coding agents (Claude via GitHub
> Copilot). All code was reviewed, tested, and approved by the maintainer.
> Architecture, design decisions, and final implementation remain the
> responsibility of the human author.

---

## What wayclick does

- **Scroll-to-click** — remap the scroll wheel to mouse clicks (the classic
  ARPG technique: scroll to rapid-fire left-click without wearing out the button)
- **Auto-click** — toggle rapid clicking on/off with a side button, with
  configurable interval, jitter, and hold duration
- **Macros** — type text, fire keystroke sequences, chain any combination of
  actions with delays in between
- **Layer switching** — maintain separate binding sets and switch between them
  at runtime (base layer, combat layer, menu layer)
- **Button chording** — bind actions to multi-button combos
- **IPC control** — connect via Unix socket from scripts, game plugins, or any
  external tool to register triggers dynamically and subscribe to events

## How it works

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

---

## Quick start

→ **Getting started?** See [docs/QUICKSTART.md](docs/QUICKSTART.md) for a complete setup guide
with worked examples of the three most common setups (scroll-to-click, auto-clicker, macros).

---

## Configuration

Wayclick is configured entirely in Lua. The config file is
`~/.config/wayclick/init.lua`.

Here's a complete scroll-to-click setup — the most popular use case:

```lua
-- Remap scroll wheel to left-click (requires exclusive device access)
wayclick.register_trigger({
  id    = "left_click",
  mode  = "oneshot",
  action = wayclick.click({ button = "left" }),
})

wayclick.bind_device({
  name      = "G Pro",       -- substring match on device name
  exclusive = true,          -- EVIOCGRAB: suppress original scroll events
  bindings  = {
    { scroll = "up",   trigger = "left_click" },
    { scroll = "down", trigger = "left_click" },
  },
})
```

See [examples/scroll_remap.lua](examples/scroll_remap.lua) for the full
annotated version.

### Available actions

| Action | Mode constraint | Description |
|---|---|---|
| `auto_click` | toggle / hold / oneshot | Repeated mouse clicks with configurable interval and jitter |
| `click` | oneshot | Single mouse click |
| `key_press` | toggle / hold / oneshot | Repeated keyboard key presses |
| `keystroke` | **oneshot only** | Single key chord (key + optional modifiers) |
| `type_text` | **oneshot only** | Type a string character by character |
| `scroll` | toggle / hold / oneshot | Scroll wheel output |
| `mouse_move` | toggle / hold / oneshot | Relative cursor movement |
| `mouse_move_abs` | **oneshot only** | Absolute cursor positioning |
| `click_at` | **oneshot only** | Move to absolute position and click |
| `drag` | **oneshot only** | Click-drag between two positions |
| `set_layer` | **oneshot only** | Switch active binding layer |
| `media_key` | oneshot | Press a media key (volume, play/pause, etc.) |
| `delay` | — | Pause for a fixed duration (use inside `sequence`) |
| `sequence` | any | Run actions one after another |
| `parallel` | toggle / hold | Run actions simultaneously |

See [docs/CONFIG_SCHEMA.md](docs/CONFIG_SCHEMA.md) for the full API reference
with all parameters, defaults, and examples.

### Trigger modes

| Mode | Behaviour |
|---|---|
| `toggle` | First press starts the action, second press stops it |
| `hold` | Active while the button is held; stops on release |
| `oneshot` | Fires once per press |

### Device identification

```sh
wayclick-evdev-dump list      # List all accessible input devices
wayclick-evdev-dump identify  # Press a button — prints the device and code
```

See [docs/DEVICE_MATCHING.md](docs/DEVICE_MATCHING.md) for matching by name,
VID:PID, physical path, and binding chords or tap-vs-hold patterns.

---

## CLI reference

```
wayclickctl <command> [options]
```

| Command | Description |
|---|---|
| `status` | Show daemon state, active layer, trigger count |
| `ping` | Check if the daemon is running |
| `toggle` | Toggle automation on/off |
| `enable` | Enable automation |
| `disable` | Disable automation |
| `trigger <id>` | Fire a trigger by ID |
| `list` | List all configured triggers with their current state |
| `reload` | Reload configuration from disk |
| `logs [--tail N]` | Show recent log entries (default: 50) |
| `layer get` | Show the current active layer |
| `layer set <name>` | Switch to a named layer |
| `waybar [--continuous] [--interval N] [--format minimal\|normal\|verbose]` | Output Waybar-compatible JSON |

Global flags: `--socket <path>`, `--json`, `--timeout <ms>`

---

## IPC and programmatic control

Wayclick exposes a Unix socket with a JSON-RPC 2.0 protocol. Any language that
can open a socket can control it. Connect from game plugins, scripts, or other
daemons to register dynamic triggers, subscribe to events, or query status.

```sh
# Quick test — is the daemon alive?
echo '{"jsonrpc":"2.0","id":1,"method":"ping","params":{}}' | \
  nc -U "$XDG_RUNTIME_DIR/wayclick.sock" | head -c 1000
```

See [docs/IPC.md](docs/IPC.md) for the full protocol reference, all methods,
event types, Python/bash examples, and the dynamic trigger lifecycle.

---

## Waybar integration

A status module for [Waybar](https://github.com/Alexays/Waybar) is included
in [`extras/waybar/`](extras/waybar/).

```jsonc
// ~/.config/waybar/config.jsonc
"custom/wayclick": {
    "exec": "wayclickctl waybar --continuous --interval 2",
    "exec-on-event": false,
    "return-type": "json",
    "on-click": "wayclickctl toggle",
    "format": "{}",
    "tooltip": true
}
```

Three display formats are available: `minimal` (icon only), `normal`
(icon + layer name), and `verbose` (icon + layer + active trigger count).
Four CSS themes are included: Default, Catppuccin, Pill, and Gaming.
See the [module README](extras/waybar/README.md) for full setup instructions.

---

## Installation

### From source

```sh
git clone https://github.com/nalcorso/wayclick.git
cd wayclick
cargo install --path crates/wayclickd
cargo install --path crates/wayclickctl
cargo install --path crates/wayclick-tui
cargo install --path crates/wayclick-evdev-dump
```

### Permissions and Setup

Wayclick requires read access to `/dev/input/event*` and write access to `/dev/uinput`.
See [PERMISSIONS.md](docs/PERMISSIONS.md) for the complete setup guide (groups, udev
rules, and systemd service).

For development builds and cross-compilation, see [BUILDING.md](docs/BUILDING.md).

### Hyprland

```conf
bind = SUPER, F9,  exec, wayclickctl toggle
bind = SUPER, F12, exec, wayclickctl reload
exec-once = wayclickd --enable
```

See [docs/DESKTOP_ENVIRONMENTS.md](docs/DESKTOP_ENVIRONMENTS.md) for setup guides
and examples for all supported desktop environments.

---

## Components

| Binary | Purpose |
|---|---|
| `wayclickd` | Daemon — reads devices, executes triggers, serves IPC |
| `wayclickctl` | CLI — control the running daemon |
| `wayclick-tui` | TUI — real-time dashboard with trigger state and logs |
| `wayclick-evdev-dump` | Diagnostic — list devices, monitor events, identify buttons |
| `wayclick-playground` | Visual testing — GPU-accelerated input visualizer (see [`extras/wayclick-playground/`](extras/wayclick-playground/)) |

---

## Security

- **No network access** — local-only Unix socket IPC with `0600` permissions
- **Sandboxed Lua** — `os.execute`, `io.popen`, `load`, `debug`, `require` for
  native modules all removed; `io.open` restricted to config directory, read-only
- **Least privilege** — runs as your user; needs only `wayclick` + `input` groups
- **Fuzz-tested** — config loading, IPC framing, and device matching are all
  fuzz targets

See [docs/SECURITY.md](docs/SECURITY.md) for the full threat model and Lua
sandbox details.

---

## Development

### Prerequisites

- Rust 1.85+ (via [rustup](https://rustup.rs))
- Linux with kernel support for uinput and evdev
- `gcc` or `clang` (for the vendored Lua build)

### With mise (recommended)

```sh
mise run build    # Build all crates
mise run test     # Run all tests
mise run check    # fmt + clippy + test + deny (full pre-commit suite)
mise run bench    # Run Criterion benchmarks
mise run fuzz     # Run fuzz tests (requires nightly)
```

### Without mise

```sh
cargo build --workspace
cargo test --workspace
cargo clippy --workspace -- -D warnings
cargo fmt --all -- --check
```

See [docs/BUILDING.md](docs/BUILDING.md) for cross-compilation and fuzz testing.

---

## Documentation

| Document | Description |
|---|---|
| [docs/QUICKSTART.md](docs/QUICKSTART.md) | Worked examples for the three most common setups |
| [docs/TROUBLESHOOTING.md](docs/TROUBLESHOOTING.md) | Common issues and solutions, debugging commands |
| [docs/CONFIG_SCHEMA.md](docs/CONFIG_SCHEMA.md) | Complete Lua API reference |
| [docs/IPC.md](docs/IPC.md) | IPC protocol reference with Python/bash examples |
| [docs/DEVICE_MATCHING.md](docs/DEVICE_MATCHING.md) | How to identify and bind physical devices |
| [docs/ROADMAP.md](docs/ROADMAP.md) | Planned features, known limitations, API stability |
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | Crate layout, data flow, threading model |
| [docs/PERMISSIONS.md](docs/PERMISSIONS.md) | Group setup, udev rules, systemd service |
| [docs/SECURITY.md](docs/SECURITY.md) | Threat model, Lua sandbox, IPC security |
| [docs/BUILDING.md](docs/BUILDING.md) | Build from source, testing, fuzz targets |
| [docs/CONTRIBUTING.md](docs/CONTRIBUTING.md) | Development workflow, adding action types |
| [docs/DESKTOP_ENVIRONMENTS.md](docs/DESKTOP_ENVIRONMENTS.md) | Integration guides for Hyprland, Sway, i3, KDE, GNOME |
| [extras/waybar/](extras/waybar/) | Waybar status module with CSS themes |
| [examples/](examples/) | Ready-to-use Lua config examples |

---

## Contributing

See [docs/CONTRIBUTING.md](docs/CONTRIBUTING.md).

```sh
git clone https://github.com/nalcorso/wayclick.git
cd wayclick && ./scripts/dev.sh
# Make changes, then:
mise run check   # or: cargo test --workspace && cargo clippy --workspace -- -D warnings
```

---

## License

[MIT](LICENSE) © Nick Alcorso
