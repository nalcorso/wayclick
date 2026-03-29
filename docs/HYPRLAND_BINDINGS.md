# Hyprland Key Bindings

Integrate wayclick with [Hyprland](https://hyprland.org/) by binding keys to
`wayclickctl` commands.

## Example `hyprland.conf` Bindings

```conf
# Toggle wayclick enabled/disabled
bind = SUPER, F9, exec, wayclickctl toggle

# Enable/Disable
bind = SUPER, F10, exec, wayclickctl enable
bind = SUPER, F11, exec, wayclickctl disable

# Fire specific triggers
bind = SUPER, F1, exec, wayclickctl trigger rapid_fire
bind = SUPER, F2, exec, wayclickctl trigger key_matrix

# Reload config
bind = SUPER, F12, exec, wayclickctl reload

# Open TUI
bind = SUPER, F8, exec, alacritty -e wayclick-tui
```

## With Notifications

Pair with `notify-send` for visual feedback:

```conf
bind = SUPER, F9, exec, wayclickctl toggle && notify-send "wayclick" "Toggled"
bind = SUPER, F10, exec, wayclickctl enable && notify-send "wayclick" "Enabled"
bind = SUPER, F11, exec, wayclickctl disable && notify-send "wayclick" "Disabled"
```

## Using Submap for Dedicated Mode

```conf
# Enter wayclick submap
bind = SUPER, W, submap, wayclick

# Wayclick submap bindings
submap = wayclick
bind = , T, exec, wayclickctl toggle
bind = , E, exec, wayclickctl enable
bind = , D, exec, wayclickctl disable
bind = , R, exec, wayclickctl reload
bind = , 1, exec, wayclickctl trigger rapid_fire
bind = , 2, exec, wayclickctl trigger key_matrix
bind = , ESCAPE, submap, reset
submap = reset
```

## Starting with Hyprland

Add to `hyprland.conf`:

```conf
exec-once = wayclickd --enable
```

Or use the systemd service:

```sh
systemctl --user enable --now wayclickd
```
