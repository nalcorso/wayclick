# Desktop Environment Integration

This guide shows how to integrate wayclick with various desktop environments and compositors. Each environment has different mechanisms for handling custom keybindings and IPC communication, so wayclick integration varies by platform.

## Hyprland

Hyprland is a modern tiling compositor with native keybinding support and extensive dispatcher system integration. These examples demonstrate wayclick integration with Hyprland's powerful configuration and control mechanisms.

### Example `hyprland.conf` Bindings

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

### With Notifications

Pair with `notify-send` for visual feedback:

```conf
bind = SUPER, F9, exec, wayclickctl toggle && notify-send "wayclick" "Toggled"
bind = SUPER, F10, exec, wayclickctl enable && notify-send "wayclick" "Enabled"
bind = SUPER, F11, exec, wayclickctl disable && notify-send "wayclick" "Disabled"
```

### Using Submap for Dedicated Mode

Submaps in Hyprland allow you to create a dedicated mode where specific keys perform wayclick actions. This is useful for keeping your main bindings clean:

```conf
# Enter wayclick submap with SUPER+W
bind = SUPER, W, submap, wayclick

# Wayclick submap bindings (no modifier needed)
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

### Starting with Hyprland

Add to your `hyprland.conf` to start wayclickd automatically:

```conf
exec-once = wayclickd --enable
```

Or use the systemd service for more control:

```sh
systemctl --user enable --now wayclickd
```

### Hyprland-Specific Socket Configuration

By default, wayclickd uses the standard socket path. For Hyprland, you can optionally configure:

```sh
export WAYCLICKD_SOCKET="${XDG_RUNTIME_DIR}/wayclick.sock"
wayclickd --enable
```

---

## Sway

Sway is an X11/Wayland tiling window manager and the spiritual successor to i3, designed for Wayland environments. It provides similar configuration to i3 but with better Wayland integration.

### Basic Setup

Install wayclick and configure it with a `~/.config/wayclick/config.yaml` file. See [CONFIG_SCHEMA.md](CONFIG_SCHEMA.md) for full configuration options.

### Example `~/.config/sway/config` Bindings

```conf
# Toggle wayclick enabled/disabled
bindsym $mod+F9 exec wayclickctl toggle
bindsym $mod+F10 exec wayclickctl enable
bindsym $mod+F11 exec wayclickctl disable

# Fire specific triggers
bindsym $mod+F1 exec wayclickctl trigger rapid_fire
bindsym $mod+F2 exec wayclickctl trigger key_matrix

# Reload wayclick config
bindsym $mod+F12 exec wayclickctl reload

# Open TUI
bindsym $mod+F8 exec alacritty -e wayclick-tui
```

### With Notifications

Pair with `notify-send` for feedback:

```conf
bindsym $mod+F9 exec wayclickctl toggle && notify-send "wayclick" "Toggled"
bindsym $mod+F10 exec wayclickctl enable && notify-send "wayclick" "Enabled"
bindsym $mod+F11 exec wayclickctl disable && notify-send "wayclick" "Disabled"
```

### Example: Game-Specific Layer

Use Sway bindings to toggle game mode:

```conf
# Quick game mode with wayclick enabled
bindsym $mod+g exec wayclickctl trigger game_mode && notify-send "Game Mode" "Activated"
```

Then in your `wayclick.yaml`:

```yaml
triggers:
  - name: game_mode
    actions:
      - action: enable
```

### Starting with Sway

Add to your `~/.config/sway/config`:

```conf
exec wayclickd --enable
```

Or use systemd:

```sh
systemctl --user enable --now wayclickd
```

### Sway-Specific Considerations

- **IPC Socket**: Sway uses a standard Unix socket at `$XDG_RUNTIME_DIR/sway-ipc.*.sock`, but wayclick uses its own socket
- **Environment Variables**: Sway sets `SWAYSOCK` which you may need for other tools
- **Wayland Exclusive**: Unlike i3, Sway is Wayland-only and provides better mouse device support

---

## i3

i3 is the most popular X11 tiling window manager. wayclick works on i3 but with some limitations due to the X11 environment. For the best wayclick experience with modern features, consider migrating to Sway or Hyprland.

### Basic Setup

Install wayclick and create a configuration at `~/.config/wayclick/config.yaml`. See [CONFIG_SCHEMA.md](CONFIG_SCHEMA.md) for available options.

### Example `~/.config/i3/config` Bindings

```conf
# Toggle wayclick enabled/disabled
bindsym $mod+F9 exec wayclickctl toggle
bindsym $mod+F10 exec wayclickctl enable
bindsym $mod+F11 exec wayclickctl disable

# Fire specific triggers
bindsym $mod+F1 exec wayclickctl trigger rapid_fire
bindsym $mod+F2 exec wayclickctl trigger scroll_action

# Reload wayclick config
bindsym $mod+F12 exec wayclickctl reload
```

### Example: Scroll Wheel Action

Define a simple trigger for scroll wheel remapping:

```yaml
# In ~/.config/wayclick/config.yaml
triggers:
  - name: scroll_action
    device_match:
      name: "Logitech USB Optical Mouse"
    actions:
      - action: map
        from:
          button: scroll_up
        to:
          key: Prior  # Page Up
```

### i3-Specific Considerations

- **X11 Only**: i3 does not support Wayland. Mouse support in wayclick is more limited on X11
- **No Exclusive Mode**: Exclusive mode is not available on X11; wayclick operates in shared mode only
- **libinput vs evdev**: Check your X11 input configuration; modern systems use libinput
- **Performance**: X11 event handling may have more latency than Wayland environments

### Starting with i3

Add to your `~/.config/i3/config`:

```conf
exec wayclickd --enable
```

Or use systemd:

```sh
systemctl --user enable --now wayclickd
```

---

## KDE Plasma

KDE Plasma is a full-featured desktop environment built on Qt with an integrated system for managing shortcuts and custom keybindings.

### Setup Overview

KDE Plasma stores custom shortcuts in `~/.config/kglobalshortcutsrc`. Rather than directly editing this file, you should use the KDE System Settings GUI to add custom shortcuts that invoke wayclick commands.

### Setting Up Custom Shortcuts

1. Open **System Settings** → **Shortcuts** → **Custom Shortcuts**
2. Create a new group or edit an existing one
3. Add new shortcuts that run `wayclickctl` commands

Example shortcuts to configure:
- `wayclickctl toggle` - Toggle wayclick on/off
- `wayclickctl enable` - Enable wayclick
- `wayclickctl disable` - Disable wayclick
- `wayclickctl trigger <name>` - Trigger specific actions

### Configuration File Approach

If preferred, you can edit `~/.config/kglobalshortcutsrc` directly. Add entries like:

```ini
[wayclick]
toggle=F9
enable=F10
disable=F11
reload=F12
```

Then create a script at `~/.local/bin/wayclickctl` wrapper if needed.

### Starting wayclickd

Configure wayclickd to start automatically:

```sh
systemctl --user enable --now wayclickd
```

Or add `wayclickd --enable` to your KDE Plasma startup applications.

### KDE Plasma-Specific Considerations

- **Not a Compositor**: KDE Plasma is a full desktop environment that can use different compositors (KWin)
- **Socket Path**: wayclickd uses the standard socket; ensure proper permissions in `~/.config/wayclick/`
- **Integration**: KDE Plasma integrates well with Wayland and provides good device detection

---

## GNOME

GNOME is a modern desktop environment with a focus on usability and consistency. Custom shortcuts are managed through GNOME Settings or configuration files.

### Setup Overview

GNOME stores custom keyboard shortcuts in `~/.config/dconf/user` (binary) or accessible via the graphical Settings application. The recommended approach is using GNOME Settings.

### Setting Up Custom Shortcuts

1. Open **Settings** → **Keyboard** → **Custom Shortcuts**
2. Click **+** to add a new custom shortcut
3. Set the name to something like "Toggle wayclick"
4. Set the command to `wayclickctl toggle`
5. Assign a keyboard shortcut (e.g., `Super+F9`)
6. Repeat for other wayclick commands

Example shortcuts to create:
- Name: "Toggle wayclick", Command: `wayclickctl toggle`
- Name: "Enable wayclick", Command: `wayclickctl enable`
- Name: "Disable wayclick", Command: `wayclickctl disable`

### Configuration File Approach

You can also configure via dconf-editor or command line:

```sh
# Example using gsettings (if available)
gsettings set org.gnome.settings-daemon.plugins.media-keys custom-keybindings \
  "['/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/toggle-wayclick/']"

gsettings set org.gnome.settings-daemon.plugins.media-keys.custom-keybinding:toggle-wayclick \
  name "Toggle wayclick"
gsettings set org.gnome.settings-daemon.plugins.media-keys.custom-keybinding:toggle-wayclick \
  command "wayclickctl toggle"
gsettings set org.gnome.settings-daemon.plugins.media-keys.custom-keybinding:toggle-wayclick \
  binding "<Super>F9"
```

### Starting wayclickd

Configure wayclickd to start automatically:

```sh
systemctl --user enable --now wayclickd
```

Or add `wayclickctl` to GNOME's startup applications via Settings.

### GNOME-Specific Considerations

- **Desktop Environment**: GNOME is a full desktop environment (typically uses Mutter as compositor)
- **Limited Configuration**: GNOME prioritizes user experience over fine-grained control
- **Wayland Ready**: GNOME has excellent Wayland support and provides good device detection
- **Dconf Integration**: Custom settings are stored in the dconf database, not text files

---

## General Recommendations

### Socket Path Considerations

wayclick uses a Unix socket for IPC communication between `wayclickd` and `wayclickctl`. By default, it uses:

```
$XDG_RUNTIME_DIR/wayclick.sock
```

Ensure this directory exists and has appropriate permissions:

```sh
mkdir -p "$XDG_RUNTIME_DIR"
chmod 700 "$XDG_RUNTIME_DIR"
```

### Permissions Setup

Across all environments, ensure proper permissions:

1. **User-level Socket**: The socket is created per-user and readable only by that user
2. **Device Access**: wayclick needs access to input devices (typically handled automatically via udev rules)
3. **Configuration Directory**: `~/.config/wayclick/` should be readable by the user

For custom setup:

```sh
mkdir -p ~/.config/wayclick
chmod 700 ~/.config/wayclick
```

See [PERMISSIONS.md](PERMISSIONS.md) for detailed permission configuration.

### Dry-Run Mode for Testing

Before production use, test your configuration with dry-run mode to verify that device matching and trigger actions work correctly:

```sh
wayclickctl status
wayclickctl dry-run <trigger-name>
```

This allows you to verify wayclick responds to your device without actually executing mapped actions.

### Device Matching

Choose your input devices carefully to avoid conflicts. Use the device matching features to target specific devices:

```yaml
device_match:
  vendor_id: "0x046d"      # Logitech
  product_id: "0xc07e"     # Specific model
```

For detailed device matching options, see [DEVICE_MATCHING.md](DEVICE_MATCHING.md).

### Performance Considerations

- **Exclusive Mode**: On Wayland compositors (Hyprland, Sway, Mutter/GNOME), exclusive mode provides lower latency
- **X11 Limitations**: i3 on X11 may have higher latency due to event handling differences
- **Socket Communication**: The socket-based IPC is efficient; multiple `wayclickctl` invocations add minimal overhead
- **Configuration Reload**: Use `wayclickctl reload` rather than restarting `wayclickd` to apply changes without interruption

### Cross-Environment Configuration

For multi-environment setups, keep your `~/.config/wayclick/config.yaml` the same across environments. Device matching handles environment-specific devices. Use trigger names consistently so your keybindings can reference the same triggers:

```yaml
triggers:
  - name: rapid_fire
    device_match:
      name: "Gaming Mouse"
    actions:
      # trigger definition
```

Then from any environment's configuration:

```conf
# Hyprland
bind = SUPER, F1, exec, wayclickctl trigger rapid_fire

# Sway
bindsym $mod+F1 exec wayclickctl trigger rapid_fire

# i3
bindsym $mod+F1 exec wayclickctl trigger rapid_fire

# All environments
```

This approach keeps your wayclick configuration consistent regardless of which desktop environment you're using.

### Recommended Setup Flow

1. Install wayclick via your package manager or build from source
2. Configure `~/.config/wayclick/config.yaml` for your devices
3. Start `wayclickd --enable` or use systemd service
4. Test with `wayclickctl status` and trigger commands
5. Integrate keybindings into your environment's configuration
6. Verify with dry-run mode before enabling production features
7. Monitor performance and adjust as needed

For more information on configuration options, see [CONFIG_SCHEMA.md](CONFIG_SCHEMA.md). For troubleshooting, see [TROUBLESHOOTING.md](TROUBLESHOOTING.md).
