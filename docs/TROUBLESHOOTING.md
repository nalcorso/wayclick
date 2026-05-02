# Troubleshooting Guide

This guide covers common wayclick issues and solutions. For quick diagnosis, start with
checking permissions and configuration, then consult the relevant section below.

---

## Common Issues

### Daemon Startup Failures

**Symptom:** `wayclickd` exits immediately or won't start.

**Check permissions first:**

```sh
wayclickd --check-permissions
```

**Common causes and solutions:**

- **`/dev/uinput` not accessible:** Run `./scripts/install.sh` and log out/in
- **`/dev/input/event*` not readable:** Add yourself to the `input` group:
  ```sh
  sudo usermod -aG input "$USER"
  # Log out and back in
  ```
- **Config file missing:** Create `~/.config/wayclick/init.lua`
- **XDG_RUNTIME_DIR not set:** Usually set automatically; if missing, systemd handles it

**Fallback to dry-run mode:**

If `/dev/uinput` is unavailable, the daemon automatically logs actions instead of
emitting real input. Force it with:

```sh
wayclickd --dry-run
wayclickctl logs   # view the logged actions
```

---

### Config Parsing Errors

**Symptom:** `wayclickctl status` shows 0 triggers, or `wayclickctl logs` shows Lua errors.

**Check the config:**

```sh
wayclickctl logs --tail 20   # view recent error messages
```

**Common Lua errors:**

- **Syntax error:** Check for missing commas, parentheses, or mismatched quotes in your
  `init.lua`. Wayclick sandboxes Lua, so standard Lua tooling may not catch all issues.
- **Invalid action:** Ensure action names match the API (`wayclick.click`, not
  `wayclick.click_button`). See [CONFIG_SCHEMA.md](CONFIG_SCHEMA.md) for the full reference.
- **Undefined trigger:** Verify all `trigger = "id"` references match registered trigger IDs.

**Test the config:**

```sh
wayclickctl reload
wayclickctl status   # should show your triggers
```

---

### Devices Not Detected

**Symptom:** Device bindings don't fire, or device doesn't appear in logs.

**List available devices:**

```sh
wayclick-evdev-dump list
```

This shows all accessible `/dev/input/event*` files.

**Identify your device:**

```sh
wayclick-evdev-dump identify
# Press a button on your device
```

The tool prints the device name, VID:PID, and physical path — use these in your config.

**Common causes:**

- **Device path changes on reboot:** Don't use `path = "/dev/input/event5"` — use `name` or
  `vid`/`pid` instead. See [DEVICE_MATCHING.md](DEVICE_MATCHING.md#by-device-path).
- **Permission denied:** The device file exists but isn't readable. Check udev rules:
  ```sh
  ls -la /dev/input/event*   # check readable status
  ```
- **Hotplug not working:** EvdevMonitor rescans every 2 seconds. If a device connects and
  isn't detected, try:
  ```sh
  wayclickctl reload
  ```

**Check device matching:**

Compare your config binding against the actual device name (case-insensitive substring):

```lua
-- If your device is "Logitech G Pro Gaming Mouse"
wayclick.bind_device({ name = "G Pro", bindings = { ... } })  -- ✓ works
wayclick.bind_device({ name = "g pro", bindings = { ... } })  -- ✓ case-insensitive
wayclick.bind_device({ name = "G Pro Gaming Mouse", bindings = { ... } })  -- ✓ exact match
wayclick.bind_device({ name = "Logitech", bindings = { ... } })  -- ✓ substring
wayclick.bind_device({ name = "XYZ", bindings = { ... } })  -- ✗ no match
```

---

### Triggers Not Firing

**Symptom:** Device is detected, but pressing buttons doesn't fire the trigger.

**Test the trigger manually:**

```sh
wayclickctl trigger <id>   # manually fire the trigger
```

If the trigger fires and performs the action (click, keystroke, etc.), the issue is with
device matching or button binding.

**Check device exclusivity conflicts:**

If you bind the same device in multiple `wayclick.bind_device()` calls, the first match
wins. Remove duplicate bindings.

**Check button codes:**

Use the device monitor to see which codes your buttons emit:

```sh
wayclick-evdev-dump monitor --device /dev/input/event5
# Press buttons on the device to see event codes
```

Compare the codes in your config to the output.

**Check layer filters:**

If the trigger has `layer = "combat"`, it only fires when the active layer is "combat":

```sh
wayclickctl layer get        # show active layer
wayclickctl layer set base   # switch to base layer
```

---

### IPC Connection Failures

**Symptom:** `wayclickctl` commands fail with "connection refused" or socket errors.

**Check if the daemon is running:**

```sh
ps aux | grep wayclickd
systemctl --user status wayclickd   # if running as a service
```

**Check the socket:**

```sh
ls -la "$XDG_RUNTIME_DIR/wayclick.sock"
```

**Common causes:**

- **Daemon crashed:** Check logs:
  ```sh
  journalctl --user -u wayclickd -n 50
  wayclickctl logs
  ```
- **Socket path differs:** By default, the socket is `$XDG_RUNTIME_DIR/wayclick.sock`. If
  `XDG_RUNTIME_DIR` is not set, the daemon may fail to start. Set it explicitly:
  ```sh
  export XDG_RUNTIME_DIR="/run/user/$(id -u)"
  wayclickd --enable
  ```
- **Permissions on socket:** The socket has `0600` permissions (readable only by your user).
  If another user tries to connect, access is denied.

**Restart the daemon:**

```sh
wayclickctl reload           # soft reload (re-reads config)
systemctl --user restart wayclickd   # hard restart
wayclickd --enable           # start manually
```

---

### Performance Problems

**Symptom:** High CPU usage, jerky clicking, or delays in trigger firing.

**Check interval timing:**

If you're using `auto_click` with a very low `interval_ms`, CPU usage increases:

```lua
-- ✗ 100% CPU usage
wayclick.auto_click({ button = "left", interval_ms = 1 })

-- ✓ reasonable (20 clicks/second)
wayclick.auto_click({ button = "left", interval_ms = 50 })
```

The minimum practical interval is ~10ms. Below that, OS scheduling and system load affect
actual click timing.

**Check jitter settings:**

Jitter adds random variation (good for avoiding detection, but adds overhead):

```lua
-- Turn off jitter if you don't need anti-pattern detection
wayclick.auto_click({ button = "left", interval_ms = 50, jitter_ms = 0 })
```

**Monitor active triggers:**

```sh
wayclickctl list    # shows which triggers are active/idle
```

A stuck `active` trigger consumes CPU. Toggle it off or reload the config.

**Check log volume:**

Verbose logging can slow down the daemon. Reduce with:

```lua
wayclick.set_global { log_level = "warn" }  -- reduce verbosity
```

---

### Exclusive Mode Failures

**Symptom:** `EVIOCGRAB` error in logs, or the raw device events still reach the OS.

**What EVIOCGRAB does:** With `exclusive = true`, wayclick claims exclusive access to the
device. The kernel blocks all other applications from receiving raw events. This is
required for scroll wheel remapping to prevent double input.

**Common causes:**

- **Another application holding exclusive access:** If a game or input mapper (like Xpadder
  or QJoyPad) holds the device, wayclick cannot grab it:
  ```sh
  wayclickctl logs | grep EVIOCGRAB
  ```
  Kill the conflicting application and reload wayclick's config.

- **Device doesn't support exclusive mode:** Some virtual devices (e.g., uinput-created mice)
  may not support `EVIOCGRAB`. Disable exclusive mode for those devices (but then scroll
  remapping won't work — raw scroll events will reach the OS).

- **Permission denied:** Ensure you have read+write access to the device:
  ```sh
  ls -la /dev/input/event5   # check permissions
  ```

**Verify exclusive mode is working:**

```sh
# Terminal 1: start the daemon
wayclickd --enable

# Terminal 2: open a text editor and scroll
# With exclusive mode: only wayclick's clicks appear (no native scroll)
# Without exclusive mode: both native scroll and wayclick's clicks appear
```

---

## Debugging Commands

```sh
# List all triggers and their state
wayclickctl list

# Show daemon status and active layer
wayclickctl status

# View recent log entries
wayclickctl logs --tail 50

# Reload config without restarting daemon
wayclickctl reload

# Manually fire a trigger by ID
wayclickctl trigger my_trigger_id

# Check permissions
wayclickd --check-permissions

# List all input devices
wayclick-evdev-dump list

# Identify a device (press a button to see its properties)
wayclick-evdev-dump identify

# Monitor events on a specific device
wayclick-evdev-dump monitor --device /dev/input/event5

# Run in dry-run mode (logs actions, doesn't emit real input)
wayclickd --dry-run
```

---

## Log Locations

- **Console output:** `wayclickctl logs [--tail N]` — recent log entries
- **systemd journal:** `journalctl --user -u wayclickd -f` — live daemon logs (if running as service)
- **Dry-run logs:** Run `wayclickd --dry-run` to see a verbose log of all actions

---

## Getting Help

- **Full API reference:** [CONFIG_SCHEMA.md](CONFIG_SCHEMA.md)
- **Device matching details:** [DEVICE_MATCHING.md](DEVICE_MATCHING.md)
- **Permissions and setup:** [PERMISSIONS.md](PERMISSIONS.md)
- **Build from source:** [BUILDING.md](BUILDING.md)
- **Security details:** [SECURITY.md](SECURITY.md)
- **IPC and programmatic control:** [IPC.md](IPC.md)
