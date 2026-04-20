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
`EVIOCGRAB`. This prevents other applications from receiving events from the
device.

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

## Hotplug

The EvdevMonitor scans for new devices every 2 seconds. When a device matching a
binding is connected, it is automatically added to monitoring. When a device
disconnects, its monitoring thread is cleaned up.
