# Waybar Module for Wayclick

A status module for [Waybar](https://github.com/Alexays/Waybar) that displays
the current state of the wayclick daemon — enabled/disabled, active layer,
running triggers, and more.

## Preview

```
┌─────────────────────────────────────────────┐
│  󰟸 Base           — enabled, base layer     │
│  󰟸 Gaming ⚡2     — gaming layer, 2 active  │
│  󰟸 Off            — disabled                │
│  󰟸 ✗              — daemon not running      │
└─────────────────────────────────────────────┘
```

## Setup

### 1. Add the module to your Waybar config

Edit `~/.config/waybar/config.jsonc`:

```jsonc
{
    "modules-right": ["custom/wayclick", "clock", ...],

    "custom/wayclick": {
        "exec": "wayclickctl waybar",
        "return-type": "json",
        "interval": 2,
        "on-click": "wayclickctl toggle",
        "on-click-right": "wayclickctl layer set base",
        "format": "{}",
        "tooltip": true
    }
}
```

See [`config.jsonc`](config.jsonc) for the full example with comments.

### 2. Add styles to your Waybar CSS

Copy the styles from [`style.css`](style.css) into `~/.config/waybar/style.css`.
Four themes are included — uncomment the one you prefer:

| Theme | Description |
|-------|-------------|
| **Default** | Clean text colors, suits most setups |
| **Catppuccin** | Background + border, matches Catppuccin Mocha |
| **Pill** | Rounded pill with background fills |
| **Gaming** | Glow effects, animated when triggers are active |

### 3. Reload Waybar

```bash
killall -SIGUSR2 waybar
```

## Display Formats

The `wayclickctl waybar` command supports three display formats:

```bash
wayclickctl waybar --format minimal    # 󰟸
wayclickctl waybar --format normal     # 󰟸 Base       (default)
wayclickctl waybar --format verbose    # 󰟸 Base ⚡2
```

## Continuous Mode

For lower latency updates, use continuous mode instead of Waybar's interval
polling. This keeps a single process running and outputs JSON lines:

```jsonc
{
    "custom/wayclick": {
        "exec": "wayclickctl waybar --continuous --interval 1",
        "return-type": "json",
        "tooltip": true,
        "format": "{}",
        "on-click": "wayclickctl toggle"
    }
}
```

## Tooltip

Hovering the module shows a rich tooltip:

```
wayclick: enabled
Layer: gaming
Triggers: 8 (2 active)
Active: auto_click, rapid_fire
Uptime: 2h 15m
```

## CSS Classes

The module outputs CSS classes you can target for styling:

| Class | Meaning |
|-------|---------|
| `enabled` | Daemon is enabled |
| `disabled` | Daemon is disabled |
| `active` | One or more triggers are currently firing |
| `idle` | No triggers active |
| `disconnected` | Daemon is not running |
| `layer-{name}` | Current layer (e.g., `layer-base`, `layer-gaming`) |
| `dry-run` | Daemon is in dry-run mode |

## Click Actions

| Click | Suggested Action |
|-------|-----------------|
| Left click | `wayclickctl toggle` — toggle enabled/disabled |
| Right click | `wayclickctl layer set base` — reset to base layer |
| Middle click | `wayclickctl reload` — reload config |
| Scroll up/down | Cycle layers (requires a custom script) |

## Requirements

- [Waybar](https://github.com/Alexays/Waybar)
- `wayclickctl` in your `$PATH`
- A [Nerd Font](https://www.nerdfonts.com/) for the icon (e.g., JetBrainsMono Nerd Font)
