# wayclick

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Rust: 1.85+](https://img.shields.io/badge/rust-1.85%2B-orange.svg)](https://www.rust-lang.org/)

**Kernel-level input automation for Linux.**

Wayclick reads physical button presses directly from the kernel's input layer
(evdev) and fires configurable actions through a virtual device (uinput). It
doesn't care which display server you're running — Wayland, X11, or none at
all. Write your config in Lua. Run it as a systemd service. Automate everything.

---

### Disclosures

**AI Development** — This project was developed with AI assistance (Claude via GitHub Copilot) under human direction. Architecture decisions, security model, and feature design were human-directed; code generation and documentation were AI-assisted. Threat model and security decisions were thoroughly reviewed. See [CHANGELOG.md](CHANGELOG.md) for project history. The commit history was collapsed when I decided to make this project public - this was a toy project with a lot of hard coded / personal information.

**Language Choice** — Wayclick is written in Rust for no good reason, I flipped a coin. My only ideological view when it comes to language decisions is 'Pick the right tool for the job'. If that turns out to be somthing different, I am on board.

---

## Features

- **Kernel-level input binding** — Direct access to hardware buttons via Linux evdev; intercept and modify input at the kernel level before the OS sees it

- **Flexible action composition** — Chain clicks, keystrokes, delays, mouse movements, and scrolling into complex automation sequences (e.g., scroll-to-click, rapid-fire automation, text macros)

- **Per-device binding** — Match and bind any input device independently by name, VID:PID, or physical path; different devices can have completely different bindings

- **Context switching** — Switch between entirely different binding sets at runtime (e.g., accessibility mode, gaming mode, work mode—all in one config)

- **Programmatic control** — Register and unregister triggers dynamically via Unix socket IPC; integrate with external scripts, game plugins, or accessibility tools without restarting

- **Display-server agnostic** — Works unchanged on Wayland, X11, or headless Linux; same configuration and behavior everywhere

- **Hot-reload configuration** — Edit your Lua config and reload the daemon without restarting; dry-run mode for testing automation without emitting real events

- **Anti-pattern features** — Optional jitter and timing randomization to avoid pattern detection in games with anti-cheat monitoring

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
        │            │       ▼  │
        ▼            │    IpcServer ◄──► wayclickctl / wayclick-tui
   [button event] ────┘        │
                            Unix socket
```

---

## Quick start

### 1. Install

**From source:**

```sh
git clone https://github.com/nalcorso/wayclick.git
cd wayclick
cargo install --path crates/wayclickd
cargo install --path crates/wayclickctl
cargo install --path crates/wayclick-tui
cargo install --path crates/wayclick-evdev-dump
```

**Build requirements:**
- Rust 1.85+ (via [rustup](https://rustup.rs))
- Linux with kernel support for uinput and evdev
- `gcc` or `clang` (for vendored Lua build)

### 2. Set up permissions

Wayclick needs access to `/dev/input/event*` and `/dev/uinput`. Run the install script:

```sh
./scripts/install.sh
# Log out and back in for group changes to take effect
```

**Or manually:**

```sh
# Create wayclick group for /dev/uinput access
sudo groupadd -f wayclick
sudo usermod -aG wayclick "$USER"

# Install udev rules
sudo cp deployment/udev/99-wayclick.rules /etc/udev/rules.d/
sudo udevadm control --reload && sudo udevadm trigger

# Add yourself to input group for /dev/input/event* access
sudo usermod -aG input "$USER"

# Apply group changes
newgrp wayclick
```

**Verify permissions:**

```sh
wayclickd --check-permissions
```

### 3. Identify your device

```sh
wayclick-evdev-dump identify
```

Press a button on your device. The tool will print:

```
=== DEVICE IDENTIFIED ===
  Path:    /dev/input/event5
  Name:    Logitech G Pro Gaming Mouse
  VID:PID: 046d:c08b
  Button:  BTN_EXTRA (code=276)

Lua examples:
  wayclick.bind_device({ name = "Logitech G Pro Gaming Mouse" })
  wayclick.bind_device({ vid = 0x046d, pid = 0xc08b })
```

### 4. Create your config

Create `~/.config/wayclick/init.lua`. Here's a complete scroll-to-click setup:

```lua
-- Remap scroll wheel to left-click
wayclick.register_trigger({
  id    = "left_click",
  mode  = "oneshot",
  action = wayclick.click({ button = "left" }),
})

wayclick.bind_device({
  name      = "G Pro",       -- substring match on device name
  exclusive = true,          -- suppress original scroll events
  bindings  = {
    { scroll = "up",   trigger = "left_click" },
    { scroll = "down", trigger = "left_click" },
  },
})
```

### 5. Start the daemon

```sh
# Check your config loads correctly
wayclickd --check-config ~/.config/wayclick/init.lua

# Start the daemon
wayclickd

# Or with systemd (runs at login)
cp deployment/systemd/wayclickd.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now wayclickd
```

---

## Writing your first config

### Example 1: Auto-clicker

Toggle rapid clicking with a side mouse button:

```lua
wayclick.register_trigger({
  id   = "auto_clicker",
  name = "Auto Clicker",
  mode = "toggle",        -- first press starts, second stops
  action = wayclick.auto_click({
    button      = "left",
    interval_ms = 50,     -- 20 clicks per second
    jitter_ms   = 5,      -- ±5ms random variation (anti-detection)
    hold_ms     = 5,      -- hold button 5ms per click
  }),
  cooldown_ms = 200,      -- minimum time between toggle presses
})

wayclick.bind_device({
  vid      = 0x046d,
  pid      = 0xc08b,      -- your device VID:PID
  bindings = {
    { code = "BTN_EXTRA", trigger = "auto_clicker" },
  },
})
```

### Example 2: Macro (typing command)

Press F5 to open chat, type a command, and press Enter:

```lua
wayclick.register_trigger({
  id   = "hideout",
  mode = "oneshot",
  action = wayclick.sequence({
    actions = {
      wayclick.keystroke({ key = "enter" }),            -- open chat
      wayclick.delay({ ms = 50 }),                      -- wait for chat
      wayclick.type_text({ text = "/hideout" }),        -- type command
      wayclick.delay({ ms = 30 }),
      wayclick.keystroke({ key = "enter" }),            -- send
    },
  }),
})

wayclick.bind_device({
  name     = "keyboard",
  bindings = {
    { code = "KEY_F5", trigger = "hideout" },
  },
})
```

### Configuration essentials

**Trigger modes:**

| Icon | Mode | Behaviour |
|---|---|---|
| 🔄 | toggle | First press starts, second stops |
| 👆 | hold | Active while button is held; stops on release |
| 💥 | oneshot | Fires once per press |

**All actions:**

| Action | Mode constraint | Description |
|---|---|---|
| `click` | 💥 oneshot | Single mouse click |
| `auto_click` | All | Repeated clicks at an interval with configurable jitter and hold duration |
| `keystroke` | 💥 oneshot | Single key chord (key + optional modifiers) |
| `type_text` | 💥 oneshot | Type a string character by character |
| `key_press` | All | Repeated keyboard key presses at an interval |
| `scroll` | All | Scroll wheel output |
| `mouse_move` | All | Relative cursor movement |
| `mouse_move_abs` | 💥 oneshot | Absolute cursor positioning (optional `monitor = "DP-2"` for per-output coords) |
| `click_at` | 💥 oneshot | Move to absolute position and click (optional `monitor = "DP-2"` for per-output coords) |
| `drag` | 💥 oneshot | Click-drag between two positions |
| `set_layer` | 💥 oneshot | Switch active binding layer |
| `media_key` | 💥 oneshot | Press a media key (volume, play/pause, etc.) |
| `sequence` | All | Run actions one after another with delays; loops while active under `toggle`/`hold` modes (oneshot runs once) |
| `parallel` | 🔄 toggle / 👆 hold | Run actions simultaneously |
| `delay` | N/A | Pause for a fixed duration (use inside `sequence`) |

**Device matching options:**

```lua
-- By name (substring, case-insensitive)
wayclick.bind_device({ name = "G Pro" })

-- By vendor/product ID
wayclick.bind_device({ vid = 0x046d, pid = 0xc08b })

-- By physical location
wayclick.bind_device({ phys = "usb-0000:00:14.0" })

-- Exclusive mode (required for scroll remapping; suppresses original events)
wayclick.bind_device({ name = "mouse", exclusive = true })
```

### Testing and debugging

```sh
# List all configured triggers
wayclickctl list

# Check daemon status
wayclickctl status

# View live logs
wayclickctl logs --tail 50

# Hot-reload after editing config
wayclickctl reload

# View live dashboard with trigger state
wayclickctl tui

# Dry-run mode (logs actions without emitting real events)
wayclickd --dry-run
```

### More examples

See [`examples/`](examples/) for ready-to-use configs (scroll remapping, morse clicker, etc.).
Read [`config/init.lua`](config/init.lua) for the default config with annotations.

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

Wayclick exposes a Unix socket with a JSON-RPC 2.0 protocol. Any language that can open a socket can control it. Connect from game plugins, scripts, or other daemons to register dynamic triggers, subscribe to events, or query status.

```sh
# Quick test — is the daemon alive?
echo '{"jsonrpc":"2.0","id":1,"method":"ping","params":{}}' | \
  nc -U "$XDG_RUNTIME_DIR/wayclick.sock" | head -c 1000
```

### Backend-specific methods

The daemon exposes two backend-specific helpers that the `wayclick-recorder`
crate uses to translate captured input into Lua. Both are currently only
implemented for the Hyprland focus tracker; other backends respond with
JSON-RPC error code `-32001` (`JSONRPC_UNSUPPORTED`).

| Method | Returns | Notes |
| :--- | :--- | :--- |
| `get_cursor_position` | `{ x, y }` global pixels | Hyprland only; transient failures use `-32000`. |
| `get_monitors` | `{ monitors: [{ name, x, y, width, height, scale, transform, description }] }` | Logical (post-scale) pixels. Hyprland only. |

Typed helpers `SyncClient::get_cursor_position` / `SyncClient::get_monitors`
collapse the unsupported / transient codes to `Ok(None)` so callers can
degrade gracefully.

---

## Multi-monitor support

`click_at` and `mouse_move_abs` accept an optional `monitor` field for
multi-output setups:

```lua
-- Coordinates resolve relative to DP-2's logical origin.
wayclick.click_at({ x = 100, y = 200, monitor = "DP-2" })
```

Omit `monitor` and the coordinates are interpreted in global compositor
pixels (the previous behaviour). Monitor names are reported by
`hyprctl monitors -j` (Hyprland) and via the `get_monitors` IPC method.

Under the hood the daemon prefers the Wayland `zwlr_virtual_pointer_v1`
protocol when the compositor supports it (Hyprland, Wayfire, etc.). This
is the only pointer backend that can correctly target multi-monitor
layouts — a single uinput absolute-axis pointer cannot represent a
non-rectangular union of outputs. If the protocol is unavailable
(currently Sway), the daemon falls back to uinput and logs a warning;
`click_at` may then miss the correct output on multi-monitor setups.

---

## Waybar integration

A status module for [Waybar](https://github.com/Alexays/Waybar) is included in [`extras/waybar/`](extras/waybar/).

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

Three display formats are available: `minimal` (icon only), `normal` (icon + layer name), and `verbose` (icon + layer + active trigger count). Four CSS themes are included. See the [module README](extras/waybar/README.md) for setup instructions.

---

## Security

- **No network access** — local-only Unix socket IPC with `0600` permissions
- **Sandboxed Lua** — dangerous functions removed (`os.execute`, `io.popen`, `load`, `debug`); `io.open` restricted to config directory, read-only
- **Least privilege** — runs as your user; needs only `wayclick` + `input` groups
- **Fuzz-tested** — config loading, IPC framing, and device matching are all fuzz targets

---

## Components

| Binary | Purpose |
|---|---|
| `wayclickd` | Daemon — reads devices, executes triggers, serves IPC |
| `wayclickctl` | CLI — control the running daemon |
| `wayclick-tui` | TUI — real-time dashboard with trigger state and logs |
| `wayclick-evdev-dump` | Diagnostic — list devices, monitor events, identify buttons |
| `wayclick-playground` | Visual testing — GPU-accelerated input visualizer (see [`extras/wayclick-playground/`](extras/wayclick-playground/)) |
| `wayclick-recorder` | Macro recording — captures input via IPC and emits replayable Lua snippets (see [`extras/wayclick-recorder/`](extras/wayclick-recorder/)) |

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

---

## License

[MIT](LICENSE) © Nick Alcorso
