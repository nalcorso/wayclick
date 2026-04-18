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
wayclick.trigger {
    id = "timed_macro",
    mode = "oneshot",
    action = wayclick.sequence {
        actions = {
            wayclick.auto_click { button = "left" },
            wayclick.delay { ms = 500 },
            wayclick.auto_click { button = "left" },
        },
    },
}
```

## Trigger Modes

| Mode      | Behavior                                           |
|-----------|----------------------------------------------------|
| `toggle`  | First press starts, second press stops             |
| `hold`    | Active while button is held, stops on release      |
| `oneshot` | Executes once per press                            |

## Device Bindings

```lua
wayclick.device {
    name_contains = "G Pro",         -- Match by name substring
    -- vid = 0x046d, pid = 0xc08b,   -- Match by vendor/product ID
    -- phys_contains = "usb-...",    -- Match by physical location
    -- path = "/dev/input/event5",   -- Match by device path
    exclusive = false,               -- EVIOCGRAB exclusive access
    bindings = {
        { code = "BTN_SIDE",  trigger = "rapid_fire" },
        { code = "BTN_EXTRA", trigger = "burst_fire" },
    }
}
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

See the full list with `wayclick-evdev-dump monitor`.
