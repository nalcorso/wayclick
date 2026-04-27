# Device Matching Guide

Wayclick binds physical input devices (mice, keyboards) to trigger actions. This
document explains how device matching works and how to identify your devices.

## Quick Start

1. Run the device identification tool:

```sh
wayclick-evdev-dump identify
```

2. Press a button on the device you want to bind. The tool will print:

```
=== DEVICE IDENTIFIED ===
  Path:    /dev/input/event5
  Name:    Logitech G Pro Gaming Mouse
  VID:PID: 046d:c08b
  Phys:    usb-0000:00:14.0-2/input0
  Button:  code=272

Lua device match examples:
  wayclick.bind_device({ name = "Logitech G Pro Gaming Mouse" })
  wayclick.bind_device({ vid = 0x046d, pid = 0xc08b })
```

## Listing All Devices

```sh
wayclick-evdev-dump list
```

Displays all accessible input devices with their paths, names, vendor/product
IDs, and physical locations.

## Match Types

### By Name (substring, case-insensitive)

```lua
wayclick.bind_device({
    name = "G Pro",
    bindings = { ... }
})
```

### By Vendor/Product ID

```lua
wayclick.bind_device({
    vid = 0x046d,
    pid = 0xc08b,
    bindings = { ... }
})
```

### By Physical Location

```lua
wayclick.bind_device({
    phys = "usb-0000:00:14.0",
    bindings = { ... }
})
```

### By Device Path

```lua
wayclick.bind_device({
    path = "/dev/input/event5",
    bindings = { ... }
})
```

> **Note:** Device paths can change across reboots. Prefer name or VID:PID matching.

### Multiple Matchers

When multiple match criteria are specified, wayclick matches if **any** of them
match:

```lua
wayclick.bind_device({
    name = "G Pro",
    vid = 0x046d, pid = 0xc08b,
    bindings = { ... }
})
```

## Exclusive Mode

When `exclusive = true`, wayclick grabs the device exclusively using
`EVIOCGRAB`. This prevents other applications from receiving raw events from the
device. Wayclick forwards non-consumed events (mouse movement, unmatched buttons,
unmatched scroll) through its virtual pointer device, so the mouse continues to
function normally — only matched events are intercepted.

Exclusive mode is **required** for scroll wheel remapping (to prevent both
the original scroll and the remapped click from reaching the application).

```lua
wayclick.bind_device({
    name = "G Pro",
    exclusive = true,
    bindings = { ... }
})
```

## Button Bindings

Each device binding maps physical button codes to trigger IDs:

```lua
wayclick.bind_device({
    name = "G Pro",
    bindings = {
        { code = "BTN_SIDE",  trigger = "rapid_fire" },
        { code = "BTN_EXTRA", trigger = "burst_fire" },
    }
})
```

### Common Button Codes

| Code          | Description           |
|---------------|-----------------------|
| `BTN_LEFT`    | Left mouse button     |
| `BTN_RIGHT`   | Right mouse button    |
| `BTN_MIDDLE`  | Middle mouse button   |
| `BTN_SIDE`    | Side button (back)    |
| `BTN_EXTRA`   | Extra button (forward)|

Use `wayclick-evdev-dump monitor --device /dev/input/eventN` to discover the
exact codes emitted by your device's buttons.

## Scroll Bindings

Scroll wheel events can be remapped to trigger actions. This is commonly used
in ARPGs to convert scroll wheel to rapid left-clicks. Scroll bindings require
`exclusive = true`.

```lua
wayclick.bind_device({
    name = "G Pro",
    exclusive = true,
    bindings = {
        { scroll = "up",   trigger = "left_click" },
        { scroll = "down", trigger = "left_click" },
    }
})
```

### Scroll Directions

| Direction | Description                |
|-----------|----------------------------|
| `up`      | Scroll wheel up            |
| `down`    | Scroll wheel down          |
| `left`    | Horizontal scroll left     |
| `right`   | Horizontal scroll right    |

Scroll magnitude is respected — fast scrolling (multiple notches per frame)
fires the trigger multiple times. Hi-res scroll events are automatically
suppressed when a standard scroll event matches, preventing double-triggering.

## Chord Bindings

Chords bind a trigger to multiple buttons pressed simultaneously. The buttons
must be listed in order, separated by `+`. Chord detection requires
`exclusive = true` so that the individual button events can be suppressed.

```lua
wayclick.bind_device({
    name      = "G Pro",
    exclusive = true,
    bindings  = {
        -- Fire only when BOTH side buttons are held simultaneously
        { code = "BTN_SIDE+BTN_EXTRA", trigger = "combo_action" },

        -- Individual bindings still work — they fire when only one is pressed
        { code = "BTN_SIDE",  trigger = "side_action" },
        { code = "BTN_EXTRA", trigger = "extra_action" },
    },
})
```

## Tap vs Long-Press

Use `hold_trigger` and `hold_ms` to bind different triggers to short press
(tap) and long press on the same button:

```lua
wayclick.bind_device({
    name     = "G Pro",
    bindings = {
        {
            code         = "BTN_SIDE",
            trigger      = "tap_action",   -- fires on quick press-and-release
            hold_trigger = "hold_action",  -- fires if held longer than hold_ms
            hold_ms      = 500,            -- 500ms threshold
        },
    },
})
```

When the button is released before `hold_ms`, `tap_action` fires. When it is
held for at least `hold_ms`, `hold_action` fires instead.

## Hotplug

The EvdevMonitor scans for new devices every 2 seconds. When a matched device
connects, monitoring starts automatically. When it disconnects, the reader
thread is cleaned up. No config reload is needed.

