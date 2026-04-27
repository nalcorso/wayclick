# Config Schema Reference

Wayclick is configured with Lua scripts. The entry point is `init.lua`.

## Global Options

```lua
wayclick.set_options({
    dry_run = false,           -- Log actions instead of emitting real input events
    socket_path = nil,         -- Override IPC socket path (default: XDG_RUNTIME_DIR/wayclick.sock)
    log_capacity = 512,        -- Maximum number of log entries in the ring buffer
    min_interval_ms = 1,       -- Minimum allowed click interval in ms (prevents tight CPU loops)
})
```

## Triggers

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

Automatically switch layers based on the active window (Hyprland only):

```lua
wayclick.set_profile({
    name = "gaming",
    match_app = "steam_app_.*",    -- Regex on window app_id/class
    -- match_title = "Minecraft",  -- Regex on window title
    layer = "combat",              -- Auto-switch to this layer
})
```

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

## CLI (wayclickctl)

| Command                    | Description                        |
|----------------------------|------------------------------------|
| `wayclickctl status`       | Show daemon status                 |
| `wayclickctl toggle`       | Toggle automation on/off           |
| `wayclickctl enable`       | Enable automation                  |
| `wayclickctl disable`      | Disable automation                 |
| `wayclickctl trigger <id>` | Fire a trigger                     |
| `wayclickctl list`         | List all triggers                  |
| `wayclickctl reload`       | Reload configuration               |
| `wayclickctl logs`         | Show recent log entries            |
| `wayclickctl layer get`    | Show current active layer          |
| `wayclickctl layer set <name>` | Switch to a different layer    |
| `wayclickctl ping`         | Check if daemon is running         |
| `wayclickctl waybar`       | Output Waybar-compatible JSON      |

## SIGHUP Reload

The daemon reloads its configuration on `SIGHUP`:

```bash
systemctl --user reload wayclickd    # via systemd
kill -HUP $(pidof wayclickd)         # direct signal
```

This reloads the Lua config and restarts the evdev monitor with new bindings.
