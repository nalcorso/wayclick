# Config Schema Reference

Wayclick is configured with Lua scripts. The entry point is `init.lua`.

## Global Options

```lua
wayclick.set_global {
    dry_run = false,       -- Log actions instead of emitting real input events
    socket_path = nil,     -- Override IPC socket path (default: XDG_RUNTIME_DIR/wayclick.sock)
    log_level = "info",    -- Minimum log level: trace, debug, info, warn, error
    cooldown_ms = 50,      -- Default cooldown after a trigger deactivates (ms)
}
```

## Triggers

### auto_click

Rapidly clicks a mouse button at a fixed interval.

```lua
wayclick.trigger {
    id = "rapid_fire",
    mode = "toggle",          -- toggle | hold | oneshot
    action = wayclick.auto_click {
        button = "left",      -- left | right | middle | button4 | button5
        interval_ms = 10,     -- Milliseconds between clicks
        jitter_ms = 5,        -- Random ± jitter added to interval (anti-detection)
        hold_ms = 0,          -- Milliseconds to hold button down per click (0 = instant)
    },
    cooldown_ms = 100,        -- Override global cooldown for this trigger
    duration_ms = 5000,       -- Auto-stop after N ms (0 = unlimited)
}
```

### key_sequence

Presses and releases a sequence of keys.

```lua
wayclick.trigger {
    id = "key_matrix",
    mode = "toggle",
    action = wayclick.key_sequence {
        keys = { "KEY_A", "KEY_S", "KEY_D", "KEY_F" },
        interval_ms = 50,
        jitter_ms = 10,
    },
}
```

### scroll

Scrolls in a direction.

```lua
wayclick.trigger {
    id = "scroll_down",
    mode = "hold",
    action = wayclick.scroll {
        direction = "down",   -- up | down | left | right
        amount = 3,
        interval_ms = 100,
    },
}
```

### mouse_move

Moves the mouse cursor relative to its current position.

```lua
wayclick.trigger {
    id = "jiggle",
    mode = "oneshot",
    action = wayclick.mouse_move {
        dx = 5,
        dy = 0,
        interval_ms = 50,
    },
}
```

### composite (parallel)

Run multiple actions simultaneously.

```lua
wayclick.trigger {
    id = "combo",
    mode = "toggle",
    action = wayclick.parallel {
        wayclick.auto_click { button = "left", interval_ms = 10 },
        wayclick.key_sequence { keys = { "KEY_SPACE" }, interval_ms = 200 },
    },
}
```

### composite (sequence)

Run multiple actions one after another.

```lua
wayclick.trigger {
    id = "macro",
    mode = "oneshot",
    action = wayclick.sequence {
        wayclick.key_sequence { keys = { "KEY_A" }, interval_ms = 50, repeat = 3 },
        wayclick.auto_click { button = "left", interval_ms = 100, repeat = 5 },
    },
}
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
M.my_action = wayclick.auto_click { button = "left", interval_ms = 20 }
return M
```

```lua
-- init.lua
local my = require("my_triggers")
wayclick.trigger { id = "fast", mode = "toggle", action = my.my_action }
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

## SIGHUP Reload

The daemon reloads its configuration on `SIGHUP`:

```bash
systemctl reload wayclickd    # via systemd
kill -HUP $(pidof wayclickd)  # direct signal
```

This reloads the Lua config and restarts the evdev monitor with new bindings.
