# Wayclick Quick Start

Three worked examples. Pick the one that matches what you want to do, follow
the steps, and you'll have a working config in a few minutes.

---

## Before you start

### 1. Install and set up permissions

```sh
cargo build --workspace --release
./scripts/install.sh   # creates groups, installs udev rules
# Log out and back in for group changes to take effect
```

### 2. Find your device

```sh
wayclick-evdev-dump identify
```

Press a button on your device. The tool prints the device name, VID:PID, and
physical path. Note the name — you'll use it in the config.

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

### 3. Create your config

```sh
mkdir -p ~/.config/wayclick
# Create ~/.config/wayclick/init.lua — see examples below
```

### 4. Start the daemon

```sh
wayclickd --enable
```

The daemon loads `~/.config/wayclick/init.lua`. To check it parsed correctly:

```sh
wayclickctl status     # shows trigger count and active layer
wayclickctl ping       # confirms the daemon is running
```

To reload after editing your config:

```sh
wayclickctl reload     # or: kill -HUP $(pidof wayclickd)
```

---

## Example 1 — Scroll wheel to left-click (ARPG)

**Problem:** You want to rapid-click in an ARPG (Path of Exile, Diablo, etc.)
by scrolling the mouse wheel instead of hammering the left button.

**How it works:** Wayclick grabs exclusive access to your mouse, intercepts
scroll events, fires a left-click for each scroll notch, and forwards all other
events (mouse movement, unmatched buttons) through transparently.

```lua
-- ~/.config/wayclick/init.lua

wayclick.register_trigger({
  id     = "left_click",
  mode   = "oneshot",
  action = wayclick.click({ button = "left" }),
})

wayclick.bind_device({
  name      = "G Pro",    -- replace with your mouse name substring
  exclusive = true,       -- required for scroll remapping
  bindings  = {
    { scroll = "up",   trigger = "left_click" },
    { scroll = "down", trigger = "left_click" },
  },
})
```

> **Why `exclusive = true`?** Without it, the scroll event reaches the game
> *and* fires a click — you'd get double input. Exclusive mode suppresses the
> raw event; wayclick's virtual device delivers only the click.

**Test it:**

```sh
wayclickctl status   # should show 1 trigger
```

Open a text editor and scroll — you should see left-click selections.
Use `wayclickctl toggle` to disable it while browsing.

See the full annotated example at [examples/scroll_remap.lua](../examples/scroll_remap.lua).

---

## Example 2 — Auto-clicker with toggle

**Problem:** You want a button on your mouse to toggle rapid left-clicking on
and off (useful for idle games, repetitive tasks, or burst-fire mechanics).

```lua
-- ~/.config/wayclick/init.lua

wayclick.register_trigger({
  id   = "auto_clicker",
  name = "Auto Clicker",
  mode = "toggle",        -- first press starts, second press stops
  action = wayclick.auto_click({
    button      = "left",
    interval_ms = 50,     -- click every 50ms (20 clicks/second)
    jitter_ms   = 5,      -- ±5ms random variation (optional, anti-pattern detection)
    hold_ms     = 5,      -- hold the button down 5ms per click (more realistic)
  }),
  cooldown_ms = 200,      -- minimum time between toggle presses
})

wayclick.bind_device({
  vid      = 0x046d,
  pid      = 0xc08b,      -- replace with your device VID:PID
  bindings = {
    { code = "BTN_EXTRA", trigger = "auto_clicker" },
  },
})
```

**Test it:**

```sh
wayclickctl list     # shows auto_clicker: idle
```

Press your side button — the trigger state changes to `active`. Press it again
to stop. Or use `wayclickctl toggle` to disable all automation at once.

**Adjusting speed:**

| `interval_ms` | Clicks/second |
|---|---|
| 10 | 100 |
| 50 | 20 |
| 100 | 10 |
| 1000 | 1 |

The minimum interval is 1ms. The maximum is 3,600,000ms (1 hour).

---

## Example 3 — Typing macro (Path of Exile hideout)

**Problem:** Pressing F5 should open the in-game chat, type `/hideout`, and
press Enter — all as a single keystroke.

**How it works:** `wayclick.sequence` chains actions one after another.
`wayclick.keystroke` sends a single key chord. `wayclick.type_text` types a
string character by character.

```lua
-- ~/.config/wayclick/init.lua

wayclick.register_trigger({
  id   = "hideout",
  mode = "oneshot",
  action = wayclick.sequence({
    actions = {
      wayclick.keystroke({ key = "enter" }),            -- open chat
      wayclick.delay({ ms = 50 }),                      -- wait for chat to open
      wayclick.type_text({ text = "/hideout" }),        -- type the command
      wayclick.delay({ ms = 30 }),
      wayclick.keystroke({ key = "enter" }),            -- send command
    },
  }),
})

wayclick.bind_device({
  name     = "keyboard",    -- matches any device with "keyboard" in the name
  bindings = {
    { code = "KEY_F5", trigger = "hideout" },
  },
})
```

> **Note:** `type_text` uses a US QWERTY mapping. If your system uses a
> different keyboard layout, the characters may differ. Use individual
> `keystroke` calls with explicit key names for layout-independent macros.

**Test it:**

```sh
wayclickctl list   # should show: hideout (oneshot, idle)
```

Open a text editor, press F5 — you should see `\n/hideout\n` appear.
In Path of Exile, pressing F5 will open chat and teleport you to your hideout.

**Variations:**

```lua
-- Faster typing (less delay between characters):
wayclick.type_text({ text = "/hideout", delay_ms = 15 })

-- Slower typing (more reliable in games with input lag):
wayclick.type_text({ text = "/hideout", delay_ms = 80 })

-- Other useful PoE macros:
wayclick.type_text({ text = "/remaining" })   -- show monster count
wayclick.type_text({ text = "/deaths" })      -- death counter
wayclick.type_text({ text = "/age" })         -- character age
```

---

## Next steps

- **Full Lua API reference** → [CONFIG_SCHEMA.md](CONFIG_SCHEMA.md)
- **Programmatic control and game plugins** → [IPC.md](IPC.md)
- **Device matching options** → [DEVICE_MATCHING.md](DEVICE_MATCHING.md)
- **Systemd service setup** → [PERMISSIONS.md](PERMISSIONS.md)
- **Waybar status module** → [../extras/waybar/README.md](../extras/waybar/README.md)
