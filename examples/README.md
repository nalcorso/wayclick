# Examples

Example Lua configurations for wayclick. Copy any example to
`~/.config/wayclick/init.lua` to use it, or `require` it from your own config.

## 📚 Example Configs

### Difficulty Levels

- 🟢 **Beginner** - Basic config syntax, simple bindings, one trigger per device
- 🟡 **Intermediate** - Multiple devices, layers, conditions, mixed binding types
- 🔴 **Advanced** - Complex Lua logic, chords, multi-step actions, game-specific optimization

### Learning Path

#### Start here (Beginner):

**[scroll_remap.lua](scroll_remap.lua)** 🟢
- **What it does:** Remaps mouse scroll wheel to left-click for ARPG gaming (fast clicking without wearing out your mouse button)
- **Teaches:** 
  - Device matching by name
  - Exclusive device mode (required for scroll remapping)
  - Scroll bindings (`scroll = "up"` / `scroll = "down"`)
  - Simple oneshot actions
  - Event forwarding (unmatched events pass through to OS)
- **Use case:** Any clicking-intensive game (Diablo, Path of Exile, etc.)
- **Concepts:** Device bindings, exclusive mode, scroll events

#### Intermediate (Progressive):

**[morse_clicker.lua](morse_clicker.lua)** 🟡
- **What it does:** Clicks out Morse code for any word, spells "banana" to earn a Revolution Idle achievement
- **Teaches:**
  - Reusable helper functions (`morse_letter()`, `morse_word()`)
  - Composite actions (sequence of clicks with precise timing)
  - Delay actions for timing control
  - Lua table iteration for character mapping
  - Multi-device binding strategies
  - Dynamic trigger registration patterns
- **Use case:** Automating character actions in idle games, learning automation sequences
- **Concepts:** Sequences, delays, Lua logic, actions composition

#### Advanced (Extend Your Skills):

Consider these patterns from the default config and docs:

- **Multi-layer setups** — Switch between game-specific configs dynamically (see `docs/CONFIG_SCHEMA.md` Complete Example)
- **Button chords** — Bind multi-button combos like `"BTN_LEFT+BTN_RIGHT"` (requires exclusive mode)
- **Hold vs tap** — Bind different triggers to tap and long-press on the same button (hold_ms parameter)
- **Per-trigger cooldowns** — Rate-limit specific triggers while others run freely (cooldown_ms)
- **Jitter for anti-detection** — Randomize timing and hold duration to avoid pattern detection (jitter_ms)

### Quick Reference Table

| Example | Difficulty | Concepts | Use Case |
|---------|-----------|----------|----------|
| [scroll_remap.lua](scroll_remap.lua) | 🟢 Beginner | Device binding, exclusive mode, scroll events, simple actions | ARPG click farming |
| [morse_clicker.lua](morse_clicker.lua) | 🟡 Intermediate | Sequences, delays, Lua helpers, table iteration, dynamic logic | Idle games, macros |
| Default config (see below) | 🟢 Beginner | Multiple triggers, mixed actions, exclusive + non-exclusive devices | Basic gaming setup |

### Key Concepts Cross-Reference

Use this table to find which examples teach which concepts:

| Concept | Where to Learn | Documentation |
|---------|---|---|
| **Device matching** | scroll_remap.lua, morse_clicker.lua | [CONFIG_SCHEMA.md § Device Bindings](../docs/CONFIG_SCHEMA.md#device-bindings) |
| **Exclusive mode** | scroll_remap.lua | [CONFIG_SCHEMA.md § Binding Options](../docs/CONFIG_SCHEMA.md#binding-options) |
| **Scroll remapping** | scroll_remap.lua | [CONFIG_SCHEMA.md § scroll](../docs/CONFIG_SCHEMA.md#scroll) |
| **Layers** | (see docs for complete example) | [CONFIG_SCHEMA.md § Layers](../docs/CONFIG_SCHEMA.md#layers) |
| **Sequences** | morse_clicker.lua | [CONFIG_SCHEMA.md § composite (sequence)](../docs/CONFIG_SCHEMA.md#composite-sequence) |
| **Delays** | morse_clicker.lua | [CONFIG_SCHEMA.md § delay](../docs/CONFIG_SCHEMA.md#delay) |
| **Trigger modes** | All examples | [CONFIG_SCHEMA.md § Trigger modes](../docs/CONFIG_SCHEMA.md#trigger-modes) |
| **Button chords** | (see docs for complete example) | [CONFIG_SCHEMA.md § Complete Example Config](../docs/CONFIG_SCHEMA.md#complete-example-config) |
| **Cooldown & jitter** | (see docs for complete example) | [CONFIG_SCHEMA.md § auto_click](../docs/CONFIG_SCHEMA.md#auto_click) |

## Usage

```sh
# Copy an example as your config
cp examples/scroll_remap.lua ~/.config/wayclick/init.lua

# Or start from the default config and add parts you need
cp config/init.lua ~/.config/wayclick/init.lua

# Validate your config before starting the daemon
wayclickd --check-config ~/.config/wayclick/init.lua

# Start the daemon
wayclickd

# See live logs and trigger state
wayclickctl tui
```

## Building Your Own

1. **Start with an example** that matches your use case
2. **Customize device names** — run `wayclickctl devices` to list your devices
3. **Find button codes** — run `wayclick-evdev-dump monitor` and press the buttons you want to bind
4. **Test with dry-run** — `wayclickd --dry-run` to verify behavior without emitting real events
5. **Hot-reload** — Edit `~/.config/wayclick/init.lua` then run `wayclickctl reload`
6. **Debug** — Use `wayclickctl logs --tail 50` to see what's happening

## See Also

- [docs/CONFIG_SCHEMA.md](../docs/CONFIG_SCHEMA.md) — Full Lua API reference with all action types
- [docs/CONFIG_SCHEMA.md § Complete Example Config](../docs/CONFIG_SCHEMA.md#complete-example-config) — Multi-layer, multi-device working example
- [docs/TROUBLESHOOTING.md](../docs/TROUBLESHOOTING.md) — Debug common config issues
- [docs/DEVICE_MATCHING.md](../docs/DEVICE_MATCHING.md) — Device selection strategies
