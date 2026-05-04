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

**AI Development** — This project was developed with AI assistance (Claude via GitHub Copilot) under human direction. Architecture decisions, security model, and feature design were human-directed; code generation and documentation were AI-assisted. Threat model and security decisions were thoroughly reviewed. See [CHANGELOG.md](CHANGELOG.md) for project history.

**Language Choice** — Wayclick is written in Rust for practical reasons: performance, memory safety, and a mature ecosystem for systems programming. This is a pragmatic choice, not an ideological one.

---

## What wayclick does

- **Scroll-to-click** — remap the scroll wheel to mouse clicks (the classic ARPG technique: scroll to rapid-fire left-click without wearing out the button)
- **Auto-click** — toggle rapid clicking on/off with a side button, with configurable interval, jitter, and hold duration
- **Macros** — type text, fire keystroke sequences, chain any combination of actions with delays
- **Layer switching** — maintain separate binding sets and switch between them at runtime (base layer, combat layer, menu layer)
- **Button chording** — bind actions to multi-button combos
- **IPC control** — connect via Unix socket from scripts, game plugins, or external tools to register triggers and subscribe to events

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
sudo cp udev/99-wayclick.rules /etc/udev/rules.d/
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
cp systemd/wayclickd.service ~/.config/systemd/user/
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

| Mode | Behaviour |
|---|---|
| `toggle` | First press starts, second stops |
| `hold` | Active while button is held; stops on release |
| `oneshot` | Fires once per press |

**Common actions:**

| Action | Mode constraint | Description |
|---|---|---|
| `click` | oneshot | Single mouse click |
| `auto_click` | toggle/hold/oneshot | Repeated clicks at an interval |
| `keystroke` | oneshot only | Single key chord (key + modifiers) |
| `type_text` | oneshot only | Type a string character by character |
| `key_press` | toggle/hold/oneshot | Repeated key press at an interval |
| `scroll` | toggle/hold/oneshot | Scroll wheel output |
| `mouse_move` | toggle/hold/oneshot | Relative cursor movement |
| `sequence` | any | Chain actions one after another with delays |
| `delay` | — | Pause for a duration (use inside `sequence`) |

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
