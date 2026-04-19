# Security Model

## Threat Model

Wayclick is a local-only automation daemon. It does **not**:

- Listen on network interfaces
- Accept remote connections
- Execute arbitrary commands from config (Lua sandbox prevents this)
- Store or transmit credentials

### Attack Surface

| Surface                  | Mitigation                                         |
|--------------------------|----------------------------------------------------|
| Lua config execution     | Sandboxed VM: `os.execute`, `io.popen`, `load`, `loadfile`, `dofile`, `debug.*` removed. `io.open` restricted to config directory, read-only. |
| IPC socket               | Unix domain socket with `0600` permissions. Only the owning user can connect. |
| `/dev/uinput` access     | Controlled via the `wayclick` group and udev rules. |
| `/dev/input` access      | Standard `input` group membership.                 |
| Config hot-reload        | Only reloads from the configured Lua path. Validates before applying. |
| IPC frame parsing        | Maximum frame size enforced (64KB). JSON-RPC validation. Fuzz-tested. |

### Lua Sandbox Details

The following functions are **removed** from the Lua global environment:

- `os.execute` — shell command execution
- `os.exit` — process termination
- `io.popen` — pipe to shell command
- `load` — load arbitrary Lua code from strings
- `loadfile` — load Lua code from arbitrary files
- `dofile` — execute Lua code from arbitrary files
- `debug.*` — entire debug table

Additionally:

- `io.open` is replaced with a wrapper that **only allows read mode** and **only permits access to files within the config directory** (`~/.config/wayclick/`). Paths are canonicalized to prevent symlink and `../` traversal attacks.
- `package.path` is restricted to the config's `lua/` subdirectory
- `package.cpath` is cleared — no native C modules can be loaded
- `require()` can only load modules from the config directory

### Data Exfiltration Analysis

Even if a Lua config could read sensitive files, there is **no exfiltration channel**:

| Channel              | Status   | Notes                                                    |
|----------------------|----------|----------------------------------------------------------|
| Network sockets      | Blocked  | Standard Lua 5.4 has no socket/HTTP library. `package.cpath = ""` prevents loading LuaSocket or any native C module. |
| Shell commands        | Blocked  | `os.execute` and `io.popen` are nil.                     |
| File writes           | Blocked  | `io.open` rejects write and append modes.                |
| Dynamic code loading  | Blocked  | `load`, `loadfile`, `dofile` are nil.                    |
| Encoding into actions | Not viable | Lua runs only at config-load time. Action constructors return static config tables — file contents cannot be dynamically injected into keystrokes or mouse actions at runtime. |

With `io.open` now restricted to the config directory, this is defense-in-depth: even if an exfiltration channel were discovered, the readable file set is limited to the user's own config files.

### Keylogging Considerations

Wayclick can bind keyboard events (`KEY_*` codes) on specific input devices. With debug logging enabled, each key press produces a log entry like:

```
Button KEY_A pressed, firing trigger 'my_trigger'
```

**Why this is not a vulnerability:**

- Only the user's own devices can be monitored (no privilege escalation)
- Only devices matching explicit `bind_device` rules are monitored — nothing is captured by default
- Logs contain key code names (`KEY_A`), not composed characters or passwords
- The log is an in-memory ring buffer (default 100 entries), accessible only via the user's own IPC socket
- Under systemd, log output goes to the user journal (readable only by the same user)
- There is no way to transmit captured data off-machine from Lua (see exfiltration analysis above)

This is a "user configures their own keylogger" scenario — equivalent to running `evtest` on your own input device. The same user who writes the config is the one whose input gets logged.

**Recommendation:** Avoid binding keyboard triggers with debug-level logging in shared environments. Default log level (`info`) does not include per-key-press messages.

### IPC Security

- The IPC socket is created with mode `0600` (user read/write only)
- Located in `$XDG_RUNTIME_DIR` (typically `/run/user/<uid>/`)
- JSON-RPC frames are limited to 64KB maximum
- Unknown methods return an error; they do not execute anything

## Recommendations

1. **Do not run as root.** Use the systemd user service.
2. **Audit your Lua config** before sharing it with others.
3. **Keep wayclick updated** to receive security fixes.
4. Use `wayclickd --check-permissions` to verify minimal access.

## Reporting Vulnerabilities

If you discover a security issue, please report it via GitHub Security
Advisories (preferred) or by email. Do not open a public issue.
