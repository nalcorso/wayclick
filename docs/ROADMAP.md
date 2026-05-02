# Wayclick Roadmap

This document provides a clear picture of wayclick's feature status:

- **Stable Features** — Thoroughly tested, documented, and production-ready
- **Known Limitations** — Accepted gaps; workarounds exist or features are low-priority
- **In Development** — Currently being worked on; expect changes
- **Planned for Future** — High-priority features not yet started
- **Recently Completed** — Recently shipped and considered stable

**Status summary:**
- ✅ **Implemented** — Works reliably; API is stable
- 🔧 **In Development** — Active work; API may change
- 📋 **Planned** — High-priority; not yet started
- ⚠️ **Limited** — Known gaps; workarounds available
- ❌ **Out of Scope** — Won't implement

---

## Stable Features

### ✅ Implemented and Production-Ready

These work reliably and the API is stable:

- **Scroll-to-click** — remap scroll wheel to any action (`exclusive = true`
  required); magnitude-aware (fast scrolling fires multiple times)
- **Auto-click** — toggle/hold rapid clicking with configurable interval,
  jitter, and hold duration
- **key_press** — toggle/hold repeated key pressing with modifiers
- **keystroke** — single key chord (oneshot-only)
- **type_text** — type a string character by character (US QWERTY, oneshot-only)
- **Sequences and delays** — chain any actions with configurable timing
- **Parallel actions** — run multiple action loops simultaneously
- **Layers** — switch binding sets at runtime (`set_layer`, `wayclickctl layer set`)
- **Button chording** — bind multi-button combos (requires `exclusive = true`)
- **Tap vs long-press** — different triggers for tap and hold on the same button
- **IPC control** — full JSON-RPC 2.0 over Unix socket
- **Dynamic triggers** — register/unregister triggers via IPC; owned by the
  connection, automatically cleaned up on disconnect
- **Event subscriptions** — subscribe to push events (trigger fired, layer
  changed, enabled/disabled, config reloaded)
- **Hot-reload** — `SIGHUP` or `wayclickctl reload`
- **Dry-run mode** — log all actions without emitting real events
- **TUI dashboard** — real-time trigger state and log viewer
- **Waybar integration** — event-driven status module with CSS themes, trigger flash, layer cycling, per-trigger dots

---

## Known Limitations

These are accepted gaps that affect real use cases. Some have workarounds; others are low-priority.

### ⚠️ type_text is US QWERTY only

`wayclick.type_text()` maps characters to physical key positions on a US
QWERTY keyboard. If your system uses a different layout (AZERTY, QWERTZ,
Dvorak, etc.), the characters emitted will not match what you type.

**Workaround:** Use individual `keystroke` calls with explicit `KEY_*` names.
The raw key codes are layout-independent.

**Planned fix:** Layout-aware type_text using XKB.

### ⚠️ Profile rules (set_profile) are not fully implemented

`wayclick.set_profile()` parses the config without error but automatic layer
switching based on the active window is not yet implemented.

**Workaround:** Use a compositor event hook (e.g., a Hyprland `windowfocused`
handler) that calls `wayclickctl layer set <name>`.

### ⚠️ Chord detection requires exclusive mode

Button chords (`"BTN_SIDE+BTN_EXTRA"`) only work when `exclusive = true` on the
device binding. Without exclusive access, wayclick cannot suppress the individual
button events, so both the chord and the individual buttons would fire.

### ⚠️ Dynamic triggers are connection-scoped

Dynamic triggers registered via IPC are tied to the socket connection that
created them. When the connection closes (clean or abrupt), all its triggers are
removed. There is no way to make a dynamic trigger persist across reconnects.

**Design note:** This is intentional — it prevents orphaned triggers from a
crashed client. Persistence is planned as an opt-in feature.

### ⚠️ No action recording

There is no way to record a sequence of button presses and replay them as a
macro. All configs must be written by hand.

---

## Recently Completed

### ✅ Per-trigger enable/disable

`wayclickctl trigger enable/disable <id>` and the IPC methods
`enable_trigger` / `disable_trigger` are now available. Individual triggers can
be disabled without affecting others.

**Alternative:** Use layers — put triggers you want to selectively disable on a
separate layer and switch away from it.

### ✅ wayclickctl CLI tools for config checking and layer management

`wayclickctl check-config <path>`, `wayclickctl layer list`,
`wayclickctl layer cycle [--backward]`, and `wayclickctl watch [--json]` are
now available. You no longer need to use IPC directly for these common tasks.

---

## Planned for Future

### 📋 Wayland window-focus layer switching

Implement automatic layer switching based on the focused window. This requires
compositor integration (e.g., querying `hyprctl activeworkspace` or subscribing
to `ext-foreign-toplevel-list-v1` for compositor-independent support).

This will make `set_profile()` functional.

### 📋 Layout-aware type_text

Detect the active keyboard layout via XKB and map characters to the correct
physical keys. This makes `type_text` usable on non-US keyboard layouts.

### 📋 Persistent dynamic triggers

Allow dynamic triggers to be marked as persistent when registered via IPC. A
persistent trigger survives connection close and is re-registered automatically
on reconnect (keyed by client ID + trigger ID).

### 📋 Multi-device chording

Allow chords that span multiple physical devices — for example, holding a
keyboard key while pressing a mouse button. Currently chords are limited to
buttons on the same device.

### 📋 Per-trigger burst-fire controls

More granular controls for scroll remapping: per-trigger rate limiting,
maximum burst size, and cooldown periods.

### 📋 Packaged binaries

Packages for AUR (Arch), Nix, and common distros. Currently source-only.

---

## API Stability Commitments

### ✅ Stable — No breaking changes planned

- Lua API: `wayclick.register_trigger`, `wayclick.bind_device`, `wayclick.set_options`
- All action functions and their parameter names
- IPC JSON-RPC method names and parameter schemas
- Event type names and payload fields
- `wayclickctl` subcommand names and flags

### 🔧 May change

- `wayclick.set_profile()` — the API signature may change when this feature is
  implemented, since the current stub parameters may not match the final design
- IPC response payload shapes (additional fields may be added; existing fields
  will not be removed)
- Internal config file format (the Lua API is stable, but the serialized
  `ActionConfig` JSON used internally for IPC `register_trigger` may evolve)

### ❌ Deliberately out of scope

- **GUI configurator** — wayclick is a daemon for power users; the Lua + TUI
  combination is the intended interface
- **Windows / macOS support** — wayclick is a Linux kernel interface tool;
  evdev and uinput are Linux-specific
- **Network-accessible IPC** — the Unix socket is user-local by design; adding
  TCP or HTTP would expand the attack surface with no benefit for the target use case
- **Plugin marketplace** — configs are Lua scripts shared as files; no package
  registry is planned
