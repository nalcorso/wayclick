# wayclick-input-viz

A standalone visual input testing tool for validating [wayclick](https://github.com/nalcorso/wayclick)
actions and behaviors. Opens a GPU-accelerated window that captures and visualizes all mouse and
keyboard input with particle effects, performance counters, and an event log.

![Neon cyberpunk aesthetic — dark background with cyan, magenta, and green particle effects]

## Features

- **Particle effects** — burst explosions on click, movement trail, scroll fountain,
  floating key labels
- **Click rate counter** — rolling 5-second window, per-button totals
- **Event log** — scrolling color-coded log of all input events
- **GLSL shaders** — animated background grid, bloom/glow post-processing
- **Standalone** — runs without wayclick; useful for testing any input setup
- **Self-contained** — font and shaders are embedded in the binary

## Usage

```sh
# From the wayclick repo root
cargo run -p wayclick-input-viz

# Or build release for smoother performance
cargo run -p wayclick-input-viz --release
```

## Testing Wayclick

1. Start `wayclickd` with your config
2. Launch `wayclick-input-viz` — it becomes your click target
3. Trigger your wayclick actions and observe:
   - **Click rate** matches your configured auto-click speed
   - **Scroll events** appear (or don't, if remapped) correctly
   - **Key sequences** show the expected keys
   - **No duplicate or dropped events** in the log

## Visual Design

Dark neon/cyberpunk theme:

| Input Type    | Color    |
|---------------|----------|
| Left click    | Cyan     |
| Right click   | Magenta  |
| Middle click  | Gold     |
| Side buttons  | Orange   |
| Scroll        | Green    |
| Keyboard      | Silver   |
| Mouse trail   | Purple   |

## Dependencies

- [macroquad](https://github.com/not-fl3/macroquad) — OpenGL 2D graphics
- [JetBrains Mono](https://www.jetbrains.com/mono/) — embedded font (OFL 1.1 license)

## License

MIT — same as the wayclick project.
