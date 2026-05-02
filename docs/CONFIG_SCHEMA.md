# Lua API Reference

Wayclick is configured with Lua scripts. The entry point is `~/.config/wayclick/init.lua`.

The `wayclick` table is provided by the daemon — you don't `require` it. Your
script calls functions on it to register triggers, bind devices, and set options.
Everything is evaluated once at startup (or on hot-reload). There is no persistent
Lua runtime after config load.

---

## Table of Contents

- [Global Options](#global-options)
- [Triggers](#triggers)
  - [Trigger modes](#trigger-modes)
  - [Action types](#action-types)
    - [auto_click](#auto_click)
    - [click](#click)
    - [key_press](#key_press)
    - [keystroke](#keystroke)
    - [type_text](#type_text)
    - [scroll](#scroll)
    - [mouse_move](#mouse_move)
    - [composite (parallel)](#composite-parallel)
    - [composite (sequence)](#composite-sequence)
    - [delay](#delay)
    - [mouse_move_abs](#mouse_move_abs)
    - [click_at](#click_at)
    - [drag](#drag)
    - [set_layer](#set_layer)
    - [media_key](#media_key)
- [Device Bindings](#device-bindings)
  - [Binding Options](#binding-options)
- [Layers](#layers)
- [Per-App Profiles](#per-app-profiles)
- [Lua Modules](#lua-modules)
- [Key Names](#key-names)
  - [Media Keys](#media-keys)
- [CLI reference (wayclickctl)](#cli-reference-wayclickctl)
- [Config hot-reload](#config-hot-reload)
- [Complete Example Config](#complete-example-config)

---

## Global Options

```lua
wayclick.set_options({
    dry_run        = false,  -- Log actions instead of emitting real input events
    socket_path    = nil,    -- Override IPC socket path (default: $XDG_RUNTIME_DIR/wayclick.sock)
    log_capacity   = 512,    -- Ring buffer size for log entries (default: 512)
    min_interval_ms = 1,     -- Minimum click/key interval in ms (default: 1; must be ≥ 1)
})
```

**Interval limits:** The minimum action interval is `min_interval_ms` (configurable,
default 1ms). The maximum is a compile-time constant of **3,600,000ms (1 hour)**.
Values outside this range are rejected at config load time.

## Triggers

Triggers are the core unit of wayclick. Each trigger has an ID, a mode, and an
action. Triggers are activated by device bindings or fired programmatically via
IPC.

```lua
wayclick.register_trigger({
    id          = "my_trigger",    -- required: unique string identifier
    name        = "My Trigger",    -- optional: human-readable name (defaults to id)
    description = "",              -- optional: description for display in TUI/IPC
    mode        = "oneshot",       -- required: "toggle" | "hold" | "oneshot"
    action      = ...,             -- required: action function result
    cooldown_ms = 0,               -- optional: minimum ms between activations (default: 0)
    duration_ms = nil,             -- optional: auto-stop toggle/hold after N ms (default: unlimited)
})
```

### Trigger modes

| Mode | Behaviour |
|---|---|
| `toggle` | First activation starts the action loop; second activation stops it |
| `hold` | Action runs while the trigger is held; stops when released |
| `oneshot` | Runs the action once per activation |

**Oneshot-only constraint:** The actions `keystroke`, `type_text`, `click_at`,
`drag`, `mouse_move_abs`, and `set_layer` can only be used with `mode = "oneshot"`.
Using them with `toggle` or `hold` is a config error.

---

### auto_click

Rapidly clicks a mouse button at a fixed interval.

```lua
wayclick.register_trigger({
    id = "rapid_fire",
    mode = "toggle",          -- toggle | hold | oneshot
    action = wayclick.auto_click({
        button = "left",      -- left | right | middle | button4 | button5
        interval_ms = 10,     -- Milliseconds between clicks
        jitter_ms = 5,        -- Random ± jitter added to interval (anti-detection)
        hold_ms = 0,          -- Milliseconds to hold button down per click (0 = instant)
    }),
    cooldown_ms = 100,        -- Cooldown before this trigger can re-activate (ms)
    duration_ms = 5000,       -- Auto-stop after N ms (0 = unlimited)
})
```

### click

Performs a single click at the current cursor position. Useful for remapping
scroll wheel or other inputs to a single mouse click.

```lua
wayclick.register_trigger({
    id = "left_click",
    mode = "oneshot",
    action = wayclick.click({
        button = "left",      -- left | right | middle | button4 | button5
        hold_ms = 0,          -- Milliseconds to hold button down (0 = instant)
    }),
})
```

### key_press

Presses and releases a keyboard key repeatedly on a fixed interval.

```lua
wayclick.register_trigger({
    id = "press_a",
    mode = "toggle",
    action = wayclick.key_press({
        key = "a",            -- required: key name (e.g. "a", "F5", "KEY_Z")
        interval_ms = 1000,   -- optional: ms between presses (default: 1000)
        duration_ms = nil,    -- optional: stop after this many ms (default: runs until stopped)
        jitter_ms = 0,        -- optional: random ±jitter added to interval (default: 0)
        modifiers = {},       -- optional: list of modifier keys held during each press
    }),
})
```

To hold modifier keys during each auto-repeated key press:

```lua
wayclick.register_trigger({
    id = "repeat_ctrl_z",
    mode = "toggle",
    action = wayclick.key_press({
        key = "z",
        modifiers = {"ctrl"},
        interval_ms = 500,
    }),
})
```

To press multiple keys in sequence, use `wayclick.sequence`:

```lua
wayclick.register_trigger({
    id = "key_combo",
    mode = "oneshot",
    action = wayclick.sequence({
        actions = {
            wayclick.key_press({ key = "KEY_A" }),
            wayclick.delay({ ms = 50 }),
            wayclick.key_press({ key = "KEY_S" }),
            wayclick.delay({ ms = 50 }),
            wayclick.key_press({ key = "KEY_D" }),
        },
    }),
})
```

### keystroke

Sends a single key chord (one press-and-release, optionally with modifiers).
This action is **oneshot-only** and cannot be used with `toggle` or `hold` modes.

```lua
wayclick.register_trigger({
    id = "undo",
    mode = "oneshot",
    action = wayclick.keystroke({
        key = "z",              -- required: key name (e.g. "z", "F4", "KEY_ENTER")
        modifiers = {"ctrl"},   -- optional: list of modifier keys to hold (default: none)
        hold_ms = 0,            -- optional: ms to hold keys before releasing (default: 0)
    }),
})
```

Modifier keys are pressed before the main key and released in reverse order after.

**Common modifier aliases:**

| Alias                    | Key              |
|--------------------------|------------------|
| `"ctrl"`, `"control"`    | Left Ctrl        |
| `"shift"`                | Left Shift       |
| `"alt"`                  | Left Alt         |
| `"super"`, `"win"`, `"meta"` | Left Super   |
| `"altgr"`, `"ralt"`      | Right Alt        |
| `"rctrl"`                | Right Ctrl       |
| `"rshift"`               | Right Shift      |

Full `KEY_*` names (e.g. `"KEY_LEFTCTRL"`) are also accepted.

Examples:

```lua
-- Ctrl+Z (undo)
wayclick.keystroke({ key = "z", modifiers = {"ctrl"} })

-- Ctrl+Shift+Z (redo)
wayclick.keystroke({ key = "z", modifiers = {"ctrl", "shift"} })

-- Alt+F4 (close window)
wayclick.keystroke({ key = "F4", modifiers = {"alt"} })

-- Super key alone
wayclick.keystroke({ key = "KEY_LEFTMETA" })
```

### type_text

Types a string character by character using simulated keystrokes. Useful for chat commands,
macros, and any scenario where you need to type multiple characters in sequence.

> **Keyboard layout:** `type_text` uses a **US QWERTY** mapping. Characters are typed using the
> physical key positions on a US keyboard. If your system uses a different keyboard layout, the
> output may differ from the text you specify.

```lua
wayclick.type_text({
    text     = "/hideout",  -- required: string to type
    delay_ms = 30,          -- optional: ms between each keystroke (default: 30)
})
```

`type_text` returns a sequence action and can be composed with `wayclick.sequence()`.
It is **oneshot-only** (same restriction as `keystroke`).

**Supported characters:**

- Printable ASCII: `a`–`z`, `A`–`Z`, `0`–`9`, space, and all standard US QWERTY punctuation
- `\n` — types Enter (KEY_ENTER)
- `\t` — types Tab (KEY_TAB)
- All other characters (accented letters, emoji, etc.) produce a config load error

**Example: Path of Exile hideout macro**

Press F5 to open the chat, type `/hideout`, then send with Enter:

```lua
wayclick.register_trigger({
    id = "hideout",
    mode = "oneshot",
    action = wayclick.sequence({
        actions = {
            wayclick.keystroke({ key = "enter" }),           -- open chat
            wayclick.type_text({ text = "/hideout" }),       -- type command (30ms between chars)
            wayclick.keystroke({ key = "enter" }),           -- send command
        }
    }),
})

wayclick.bind_device({
    name = "keyboard",
    bindings = {
        { code = "KEY_F5", trigger = "hideout" },
    },
})
```

To speed up or slow down typing, adjust `delay_ms`:

```lua
wayclick.type_text({ text = "/hideout", delay_ms = 0 })   -- instant (no delay)
wayclick.type_text({ text = "/hideout", delay_ms = 100 })  -- slower, 100ms per key
```

### scroll

Scrolls in a direction.

```lua
wayclick.register_trigger({
    id = "scroll_down",
    mode = "hold",
    action = wayclick.scroll({
        direction = "down",   -- up | down | left | right
        amount = 3,
        interval_ms = 100,
    }),
})
```

### mouse_move

Moves the mouse cursor relative to its current position.

```lua
wayclick.register_trigger({
    id = "jiggle",
    mode = "oneshot",
    action = wayclick.mouse_move({
        dx = 5,
        dy = 0,
        interval_ms = 50,
    }),
})
```

### composite (parallel)

Run multiple actions simultaneously.

```lua
wayclick.register_trigger({
    id = "combo",
    mode = "toggle",
    action = wayclick.parallel({
        actions = {
            wayclick.auto_click({ button = "left", interval_ms = 10 }),
            wayclick.key_press({ key = "KEY_SPACE" }),
        },
    }),
})
```

### composite (sequence)

Run multiple actions one after another.

```lua
wayclick.register_trigger({
    id = "macro",
    mode = "oneshot",
    action = wayclick.sequence({
        actions = {
            wayclick.key_press({ key = "KEY_A" }),
            wayclick.delay({ ms = 100 }),
            wayclick.auto_click({ button = "left", interval_ms = 100 }),
        },
    }),
})
```

### delay

Pauses execution for a fixed duration. Useful between steps in a sequence.

```lua
wayclick.register_trigger({
    id = "timed_macro",
    mode = "oneshot",
    action = wayclick.sequence({
        actions = {
            wayclick.auto_click({ button = "left" }),
            wayclick.delay({ ms = 500 }),
            wayclick.auto_click({ button = "left" }),
        },
    }),
})
```

### mouse_move_abs

Moves the cursor to an absolute screen position (coordinates 0–32767).

```lua
wayclick.register_trigger({
    id = "center_cursor",
    mode = "oneshot",
    action = wayclick.mouse_move_abs({ x = 16383, y = 16383 }),
})
```

### click_at

Moves cursor to absolute position and clicks.

```lua
wayclick.register_trigger({
    id = "click_button",
    mode = "oneshot",
    action = wayclick.click_at({
        x = 1000, y = 500,
        button = "left",    -- Optional, default: "left"
        hold_ms = 0,        -- Optional, default: 0
        settle_ms = 5,      -- Optional, default: 5 (ms to wait after move before clicking)
    }),
})
```

### drag

Performs a mouse drag from one position to another with interpolated movement.

```lua
wayclick.register_trigger({
    id = "drag_item",
    mode = "oneshot",
    action = wayclick.drag({
        from_x = 100, from_y = 200,
        to_x = 500, to_y = 400,
        button = "left",        -- Optional, default: "left"
        duration_ms = 500,      -- Optional, default: 100
    }),
})
```

### set_layer

Switches the active binding layer. OneShot mode only.

```lua
wayclick.register_trigger({
    id = "switch_to_combat",
    mode = "oneshot",
    action = wayclick.set_layer({ layer = "combat" }),
})
```

### media_key

Convenience wrapper for `key_press` with media key names.

```lua
wayclick.register_trigger({
    id = "play_pause",
    mode = "oneshot",
    action = wayclick.media_key({ key = "play_pause" }),
})
```

## Trigger Modes

| Mode      | Behavior                                           |
|-----------|----------------------------------------------------|
| `toggle`  | First press starts, second press stops             |
| `hold`    | Active while button is held, stops on release      |
| `oneshot` | Executes once per press                            |

## Device Bindings

```lua
wayclick.bind_device({
    name = "G Pro",                  -- Match by name substring
    -- vid = 0x046d, pid = 0xc08b,   -- Match by vendor/product ID
    -- phys = "usb-...",             -- Match by physical location
    -- path = "/dev/input/event5",   -- Match by device path (deprecated)
    exclusive = false,               -- EVIOCGRAB exclusive access
    bindings = {
        -- Simple button trigger
        { code = "BTN_SIDE",  trigger = "rapid_fire" },

        -- Keyboard key trigger
        { code = "KEY_F1", trigger = "toggle_clicker" },

        -- Chord (multiple buttons pressed simultaneously)
        { code = "BTN_SIDE+BTN_EXTRA", trigger = "combo_action" },

        -- Hold duration (tap vs long-press)
        { code = "BTN_SIDE", trigger = "tap_action",
          hold_trigger = "hold_action", hold_ms = 500 },

        -- Layer-specific binding (only active in a specific layer)
        { code = "BTN_SIDE", trigger = "base_action", layer = "base" },
        { code = "BTN_SIDE", trigger = "combat_action", layer = "combat" },

        -- Scroll wheel remapping (requires exclusive = true)
        -- { scroll = "up",   trigger = "left_click" },
        -- { scroll = "down", trigger = "left_click" },
    },
})
```

### Binding Options

| Field          | Type     | Description                                              |
|----------------|----------|----------------------------------------------------------|
| `code`         | string   | Event code name(s). Use `+` for chords (e.g. `"BTN_SIDE+BTN_EXTRA"`) |
| `trigger`      | string   | Trigger ID to fire on press (or tap if `hold_trigger` set) |
| `hold_trigger` | string?  | Trigger ID to fire on long-press (requires `hold_ms`)    |
| `hold_ms`      | number?  | Hold threshold in ms (requires `hold_trigger`)           |
| `layer`        | string?  | Only active when this layer is current (nil = all layers)|
| `scroll`       | string?  | Scroll direction: `"up"`, `"down"`, `"left"`, `"right"`. Requires `exclusive = true`. Use instead of `code` for scroll bindings |

## Layers

Layers allow different bindings to be active at different times.

```lua
-- Switch layer via action
wayclick.register_trigger({
    id = "enter_combat",
    mode = "oneshot",
    action = wayclick.set_layer({ layer = "combat" }),
})

-- Layer can also be set via CLI: wayclickctl layer set combat
```

The default layer is `"base"`. Layer state persists across config reloads.

## Per-App Profiles

> **⚠️ Not yet implemented.** The `wayclick.set_profile()` function parses and
> stores profile rules without error, but the engine does not act on them at
> runtime. Automatic layer switching based on window focus is planned — see
> [ROADMAP.md](ROADMAP.md).

The API is defined for forward compatibility:

```lua
wayclick.set_profile({
    name       = "gaming",
    match_app  = "steam_app_.*",    -- regex on window app_id/class
    -- match_title = "Minecraft",   -- regex on window title
    layer      = "combat",          -- intended: auto-switch to this layer
})
```

In the meantime, use `wayclickctl layer set <name>` from a window-focus hook
(e.g., a Hyprland `windowfocused` event handler) to achieve the same effect.

## Lua Modules

Place helper Lua files in `~/.config/wayclick/lua/`. They can be loaded with
`require()`:

```lua
-- ~/.config/wayclick/lua/my_triggers.lua
local M = {}
M.my_action = wayclick.auto_click({ button = "left", interval_ms = 20 })
return M
```

```lua
-- init.lua
local my = require("my_triggers")
wayclick.register_trigger({ id = "fast", mode = "toggle", action = my.my_action })
```

## Key Names

Key names follow the Linux `KEY_*` constants:

| Name         | Code |
|--------------|------|
| `KEY_A`–`KEY_Z` | 30–44, etc. |
| `KEY_1`–`KEY_0` | 2–11 |
| `KEY_F1`–`KEY_F12` | 59–68, 87–88 |
| `KEY_SPACE`  | 57 |
| `KEY_ENTER`  | 28 |
| `KEY_ESC`    | 1 |
| `KEY_TAB`    | 15 |
| `KEY_LEFTSHIFT` | 42 |
| `KEY_LEFTCTRL` | 29 |
| `KEY_LEFTALT` | 56 |

### Media Keys

| Name               | Code | Constant             |
|--------------------|------|----------------------|
| `KEY_MUTE`         | 113  | `wayclick.keys.MUTE` |
| `KEY_VOLUMEDOWN`   | 114  | `wayclick.keys.VOLUME_DOWN` |
| `KEY_VOLUMEUP`     | 115  | `wayclick.keys.VOLUME_UP` |
| `KEY_NEXTSONG`     | 163  | `wayclick.keys.NEXT_SONG` |
| `KEY_PLAYPAUSE`    | 164  | `wayclick.keys.PLAY_PAUSE` |
| `KEY_PREVIOUSSONG` | 165  | `wayclick.keys.PREVIOUS_SONG` |
| `KEY_STOPCD`       | 166  | `wayclick.keys.STOP_CD` |
| `KEY_RECORD`       | 167  | `wayclick.keys.RECORD` |
| `KEY_REWIND`       | 168  | `wayclick.keys.REWIND` |
| `KEY_FASTFORWARD`  | 208  | `wayclick.keys.FAST_FORWARD` |
| `KEY_BRIGHTNESSDOWN` | 224 | `wayclick.keys.BRIGHTNESS_DOWN` |
| `KEY_BRIGHTNESSUP` | 225  | `wayclick.keys.BRIGHTNESS_UP` |

See the full list with `wayclick-evdev-dump monitor`.

## CLI reference (wayclickctl)

See the README for the full table. Key commands:

| Command | Description |
|---|---|
| `wayclickctl status` | Show daemon state, active layer, trigger count |
| `wayclickctl ping` | Check if the daemon is running |
| `wayclickctl toggle` | Toggle automation on/off |
| `wayclickctl enable` / `disable` | Enable or disable automation |
| `wayclickctl trigger <id>` | Fire a trigger by ID |
| `wayclickctl list` | List all triggers with current state |
| `wayclickctl reload` | Reload configuration from disk |
| `wayclickctl logs [--tail N]` | Show recent log entries |
| `wayclickctl layer get` | Show current active layer |
| `wayclickctl layer set <name>` | Switch to a named layer |
| `wayclickctl waybar [--continuous]` | Output Waybar-compatible JSON |

## Config hot-reload

The daemon reloads its configuration on `SIGHUP` or via the CLI:

```sh
wayclickctl reload                  # via CLI
systemctl --user reload wayclickd   # via systemd
kill -HUP $(pidof wayclickd)        # direct signal
```

Hot-reload restarts the evdev monitor with the new bindings. Running triggers
are stopped before reload. The active layer is preserved.

---

## Complete Example Config

Below is a real, working configuration that demonstrates multiple components
working together: global options, multiple trigger types, device bindings with
different binding types, layer management, and proper commenting for clarity.

```lua
-- ~/.config/wayclick/init.lua
-- Complete example showcasing global options, triggers, devices, and layers

-- ============================================================================
-- GLOBAL CONFIGURATION
-- ============================================================================
wayclick.set_options({
  dry_run        = false,          -- Set to true for testing without emitting real events
  socket_path    = nil,            -- Use default socket at $XDG_RUNTIME_DIR/wayclick.sock
  log_capacity   = 512,            -- Ring buffer size for log entries
  min_interval_ms = 1,             -- Minimum interval between events (ms)
})

-- ============================================================================
-- LAYER DEFINITIONS
-- ============================================================================
-- Define two layers: "gaming" (with auto-clicker) and "normal" (disabled)
-- Start in the "normal" layer by default.

wayclick.set_profile({
  layers = {
    normal  = { name = "Normal",  is_default = true  },
    gaming  = { name = "Gaming",  is_default = false },
  },
})

-- ============================================================================
-- TRIGGER: Auto-click toggle (gaming layer)
-- ============================================================================
-- Toggle rapid left-clicking on/off. Only enabled in gaming layer.
-- - First press starts the loop
-- - Second press stops it
-- - Fires every 10ms with ±5ms jitter for anti-detection

wayclick.register_trigger({
  id          = "auto_clicker",
  name        = "Auto Clicker",
  description = "Rapid left-click toggle for gaming",
  mode        = "toggle",
  cooldown_ms = 300,              -- Minimum 300ms before retrigger
  duration_ms = nil,              -- Run forever (until manually stopped)
  action      = wayclick.auto_click({
    button      = "left",
    interval_ms = 10,
    jitter_ms   = 5,
    hold_ms     = 2,              -- Hold button for 2ms per click
  }),
})

-- ============================================================================
-- TRIGGER: Scroll-to-click remap
-- ============================================================================
-- Single click per scroll notch. Fast scrolling fires multiple times.
-- Useful for ARPGs and click-intensive games.

wayclick.register_trigger({
  id          = "scroll_click",
  name        = "Scroll Click",
  description = "Remap scroll wheel to left-click",
  mode        = "oneshot",
  action      = wayclick.click({ button = "left" }),
})

-- ============================================================================
-- TRIGGER: Right-click
-- ============================================================================

wayclick.register_trigger({
  id     = "right_click",
  name   = "Right Click",
  mode   = "oneshot",
  action = wayclick.click({ button = "right" }),
})

-- ============================================================================
-- TRIGGER: Layer switch helper (oneshot)
-- ============================================================================
-- Jump to the "gaming" layer with a keybind

wayclick.register_trigger({
  id     = "switch_to_gaming",
  name   = "Switch to Gaming",
  mode   = "oneshot",
  action = wayclick.set_layer("gaming"),
})

-- ============================================================================
-- TRIGGER: Layer switch helper (oneshot)
-- ============================================================================
-- Jump back to "normal" layer

wayclick.register_trigger({
  id     = "switch_to_normal",
  name   = "Switch to Normal",
  mode   = "oneshot",
  action = wayclick.set_layer("normal"),
})

-- ============================================================================
-- DEVICE BINDINGS: Gaming Mouse
-- ============================================================================
-- Bind to a Logitech G Pro mouse (or by VID:PID for portability).
-- In gaming layer: MB5 (forward) toggles auto-click, scroll fires left-click
-- exclusive=true prevents the OS from seeing these events.

wayclick.bind_device({
  -- Match by device name
  name = "Logitech USB Receiver Mouse",
  
  -- Optionally match by vendor:product ID instead:
  -- vid = 0x046d, pid = 0xc08b,
  
  exclusive = true,               -- Grab the device exclusively
  
  bindings = {
    -- Button bindings
    { code = "BTN_EXTRA",  trigger = "auto_clicker" },  -- MB5 (forward)
    { code = "BTN_SIDE",   trigger = "right_click"  },  -- MB4 (back)
    
    -- Scroll bindings
    { scroll = "up",   trigger = "scroll_click" },
    { scroll = "down", trigger = "scroll_click" },
  },
})

-- ============================================================================
-- DEVICE BINDINGS: Keyboard
-- ============================================================================
-- Layer switching via keyboard shortcuts (non-exclusive).
-- These don't prevent the OS from seeing the key events.

wayclick.bind_device({
  name = "AT Translated Set 2 keyboard",  -- Most Linux systems
  exclusive = false,
  bindings = {
    -- F1 = switch to normal layer
    { code = "KEY_F1", trigger = "switch_to_normal" },
    -- F2 = switch to gaming layer
    { code = "KEY_F2", trigger = "switch_to_gaming" },
  },
})

-- ============================================================================
-- NOTES FOR CUSTOMIZATION
-- ============================================================================
--
-- 1. Find your device name with: wayclickctl devices
--
-- 2. Find button/key codes with: wayclick-evdev-dump monitor
--    Then click/press the button you want to bind.
--
-- 3. Common button codes:
--    - BTN_LEFT, BTN_RIGHT, BTN_MIDDLE (main buttons)
--    - BTN_SIDE, BTN_EXTRA (back/forward)
--    - BTN_FORWARD, BTN_BACK (alternate names)
--
-- 4. Common key codes:
--    - KEY_F1 through KEY_F12 (function keys)
--    - KEY_A through KEY_Z (letters)
--    - KEY_SPACE, KEY_ENTER, KEY_ESC, etc.
--
-- 5. Hot-reload your config:
--    wayclickctl reload
--    Or send SIGHUP to the daemon:
--    kill -HUP $(pidof wayclickd)
```

This example demonstrates:
- **Global options:** DRY_RUN disabled, default socket path, standard log capacity
- **Layers:** Two named layers (gaming/normal) for context-aware configs
- **Multiple trigger types:** Toggle (auto-click), oneshot (click, layer switch)
- **Device bindings:** Exclusive gaming mouse + non-exclusive keyboard layer switching
- **Documentation:** Inline comments explain each section and how to customize
