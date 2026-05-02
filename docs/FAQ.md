# Frequently Asked Questions

## Does wayclick work with X11 and Wayland?

Yes, wayclick works with both display servers equally well. It interfaces directly with the Linux **evdev** input subsystem and the **uinput** virtual input device, which are universal across X11, Wayland, and any other display server.

The key requirement is that `wayclickd` runs with permission to access `/dev/input/event*` devices. This is a kernel-level interface and does not depend on any display-server-specific APIs.

**See:** [docs/PERMISSIONS.md](PERMISSIONS.md) for device access setup.

---

## Does wayclick work with online games?

Yes, wayclick works with online games. It emits real `evdev` input events that appear indistinguishable from hardware input to most games.

**Important:** Some anti-cheat systems (e.g., Valorant's Vanguard, Apex Legends) inspect the input stack at the kernel level and may flag or block automation. Always check your game's terms of service before using wayclick, especially in competitive multiplayer titles.

**How it works:** Wayclick uses Linux's native `uinput` driver to emit input events. These are real kernel-level events, not synthetic events from a GUI library. Most games detect automation through behavioral analysis (e.g., "clicks are too regular") rather than by detecting the input mechanism itself.

**Anti-detection tips:**
- Use jitter (random timing variance) in `auto_click` to randomize intervals
- Use randomized hold durations to vary click shapes
- Rotate through multiple device bindings to avoid pattern detection

---

## What's the CPU and memory impact?

Wayclick is extremely lightweight:

- **Memory:** ~10–15 MB resident (daemon + TUI)
- **CPU:** <0.1% idle; ~1–2% while actively emitting events (depends on interval)
- **Startup time:** <50ms to load config and bind devices

The daemon is event-driven; it only wakes up when:
1. An input event arrives from a bound device
2. An action is scheduled (e.g., a delay between trigger steps)
3. An IPC client connects

There is no polling loop or continuous background activity.

---

## Can I run multiple wayclick instances?

Yes, you can run multiple `wayclickd` instances, but they **must use different socket paths** to avoid conflicts.

**Setup:**

```bash
# Instance 1 (default socket)
wayclickd

# Instance 2 (custom socket path)
wayclickd --socket /tmp/wayclick-2.sock
```

**Via config:**

```lua
wayclick.set_options({
  socket_path = "/tmp/wayclick-custom.sock"
})
```

**When you need multiple instances:**
- Running different configs for different devices/layers
- Isolating configs by workspace or context
- Testing a new config without stopping the current one

**Control each instance separately:**

```bash
# Instance 1 (default)
wayclickctl status

# Instance 2 (custom socket)
wayclickctl --socket /tmp/wayclick-2.sock status
```

---

## How do I debug Lua config syntax errors?

Use the `wayclickd --check-config` command to validate your config before starting the daemon:

```bash
wayclickd --check-config ~/.config/wayclick/init.lua
```

**Output on success:**
```
Config loaded successfully. X triggers, Y devices, Z layers.
```

**Output on error:**
```
Lua error at line 42: unexpected symbol 'end'
```

**Common mistakes:**
1. **Missing closing brace:** Every `{` must have a matching `}`
2. **Typo in function name:** `wayclick.register_tigger` (should be `register_trigger`)
3. **Wrong table key:** `button = "left"` vs `Button = "left"` (Lua is case-sensitive)
4. **Incomplete action:** `action = wayclick.click()` ← parentheses required
5. **Quotes in strings:** `button = 'left'` and `button = "left"` are both valid, but `button = left` is not

**Debugging workflow:**

1. Check config syntax:
   ```bash
   wayclickd --check-config ~/.config/wayclick/init.lua
   ```

2. Start daemon in dry-run mode (no real events emitted):
   ```bash
   wayclickd --dry-run
   ```

3. View logs in real-time:
   ```bash
   wayclickctl logs --tail 50
   ```

4. Reload after edits:
   ```bash
   wayclickctl reload
   ```

---

## Why are my triggers not firing?

**Checklist:**

1. **Device not found?**
   - List available devices: `wayclickctl devices`
   - Verify device name matches exactly (case-sensitive, spaces matter)
   - Optionally match by VID:PID instead of name: `vid = 0x046d, pid = 0xc08b`

2. **Wrong binding code?**
   - Identify button/key codes: `wayclick-evdev-dump monitor` (then press the button)
   - Check if code uses `scroll` binding for scroll events (not `code`)

3. **Layer mismatch?**
   - Verify trigger is not on a layer you haven't activated
   - Check current layer: `wayclickctl layer get`
   - Switch layers: `wayclickctl layer set <name>`

4. **Trigger disabled?**
   - Check if automation is disabled globally: `wayclickctl status`
   - Check if trigger is disabled specifically: `wayclickctl trigger disable <id>`

5. **Cooldown active?**
   - Triggers with `cooldown_ms` cannot fire again until the cooldown expires
   - Press the button again after the cooldown time

6. **Permission issue?**
   - Verify user has access to `/dev/input/event*`: `ls -la /dev/input/`
   - See [docs/PERMISSIONS.md](PERMISSIONS.md) for permission setup (udev rules or group membership)

7. **Config not reloaded?**
   - After editing `init.lua`, reload: `wayclickctl reload`
   - Verify reload succeeded: `wayclickctl logs --tail 5`

**See:** [docs/TROUBLESHOOTING.md](TROUBLESHOOTING.md) for in-depth debugging.

---

## Do I need exclusive mode for all devices?

No. **Exclusive mode is only needed for certain features:**

- ✅ **Scroll remapping** — requires `exclusive = true`
- ✅ **Multi-button chords** (e.g., `"BTN_LEFT+BTN_RIGHT"`) — requires `exclusive = true`
- ❌ **Simple button binding** — does NOT require exclusive; OS gets a copy of unhandled events
- ❌ **Keyboard bindings** — does NOT require exclusive; other apps still see the keys

**Why exclusive?**

When `exclusive = true`, wayclick grabs the physical device from the kernel. All events from that device go to wayclick first; it decides which to suppress and which to forward. This is required for scroll remapping because:
- Without exclusive: scroll up/down events reach your application + wayclick
- With exclusive: wayclick intercepts scroll, converts it to a click, and forwards only the click

**Non-exclusive device:**
```lua
wayclick.bind_device({
  name = "My Keyboard",
  exclusive = false,  -- OS still sees key events
  bindings = {
    { code = "KEY_F1", trigger = "my_trigger" },
  },
})
```

**See:** [docs/DEVICE_MATCHING.md](DEVICE_MATCHING.md) for detailed device binding rules.

---

## How do I connect the playground to wayclickd?

The **wayclick playground** (TUI dashboard) connects to `wayclickd` via the IPC socket to display live trigger state and logs.

**Connection modes:**

1. **Default socket** (automatic):
   ```bash
   wayclickctl tui
   ```
   Connects to `$XDG_RUNTIME_DIR/wayclick.sock` (usually `/run/user/1000/wayclick.sock`)

2. **Custom socket path:**
   ```bash
   wayclickctl --socket /tmp/wayclick-custom.sock tui
   ```

3. **Dry-run mode** (log events without emitting real input):
   - Start daemon: `wayclickd --dry-run`
   - Open TUI: `wayclickctl tui`
   - Trigger buttons in `wayclickctl tui` → see log entries in real-time
   - Verify your config logic before going live

**TUI features:**
- Real-time trigger firing and state changes
- Live log with recent events
- Layer indicator and switching
- Trigger enable/disable toggles
- Manual trigger firing
- JSON-RPC command shell

**Socket configuration in init.lua:**
```lua
wayclick.set_options({
  socket_path = "/tmp/wayclick-custom.sock"  -- Must match wayclickd's socket
})
```

If the TUI can't connect, verify:
- Daemon is running: `pgrep wayclickd`
- Socket exists: `ls -la /run/user/$UID/wayclick.sock`
- Socket permissions: `stat /run/user/$UID/wayclick.sock`

---
