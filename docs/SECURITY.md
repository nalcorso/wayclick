#!/usr/bin/env bash
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
| Lua config execution     | Sandboxed VM: `os.execute`, `io.popen`, `load`, `loadfile`, `dofile`, `debug.*` removed. `io.open` blocks write mode. |
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

- `io.open` is replaced with a wrapper that **only allows read mode**
- `package.path` is restricted to the config's `lua/` subdirectory
- `require()` can only load modules from the config directory

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
