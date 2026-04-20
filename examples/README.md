# Examples

Example Lua configurations for wayclick. Copy any example to
`~/.config/wayclick/init.lua` to use it, or `require` it from your own config.

## Available Examples

| Example | Description |
|---|---|
| [morse_clicker.lua](morse_clicker.lua) | Clicks mouse morse code for any word. Includes full A–Z + 0–9 morse dictionary with generic `morse_letter()` and `morse_word()` helpers. Demo spells "banana" for the Revolution Idle achievement. |
| [scroll_remap.lua](scroll_remap.lua) | Remaps mouse wheel up/down to left-click for ARPG gaming. Demonstrates exclusive device mode with scroll bindings and automatic event forwarding. |

## Usage

```sh
# Copy an example as your config
cp examples/morse_clicker.lua ~/.config/wayclick/init.lua

# Or start from the default config and add parts you need
cp config/init.lua ~/.config/wayclick/init.lua
```

See [docs/CONFIG_SCHEMA.md](../docs/CONFIG_SCHEMA.md) for the full Lua API
reference.
