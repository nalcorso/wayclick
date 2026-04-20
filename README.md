# wayclick

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Rust: 1.85+](https://img.shields.io/badge/rust-1.85%2B-orange.svg)](https://www.rust-lang.org/)

**Programmable mouse automation daemon for Linux.** Read physical button presses
via evdev, execute configurable actions through uinput — works with any Linux
desktop environment (Wayland, X11, or headless), fully scriptable with Lua.

> **AI Disclosure** — This project was developed with the assistance of AI coding
> agents (Claude Opus 4.6 via GitHub Copilot) running locally. Because the agents
> run locally through the developer's GitHub Copilot session, all commits are
> attributed to the maintainer's GitHub account rather than a separate AI user.
> All AI-generated code was reviewed, tested, and approved by the maintainer. The
> architecture, design decisions, and final implementation remain the
> responsibility of the human author.

---

## Features

- **Lua-scriptable** — Neovim-style `init.lua` configuration with composable
  actions, helper functions, and module support
- **Display server agnostic** — operates at the kernel input layer (evdev/uinput),
  works with Wayland, X11, or any Linux environment
- **Per-device targeting** — bind actions to specific mice by name, VID:PID, or
  physical port — survives USB reconnects via hotplug monitoring
- **Composable actions** — auto-click, key sequences, mouse movement, scroll,
  delays, drag, media keys — combine them with `sequence` and `parallel`
- **Trigger modes** — Toggle (on/off), Hold (active while pressed), OneShot
  (fire once)
- **Layers** — switch between binding sets at runtime (e.g. "base" vs "combat")
- **Button chording** — bind actions to multi-button combos like `BTN_SIDE+BTN_EXTRA`
- **Scroll remapping** — remap mouse wheel up/down to clicks or any action
  (popular for ARPGs) with magnitude-aware multi-fire
- **Hot-reload** — edit your config and send `SIGHUP` or `wayclickctl reload` —
  no restart needed
- **TUI dashboard** — real-time view of triggers, devices, and logs
- **IPC control** — JSON-RPC over Unix socket; control the daemon from scripts,
  keybindings, or the CLI
- **Waybar integration** — status module with layer display, active trigger
  indicators, and themed CSS presets

## Components

| Binary | Purpose |
|---|---|
| `wayclickd` | Daemon — reads devices, executes triggers, serves IPC |
| `wayclickctl` | CLI — control the daemon (status, toggle, trigger, reload, layer) |
| `wayclick-tui` | TUI — real-time dashboard with trigger state and logs |
| `wayclick-evdev-dump` | Diagnostic — list devices, monitor events, identify buttons |
| `wayclick-input-viz` | Visual testing — GPU-accelerated input visualizer with particle effects |

## Quick Start

```sh
# Build
cargo build --workspace --release

# Set up permissions (creates groups + udev rules)
./scripts/install.sh

# Log out and back in for group changes to take effect, then:
wayclickd --enable
```

The daemon loads `~/.config/wayclick/init.lua` on startup. If the file doesn't
exist, copy the example:

```sh
mkdir -p ~/.config/wayclick
cp config/init.lua ~/.config/wayclick/init.lua
```

## Configuration

Wayclick is configured with Lua. Here's a minimal auto-clicker on mouse button 5:

```lua
wayclick.set_options({ dry_run = false })

wayclick.register_trigger({
  id = "auto_clicker",
  name = "Auto Clicker",
  mode = "toggle",
  cooldown_ms = 300,
  action = wayclick.auto_click({
    button = "left",
    interval_ms = 10,
    jitter_ms = 2,
    hold_ms = 2,
  }),
})

wayclick.bind_device({
  vid = 0x046d, pid = 0xc08b,   -- Logitech G Pro
  bindings = {
    { code = "BTN_EXTRA", trigger = "auto_clicker" },
  },
})
```

### Available Actions

| Action | Description |
|---|---|
| `auto_click` | Repeated mouse clicks with configurable interval, jitter, and hold |
| `key_sequence` | Press/release keyboard keys (including media keys) |
| `scroll` | Scroll wheel automation (vertical or horizontal) |
| `mouse_move` | Relative mouse movement with speed control |
| `mouse_move_abs` | Absolute cursor positioning |
| `click_at` | Move to position and click |
| `drag` | Click-drag from one position to another |
| `delay` | Pause for a duration (for sequences) |
| `set_layer` | Switch the active binding layer |
| `media_key` | Press a media key (volume, play/pause, etc.) |
| `sequence` | Run actions one after another |
| `parallel` | Run actions simultaneously |

### Trigger Modes

| Mode | Behaviour |
|---|---|
| `toggle` | First press starts, second press stops |
| `hold` | Active while button is held, stops on release |
| `oneshot` | Fires once per press |

### Device Identification

Use the built-in diagnostic tool to find your device:

```sh
# List all input devices
wayclick-evdev-dump list

# Press a button to identify it
wayclick-evdev-dump identify
```

See [docs/DEVICE_MATCHING.md](docs/DEVICE_MATCHING.md) for matching by name,
VID:PID, or physical path.

## CLI Reference

```sh
wayclickctl status              # Show daemon state and active triggers
wayclickctl toggle              # Toggle enabled/disabled
wayclickctl enable              # Enable the daemon
wayclickctl disable             # Disable the daemon
wayclickctl trigger <id>        # Fire a trigger by ID
wayclickctl list                # List all configured triggers
wayclickctl reload              # Reload configuration
wayclickctl layer get           # Show current layer
wayclickctl layer set <name>    # Switch to a named layer
wayclickctl logs                # Tail recent log entries
wayclickctl waybar              # Output Waybar-compatible JSON
wayclickctl waybar --continuous # Continuous mode for Waybar exec
```

## Waybar Integration

A status module for [Waybar](https://github.com/Alexays/Waybar) is included
in [`extras/waybar/`](extras/waybar/). It displays the daemon state, current
layer, and active triggers with themed CSS.

```jsonc
// ~/.config/waybar/config.jsonc
"custom/wayclick": {
    "exec": "wayclickctl waybar",
    "return-type": "json",
    "interval": 2,
    "on-click": "wayclickctl toggle",
    "format": "{}",
    "tooltip": true
}
```

Three display formats are available: `minimal` (icon only), `normal`
(icon + layer), and `verbose` (icon + layer + active count). Four CSS themes
are included (Default, Catppuccin, Pill, Gaming). See the
[module README](extras/waybar/README.md) for full setup instructions.

## Installation

### From Source

```sh
git clone https://github.com/nalcorso/wayclick.git
cd wayclick
cargo install --path crates/wayclickd
cargo install --path crates/wayclickctl
cargo install --path crates/wayclick-tui
cargo install --path crates/wayclick-evdev-dump
```

### Permissions

Wayclick needs read access to `/dev/input/event*` and write access to
`/dev/uinput`. The install script handles this, or do it manually:

```sh
sudo groupadd -f wayclick
sudo usermod -aG wayclick,input "$USER"
sudo cp udev/99-wayclick.rules /etc/udev/rules.d/
sudo udevadm control --reload && sudo udevadm trigger
```

See [docs/PERMISSIONS.md](docs/PERMISSIONS.md) for details.

### systemd

```sh
cp systemd/wayclickd.service ~/.config/systemd/user/
systemctl --user enable --now wayclickd
```

### Hyprland Integration

Bind `wayclickctl` commands to Hyprland keys for quick toggle/reload:

```conf
bind = SUPER, F9, exec, wayclickctl toggle
bind = SUPER, F12, exec, wayclickctl reload
exec-once = wayclickd --enable
```

See [docs/HYPRLAND_BINDINGS.md](docs/HYPRLAND_BINDINGS.md) for more examples.

## Development

### Prerequisites

- Rust 1.85+ (via [rustup](https://rustup.rs))
- Linux with kernel support for uinput and evdev
- `gcc` or `clang` (for vendored Lua build)

### With mise

```sh
mise run build      # Build all crates
mise run test       # Run all tests
mise run check      # fmt + clippy + test + deny (full pre-commit check)
mise run bench      # Run Criterion benchmarks
mise run fuzz       # Run fuzz tests (requires nightly)
```

### Without mise

```sh
cargo build --workspace
cargo test --workspace
cargo clippy --workspace -- -D warnings
cargo fmt --all -- --check
```

See [docs/BUILDING.md](docs/BUILDING.md) for cross-compilation and fuzz testing.

### Benchmarks

Criterion.rs micro-benchmarks cover the three critical paths:

| Group | What it measures |
|-------|-----------------|
| `action_execution` | Click dispatch, action sequences, parallel execution, toggle lifecycle |
| `config_loading` | Lua config parsing at various complexity levels (1–100 triggers, nesting depth) |
| `ipc_framing` | JSON-RPC frame encode/decode, Unix socket roundtrip |

```sh
# Quick smoke test (~1 min)
mise run bench-quick

# Full benchmark suite with saved report
./scripts/bench-report.sh    # → bench-results/<timestamp>-<commit>.json
```

Reports include git commit, system info (CPU model, governor, memory), and per-benchmark confidence intervals.

## Architecture

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

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the full module breakdown
and threading model.

## Examples

The [`examples/`](examples/) directory contains ready-to-use Lua configs:

- **[morse_clicker.lua](examples/morse_clicker.lua)** — clicks mouse morse code
  for any word using a full A–Z + 0–9 dictionary. Demo spells "banana" for the
  Revolution Idle achievement.

Copy an example to `~/.config/wayclick/init.lua` to try it out.

## Security

- **No network access** — local-only, Unix socket IPC with `0600` permissions
- **Sandboxed Lua** — `os.execute`, `io.popen`, `load`, `debug` all removed;
  `io.open` restricted to read-only
- **Least privilege** — runs as your user, needs only `wayclick` + `input` groups
- **Fuzz-tested** — config loading, IPC framing, and device matching are all
  fuzz targets
- **Supply chain** — `cargo deny` runs as part of `mise run check`

See [docs/SECURITY.md](docs/SECURITY.md) for the full threat model.

## Documentation

| Document | Description |
|---|---|
| [CONFIG_SCHEMA.md](docs/CONFIG_SCHEMA.md) | Complete Lua API and configuration reference |
| [ARCHITECTURE.md](docs/ARCHITECTURE.md) | Crate layout, data flow, threading model |
| [DEVICE_MATCHING.md](docs/DEVICE_MATCHING.md) | How to identify and bind physical devices |
| [PERMISSIONS.md](docs/PERMISSIONS.md) | Group setup, udev rules, systemd service |
| [SECURITY.md](docs/SECURITY.md) | Threat model, Lua sandbox, IPC security |
| [BUILDING.md](docs/BUILDING.md) | Build from source, testing, fuzz testing |
| [CONTRIBUTING.md](docs/CONTRIBUTING.md) | Development workflow, adding actions, code style |
| [HYPRLAND_BINDINGS.md](docs/HYPRLAND_BINDINGS.md) | Hyprland keybinding examples |
| [extras/waybar/](extras/waybar/) | Waybar status module with themes |
| [examples/](examples/) | Example Lua configurations |

## Contributing

See [docs/CONTRIBUTING.md](docs/CONTRIBUTING.md). In short:

```sh
git clone https://github.com/nalcorso/wayclick.git
cd wayclick && ./scripts/dev.sh
# Make changes, then:
cargo test --workspace && cargo clippy --workspace -- -D warnings
```

## License

[MIT](LICENSE) © Nick Alcorso
