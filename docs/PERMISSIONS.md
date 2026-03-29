# Permissions Guide

Wayclick requires access to two kernel subsystems:

| Resource              | Permission | Used For                                |
|-----------------------|------------|-----------------------------------------|
| `/dev/uinput`         | Write      | Emitting virtual mouse/keyboard events  |
| `/dev/input/event*`   | Read       | Reading physical device buttons          |

## Quick Setup

Run the install script:

```sh
./scripts/install.sh
```

Or do it manually:

### 1. Create the `wayclick` group

```sh
sudo groupadd -f wayclick
sudo usermod -aG wayclick "$USER"
```

### 2. Install udev rules

```sh
sudo cp udev/99-wayclick.rules /etc/udev/rules.d/
sudo udevadm control --reload && sudo udevadm trigger
```

### 3. Add yourself to the `input` group (for device reading)

```sh
sudo usermod -aG input "$USER"
```

### 4. Apply group changes

Either log out and back in, or run:

```sh
newgrp wayclick
```

## Verifying Permissions

```sh
wayclickd --check-permissions
```

This checks:

- `/dev/uinput` is writable
- `/dev/input/event*` devices are readable
- Config file exists
- XDG_RUNTIME_DIR is available for the IPC socket

## Dry-Run Mode

If `/dev/uinput` is not available, the daemon automatically falls back to
dry-run mode, logging all actions instead of emitting real input events.

You can also force dry-run mode:

```sh
wayclickd --dry-run
```

Or in your Lua config:

```lua
wayclick.set_global { dry_run = true }
```

## systemd Service

The included systemd user service runs as your user session:

```sh
# Install
cp systemd/wayclickd.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now wayclickd

# Check status
systemctl --user status wayclickd
journalctl --user -u wayclickd -f
```

## Security Notes

- The `wayclick` group grants access only to `/dev/uinput`.
- Reading input devices requires the `input` group, which is standard on most distros.
- The daemon drops all capabilities except those needed for uinput/evdev access.
- See [SECURITY.md](SECURITY.md) for the full threat model.
