# wayclick-playground

A GPU-accelerated visual testing tool for [wayclick](https://github.com/nalcorso/wayclick).
Opens an interactive window that visualises all input events with particle effects and — when
the wayclick daemon is running — shows live trigger state and lets you fire triggers by clicking.

![Neon cyberpunk aesthetic — dark background with cyan, magenta, and green particle effects]

## Features

- **IPC integration** — connects to the running wayclick daemon automatically; shows
  real-time trigger activations, layer changes, and service status
- **Trigger testing** — click any trigger row to fire it via IPC
- **Full input coverage** — BTN4/BTN5, media keys (mute, volume, play/pause, next/prev),
  and all standard keyboard/mouse events via evdev
- **Particle effects** — burst explosions on click, movement trail, scroll fountain,
  floating key labels, gold burst on trigger activation
- **Event log** — scrolling colour-coded log of all input events including IPC events
- **Graceful offline** — works as a standalone input visualiser when wayclick is not running
- **GLSL shaders** — animated background grid, bloom/glow post-processing
- **Self-contained** — font and shaders are embedded in the binary

## Usage

```sh
# From the wayclick repo root
cargo run -p wayclick-playground

# Or build release for smoother performance
cargo run -p wayclick-playground --release
```

## IPC Connection

The playground automatically connects to the wayclick daemon at startup. The connection
status is shown in the top-right corner of the HUD bar and in the status bar:

- **● LIVE** — connected; input events come via IPC, trigger list is live
- **◌ SYNC** — connecting or reconnecting
- **○ OFFLINE** — daemon not running; falls back to macroquad input only

The socket path follows `$XDG_RUNTIME_DIR/wayclick.sock` (or
`/tmp/wayclick-<uid>.sock` if `XDG_RUNTIME_DIR` is unset).

## Right Panel

The right panel is split into three sections:

| Section         | Description                                      |
|-----------------|--------------------------------------------------|
| Service panel   | Connection status, current layer, enabled state  |
| Trigger list    | Clickable live trigger list (scroll with wheel)  |
| Event log       | Timestamped colour-coded event history           |

**Trigger indicators:**
- `●` — trigger is currently active (live from IPC)
- `○` — trigger is idle
- `◎` — trigger is user-disabled

Click any row to fire the trigger. The mode badge (`ONCE`, `HOLD`, `TOGGLE`, `SEQ`) is
shown on the right of each row.

## Input Coverage

When connected to the wayclick daemon, the event log captures:

| Input              | Source   |
|--------------------|----------|
| Left/Right/Middle  | macroquad |
| BTN_BACK / BTN_FORWARD / BTN_SIDE / BTN_EXTRA | IPC (evdev) |
| Media keys (mute, volume, play/pause, next, prev) | IPC (evdev) |
| All keyboard keys  | IPC (evdev) |
| Scroll wheel       | macroquad (always) |

When offline, mouse buttons and keyboard keys fall back to macroquad's input.

## Visual Design

Dark neon/cyberpunk theme:

| Input Type       | Color     |
|------------------|-----------|
| Left click       | Cyan      |
| Right click      | Magenta   |
| Middle click     | Gold      |
| Side buttons     | Orange    |
| Scroll           | Green     |
| Keyboard         | Silver    |
| Trigger fire     | Gold burst |
| Service events   | Green     |
| Mouse trail      | Purple    |

## Dependencies

- [macroquad](https://github.com/not-fl3/macroquad) — OpenGL 2D graphics
- [serde_json](https://github.com/serde-rs/json) — IPC JSON framing
- [JetBrains Mono](https://www.jetbrains.com/mono/) — embedded font (OFL 1.1 license)

## License

MIT — same as the wayclick project.
