# Waybar Module for Wayclick

A real-time status module for [Waybar](https://github.com/Alexays/Waybar) that
displays the current state of the wayclick daemon — enabled/disabled, active
layer, running triggers, per-trigger activity, and more.

## Preview

```
 󰟸 Gaming·                      ← normal format, active layer marked
 󰟸 Gaming ⚡2                   ← verbose format (2 active triggers)
 󰟸 ●●○○                         ← triggers format (activity dots)
 󰟸                               ← minimal format
```

Tooltip (hover to see):

```
wayclick ── enabled ── Gaming
─────────────────────────────────────
 Triggers  8 total · 2 active

  ● rapid_fire          toggle   auto_click          47×
  ● auto_mode           toggle   key_press           12×
  ○ hideout_macro       oneshot  sequence             3×
  ✗ utility_1           toggle   keystroke            0×

─────────────────────────────────────
 Layers  base  gaming·  media  work

─────────────────────────────────────
 uinput  ·  uptime 2h 15m
```

(`·` marks the current layer, `●` active trigger, `○` idle, `✗` user-disabled)

## Setup

### 1. Add the module to your Waybar config

Edit `~/.config/waybar/config.jsonc`:

```jsonc
{
    "modules-right": ["custom/wayclick", "clock"],

    "custom/wayclick": {
        "exec": "wayclickctl waybar --continuous",
        "return-type": "json",
        "on-click":        "wayclickctl toggle",
        "on-click-right":  "wayclickctl layer set base",
        "on-click-middle": "wayclickctl reload",
        "on-scroll-up":    "wayclickctl layer cycle",
        "on-scroll-down":  "wayclickctl layer cycle --backward",
        "format": "{}",
        "tooltip": true,
        "escape": true
    }
}
```

See [`config.jsonc`](config.jsonc) for the full annotated example.

### 2. Add styles to your Waybar CSS

Copy the styles from [`style.css`](style.css) into `~/.config/waybar/style.css`.
Always include the **Base styles** block, then uncomment one theme:

| Theme | Description |
|-------|-------------|
| **Default** | Clean text colors, suits most setups |
| **Catppuccin Mocha** | Background + border, matches Catppuccin Mocha |
| **Pill** | Rounded pill with background fills and border pulse |
| **Gaming** | Per-color glow cycle, intense trigger flash |
| **Nord** | Cool blues and frost tones from the Nord palette |

### 3. Reload Waybar

```bash
killall -SIGUSR2 waybar
```

## Display Formats

```bash
wayclickctl waybar --format minimal    # 󰟸
wayclickctl waybar --format normal     # 󰟸 gaming        (default)
wayclickctl waybar --format verbose    # 󰟸 gaming ⚡2
wayclickctl waybar --format triggers   # 󰟸 ●●○○
```

The `triggers` format shows up to 8 per-trigger activity dots:
- `●` trigger is currently active (firing)
- `○` trigger is idle
- `✗` trigger has been user-disabled

## Event-driven vs Polling

| Mode | How it works | Update latency |
|------|-------------|---------------|
| **Event-driven** (default) | Single long-running process; daemon pushes events over IPC | <50 ms |
| **Polling fallback** | `"exec": "wayclickctl waybar"` + `"interval": 2` | ~2 s |

Event-driven mode is recommended. It uses far less CPU (no repeated spawning)
and reflects trigger activations in near real-time.

## Trigger Flash

When a trigger fires, `wayclickctl` briefly adds the `triggering` CSS class
(default 400 ms) so you can animate the indicator:

```bash
wayclickctl waybar --continuous --flash-ms 600
```

All themes in `style.css` include a `triggering` keyframe animation.
Each triggering trigger also adds a `trigger-{id}` class so you can style
individual triggers differently:

```css
#custom-wayclick.trigger-rapid-fire { color: #ff5555; }
```

## Layer Cycling

Scroll the waybar widget to cycle through available layers in order.
`wayclickctl` fetches the layer list from the daemon and wraps around:

```bash
wayclickctl layer cycle           # next layer
wayclickctl layer cycle --backward  # previous layer
wayclickctl layer list            # print all layers (current marked with *)
```

## Watching Live Events

Stream all IPC events to stdout (useful for debugging or scripting):

```bash
wayclickctl watch          # human-readable
wayclickctl watch --json   # raw JSON frames
```

## Per-Trigger Enable/Disable

Triggers can be toggled individually without touching the config:

```bash
wayclickctl trigger enable  rapid_fire
wayclickctl trigger disable rapid_fire
```

The disabled state is reflected in the `list_triggers` IPC response and shown
as `✗` in the tooltip.

## CSS Classes

| Class | Meaning |
|-------|---------|
| `enabled` | Daemon is enabled |
| `disabled` | Daemon is disabled |
| `active` | One or more triggers are currently firing |
| `idle` | No triggers active |
| `triggering` | A trigger just fired (held for `--flash-ms`) |
| `disconnected` | Daemon is not running or unreachable |
| `dry-run` | Daemon is in dry-run mode |
| `layer-{name}` | Current layer (e.g. `layer-base`, `layer-gaming`) |
| `trigger-{id}` | Added when that trigger fires (held for `--flash-ms`) |

## Click and Scroll Actions

| Action | Suggested Command |
|--------|------------------|
| Left click | `wayclickctl toggle` — toggle enabled/disabled |
| Right click | `wayclickctl layer set base` — reset to base layer |
| Middle click | `wayclickctl reload` — reload config without restart |
| Scroll up | `wayclickctl layer cycle` — next layer |
| Scroll down | `wayclickctl layer cycle --backward` — previous layer |

## Requirements

- [Waybar](https://github.com/Alexays/Waybar)
- `wayclickctl` in your `$PATH`
- A [Nerd Font](https://www.nerdfonts.com/) for the icon (e.g. JetBrainsMono Nerd Font)

