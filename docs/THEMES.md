# TUI Themes

The wayclick TUI uses the [Catppuccin Mocha](https://github.com/catppuccin/catppuccin)
color palette as its default theme.

## Catppuccin Mocha Colors

| Role       | Color Name | Hex       | Usage                        |
|------------|------------|-----------|------------------------------|
| Base       | Base       | `#1e1e2e` | Background                   |
| Surface 0  | Surface0   | `#313244` | Header/footer background     |
| Surface 1  | Surface1   | `#45475a` | Borders, inactive elements   |
| Text       | Text       | `#cdd6f4` | Primary text                 |
| Subtext 0  | Subtext0   | `#a6adc8` | Secondary text, labels       |
| Blue       | Blue       | `#89b4fa` | Accents, active borders      |
| Green      | Green      | `#a6e3a1` | Enabled, active, connected   |
| Red        | Red        | `#f38ba8` | Disabled, errors, disconnected|
| Yellow     | Yellow     | `#f9e2af` | Warnings, dry-run indicator  |
| Mauve      | Mauve      | `#cba6f7` | Trigger modes                |
| Teal       | Teal       | `#94e2d5` | Action types                 |

## UI Elements

| Element              | Foreground | Background |
|----------------------|------------|------------|
| Header bar           | Blue/Green/Red | Surface0 |
| Active border        | Blue       | Base       |
| Inactive border      | Surface1   | Base       |
| Selected item        | Text (bold)| Surface1   |
| Active trigger (●)   | Green      | —          |
| Idle trigger (○)     | Surface1   | —          |
| Error messages       | Red        | Surface0   |
| Log level [ERROR]    | Red        | —          |
| Log level [WARN]     | Yellow     | —          |
| Log level [DEBUG]    | Subtext0   | —          |
| Log level [INFO]     | Text       | —          |
| Footer keybindings   | Blue (key) + Subtext0 (desc) | Surface0 |
