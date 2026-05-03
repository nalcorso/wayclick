# Changelog

All notable changes to wayclick are documented in this file.

## [Unreleased]

### Added

- New `wayclick-ipc-client` crate consolidates the IPC client logic that was
  previously duplicated between `wayclick-core` and `wayclick-playground`.
  Suitable for third-party tools that want to talk to the daemon without
  pulling in the full `wayclick-core` library. Exposes `frame::*` primitives,
  `socket::default_socket_path`, `SyncClient` for blocking RPC,
  `connect_with_timeout` for streaming sockets, and `AsyncClient` for
  background-thread event streaming with typed `IpcMessage` events.

### Changed

- `wayclick-tui` now depends on `wayclick-ipc-client` instead of
  `wayclick-core`.
- `wayclickctl` drops `wayclick-core` entirely and uses the new crate for
  one-shot RPC plus its hand-rolled waybar streaming subscribe loops.
- `wayclick-playground` uses `wayclick-ipc-client::AsyncClient`; the
  in-tree `src/ipc_client.rs` (556 lines) is removed.
- `wayclick-core` server-side IPC is unchanged in behavior; it now imports
  the frame primitives from `wayclick-ipc-client` (single source of truth).
- `IpcCommand` on the new `AsyncClient` is generic — only `Send(Value)` and
  `Shutdown`. The playground's previous domain-specific variants
  (`FireTrigger`, `EnableTrigger`, `DisableTrigger`, `RefreshTriggers`) are
  gone; equivalent calls are now `client.send("trigger", Some(json!(…)))`,
  `client.send("enable_trigger", Some(json!(…)))`, etc. External
  consumers forking the playground should adjust accordingly.

### Fixed

- `ServiceStatus.active_triggers` is now `Vec<String>` (the trigger IDs)
  rather than the previously-incorrect `usize`. The playground's hand-rolled
  parser silently defaulted this field to 0 because of the type mismatch;
  the new typed deserialization surfaces the actual list.

### Removed

- `wayclick_core::ipc::ipc_request` and `ipc_connect` — these client-side
  helpers moved to `wayclick_ipc_client::SyncClient` and
  `wayclick_ipc_client::connect_with_timeout`.
- The frame primitives `encode_frame` / `decode_frame` / `write_frame` /
  `IpcError` / `MAX_FRAME_SIZE` likewise moved to
  `wayclick_ipc_client::frame`.

---

## Repository History

The git history for this repository was collapsed before the initial public release to preserve privacy during early development. The v0.1.0 tag represents the first public version.

For development history, see the git log from v0.1.0 onwards.

---

## [0.1.0] - 2026-05-02

### Added

- **Kernel-level input automation** — Read button presses directly from evdev; fire actions through uinput. No dependency on display servers or desktop frameworks.
- **Lua-based configuration** — Flexible, sandboxed Lua config language with full API for triggers, bindings, actions, and layers. `~/.config/wayclick/init.lua`.
- **Cross-desktop support** — Works on Wayland, X11, and headless systems. Built to be desktop-environment agnostic.
- **Exclusive device mode** — Optional `EVIOCGRAB` for advanced bindings that suppress original events (e.g., scroll-to-click).
- **Comprehensive action types** — Auto-click, macros, keystrokes, text typing, mouse movement, scrolling, media keys, click-drag, and more.
- **Layer management** — Maintain separate binding sets and switch between them at runtime (base layer, combat layer, menu layer).
- **IPC control interface** — JSON-RPC 2.0 over Unix socket. Dynamic trigger registration, event subscriptions, and remote control from scripts, game plugins, or external tools.
- **Systemd service support** — Run as a user service with automatic startup, logging, and lifecycle management.
- **Multiple control interfaces**:
  - `wayclickctl` — CLI for daemon control (status, reload, trigger firing, layer switching, logs)
  - `wayclick-tui` — Real-time dashboard with trigger state, active layer, and log streaming
  - `wayclick-evdev-dump` — Diagnostic tool for device listing, event monitoring, and button identification
  - `wayclick-playground` — GPU-accelerated visual input testing and feedback application
- **Waybar integration** — Status module with real-time automation state and layer display. Four CSS themes included (Default, Catppuccin, Pill, Gaming).
- **Fuzz testing** — Config loading, IPC framing, and device matching are fuzz-tested for robustness.
- **Comprehensive documentation** — QUICKSTART guide, CONFIG_SCHEMA reference, IPC protocol, device matching, security model, and desktop environment setup guides.

### Fixed

- Proper handling of simultaneous multi-button events
- Correct layer persistence across daemon reloads
- IPC frame boundary handling for large payloads
- Device matching with Unicode device names

### Security

- **Sandboxed Lua** — `os.execute`, `io.popen`, `load`, `debug`, and native module `require` disabled. `io.open` restricted to config directory, read-only.
- **Local-only IPC** — Unix socket with `0600` permissions. No network exposure.
- **Minimal privilege** — Runs as your user; requires only `wayclick` and `input` groups.
- **No secrets in config** — Passwords and tokens never written to disk.

### Documentation

- [QUICKSTART.md](docs/QUICKSTART.md) — Worked examples for scroll-to-click, auto-clicker, and macros
- [CONFIG_SCHEMA.md](docs/CONFIG_SCHEMA.md) — Complete Lua API reference with all parameters and examples
- [IPC.md](docs/IPC.md) — Protocol reference, all methods, event types, Python/bash examples
- [DEVICE_MATCHING.md](docs/DEVICE_MATCHING.md) — Device identification and binding strategies
- [PERMISSIONS.md](docs/PERMISSIONS.md) — Group setup, udev rules, systemd service
- [SECURITY.md](docs/SECURITY.md) — Threat model and Lua sandbox design
- [ARCHITECTURE.md](docs/ARCHITECTURE.md) — Crate layout, data flow, threading model
- [DESKTOP_ENVIRONMENTS.md](docs/DESKTOP_ENVIRONMENTS.md) — Integration guides for Hyprland, Sway, i3, KDE, GNOME

---

## Notes on Versioning

Wayclick uses semantic versioning. Before reaching 1.0, **breaking changes may occur in minor versions** (0.x.y). Config schema, IPC protocol, or API may change without notice. Starting from 1.0, semantic versioning will be strictly observed.
