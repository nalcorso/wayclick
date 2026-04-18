# Feature Matrix: wayclick vs AutoHotkey vs X-Mouse Button Control

> Comparison current as of April 2026.
> **wayclick** (latest git), **AutoHotkey** v2.0.23, **X-Mouse Button Control** v2.20.5.

**Legend:** ✅ = supported, ⚠️ = partial / possible with workarounds, ❌ = not supported, N/A = not applicable.

> **Scope note:** These tools serve very different purposes. wayclick is intentionally narrow —
> a Linux-only mouse automation daemon. Many `❌` cells reflect deliberate scope boundaries,
> not missing features. AutoHotkey is a general-purpose Windows scripting language. XMBC is a
> focused GUI tool for mouse button remapping.

These three tools occupy very different niches:

| | wayclick | AutoHotkey (AHK) | X-Mouse Button Control (XMBC) |
|---|---|---|---|
| **In a nutshell** | Linux kernel-level mouse-automation daemon | Windows scripting language for desktop automation | Windows GUI tool for mouse button remapping |
| **Primary audience** | Linux power users, gamers, tinkerers | Windows power users, developers, office workers | Windows users who want mouse customisation |

---

## 1 · Input Injection Model

> This is arguably the most decision-relevant architectural difference between the three tools.

| Feature | wayclick | AHK | XMBC |
|---|---|---|---|
| **Input read method** | Linux evdev (`/dev/input/event*`) | Windows message hooks | Windows mouse message queue |
| **Input write method** | Linux uinput (`/dev/uinput`) | SendInput / SendEvent / SendPlay | Windows mouse message injection |
| **Compositor/WM dependency** | ❌ None (kernel-level) | ✅ Windows desktop session | ✅ Windows desktop session |
| **Works with Wayland** | ✅ Native | N/A | N/A |
| **Works with X11** | ✅ Works (no dependency) | N/A | N/A |
| **Per-device input isolation** | ✅ Reads individual `/dev/input` devices | ❌ All devices merged by OS | ❌ All devices merged by OS |
| **Button count limit** | None (any evdev code) | 5 mouse buttons + wheel | 5 buttons (Windows API limit) |
| **Elevated/admin window compat** | ✅ Kernel-level (no UAC issues) | ⚠️ Needs UIAccess or admin | ⚠️ May need elevation |
| **Anti-cheat compatibility** | ⚠️ Varies (kernel-level may help) | ⚠️ Varies (user-level hooks) | ⚠️ Varies (user-level hooks) |

---

## 2 · Platform & Environment

| Feature | wayclick | AHK | XMBC |
|---|---|---|---|
| **Linux** | ✅ Primary target (kernel 5.15+) | ❌ | ❌ |
| **Windows** | ❌ | ✅ Windows 7–11, Server | ✅ Windows 10–11, Server 2012 R2–2022 (older may work) |
| **macOS** | ❌ | ❌ | ❌ |
| **Wayland** | ✅ Native (kernel evdev/uinput) | N/A | N/A |
| **X11** | ✅ Works (no X11 dependency) | N/A | N/A |
| **Architecture** | x86_64 (Rust native binary) | x86/x64 (C++ binary) | x86/x64 (auto-detected) |
| **Display-server agnostic** | ✅ Kernel-level I/O | ❌ Windows desktop only | ❌ Windows desktop only |
| **Portable mode** | N/A (daemon install) | ✅ Single .exe, no install needed | ✅ Portable zip available |

## 3 · Installation & Lifecycle

| Feature | wayclick | AHK | XMBC |
|---|---|---|---|
| **Installation method** | cargo build / manual | Installer or standalone .exe | NSIS installer or portable zip |
| **Auto-start** | ✅ systemd user service | ✅ Startup folder / Task Scheduler | ✅ Auto-runs on login |
| **Service/daemon mode** | ✅ systemd managed (wayclickd) | ❌ Runs as tray app | ❌ Runs as tray app |
| **Hot-reload config** | ✅ File watcher + IPC reload | ⚠️ Reload script manually | ⚠️ Some changes apply live |
| **System tray** | ❌ (TUI dashboard instead) | ✅ Tray icon with menu | ✅ Tray icon with menu |
| **TUI dashboard** | ✅ ratatui-based (wayclick-tui) | ❌ | ❌ |
| **GUI** | ❌ | ✅ GUI framework + launcher UI | ✅ Full settings GUI |
| **Unattended/headless** | ✅ Designed for it | ⚠️ Possible but not primary | ❌ Requires desktop session |

## 4 · Configuration & Scripting

| Feature | wayclick | AHK | XMBC |
|---|---|---|---|
| **Configuration language** | Lua 5.4 | AHK scripting language | GUI + XML config files |
| **Turing-complete scripting** | ✅ Full Lua | ✅ Full AHK language | ❌ |
| **Variables & expressions** | ✅ Lua variables | ✅ Full expression engine | ❌ |
| **Loops & conditionals** | ✅ Lua control flow | ✅ If/else, loops, switch | ❌ |
| **Functions / subroutines** | ✅ Lua functions | ✅ Functions, classes, objects | ❌ |
| **Module / require system** | ✅ Lua `require()` | ✅ `#Include` directives | ❌ |
| **OOP** | ✅ Lua metatables | ✅ Classes & prototypes | ❌ |
| **Config validation** | ✅ At load time | ⚠️ Runtime errors | N/A (GUI prevents invalid) |
| **Sandbox / security** | ✅ Dangerous Lua APIs removed | ❌ Full system access | N/A |
| **Compile to standalone** | ❌ | ✅ .exe compilation | N/A |
| **DLL / FFI calls** | ❌ (sandboxed) | ✅ DllCall() | ❌ |
| **Pattern matching** | ⚠️ Lua patterns (not full regex) | ✅ PCRE regex | ❌ |
| **Config check tool** | ✅ `--check-config` flag | ❌ | N/A |

## 5 · Input Sources (Triggers)

| Feature | wayclick | AHK | XMBC |
|---|---|---|---|
| **Mouse buttons** | ✅ All (evdev codes) | ✅ Left/Right/Middle/X1/X2 | ✅ Up to 5 buttons |
| **Mouse wheel** | ✅ Up/Down/Left/Right | ✅ Up/Down/Left/Right | ✅ Scroll + tilt wheel |
| **Keyboard keys** | ❌ Mouse buttons only | ✅ Any key or combination | ❌ Mouse-focused |
| **Keyboard modifiers (Ctrl+key)** | ❌ Single-button triggers only | ✅ Full modifier combos (^!#+) | ❌ |
| **Key combinations (chords)** | ❌ | ✅ Custom combos (A & B) | ✅ Button chording |
| **Gamepad / joystick** | ⚠️ If evdev device | ✅ Native (Joy1–Joy32, axes) | ❌ |
| **Hotstrings (text triggers)** | ❌ | ✅ Auto-replace as-you-type | ❌ |
| **Window context triggers** | ❌ | ✅ #HotIf per-window | ✅ Per-app/window profiles |
| **Timer / scheduled triggers** | ❌ | ✅ SetTimer() | ❌ |
| **Device hotplug** | ✅ Auto-detect connect/disconnect | N/A (system-level) | ⚠️ Reconnection support |
| **Hold duration triggers** | ❌ | ✅ Key-up events, hold detection | ✅ Timed button actions |

## 6 · Trigger Execution Modes

| Feature | wayclick | AHK | XMBC |
|---|---|---|---|
| **Toggle** (press on / press off) | ✅ With cooldown/debounce | ⚠️ Manual via script variables | ⚠️ Sticky buttons |
| **Hold** (active while held) | ✅ Native mode | ✅ Key-down / key-up events | ✅ Hold-down actions |
| **OneShot** (fire once) | ✅ Synchronous execution | ✅ Default hotkey behaviour | ✅ Default button remap |
| **Cooldown / debounce** | ✅ Per-trigger ms | ⚠️ Manual via script | ❌ |
| **Duration limit** | ✅ Auto-stop after N ms | ⚠️ Manual via SetTimer | ❌ |

## 7 · Output Actions — Mouse

| Feature | wayclick | AHK | XMBC |
|---|---|---|---|
| **Auto-click (rapid fire)** | ✅ Configurable interval, jitter, hold_ms | ⚠️ Via Click in loop | ❌ (simulated keystrokes only) |
| **Click (press + release)** | ✅ Any button | ✅ Any button, any position | ✅ Via remapping |
| **Mouse press / release** | ✅ Separate down/up | ✅ Click "Down" / "Up" | ⚠️ Via simulated keystrokes |
| **Mouse move (relative)** | ✅ dx/dy with interval | ✅ Relative & absolute | ❌ |
| **Mouse move (absolute)** | ❌ | ✅ Move to (x, y) coords | ❌ |
| **Click at position** | ❌ | ✅ Click x, y | ❌ |
| **Drag** | ❌ | ✅ Click-drag | ✅ Click-drag (sticky) |
| **Scroll wheel output** | ✅ Any direction + amount | ✅ WheelUp/Down/Left/Right | ✅ Scroll remap |
| **Jitter / randomisation** | ✅ ±ms on interval | ❌ (manual Random()) | ❌ |
| **Hold duration per click** | ✅ hold_ms parameter | ⚠️ Manual press/sleep/release | ✅ {HOLDMS:n} |

## 8 · Output Actions — Keyboard

| Feature | wayclick | AHK | XMBC |
|---|---|---|---|
| **Key press** | ✅ By name or code | ✅ Send command (rich syntax) | ✅ Simulated keystrokes editor |
| **Key sequence** | ✅ Via Sequence composite | ✅ Send "multi-key string" | ✅ Key sequence in editor |
| **Key remapping** | ❌ | ✅ Native remap syntax | ❌ (mouse-to-key only) |
| **Modifier keys** | ✅ Individual key press | ✅ ^!+# prefixes | ✅ CTRL/ALT/SHIFT/WIN tags |
| **Type text string** | ⚠️ Via key sequence | ✅ SendText / hotstrings | ⚠️ Via simulated keystrokes |
| **Input levels** | ❌ | ✅ SendLevel / InputLevel | ❌ |

## 9 · Output Actions — System

| Feature | wayclick | AHK | XMBC |
|---|---|---|---|
| **Launch application** | ❌ (sandboxed) | ✅ Run command | ✅ Run any application |
| **Clipboard access** | ❌ (sandboxed) | ✅ Full clipboard control | ✅ Copy/Cut/Paste |
| **Window management** | ❌ | ✅ WinActivate, WinMove, WinMinimize, etc. | ✅ Basic (activate, snap) |
| **Media control** | ❌ (key codes only) | ✅ Media keys | ✅ Play/Pause/Stop/Volume |
| **Volume control** | ❌ (key codes only) | ✅ SoundSetVolume | ✅ {VOL+}/{VOL-}/{MUTE} |
| **Screen capture** | ❌ | ⚠️ Via library | ✅ Full screen / active window |
| **File I/O** | ⚠️ Lua read-only io.open | ✅ Full file read/write | ❌ |
| **Network / HTTP** | ❌ | ✅ Download, COM objects | ❌ |
| **Registry access** | N/A (Linux) | ✅ RegRead/RegWrite | ❌ |
| **Process management** | ❌ | ✅ ProcessExist, ProcessClose | ✅ {KILL:exe} |
| **GUI creation** | ❌ | ✅ Full GUI framework | ❌ |
| **Shell command execution** | ❌ (sandboxed) | ✅ Run / RunWait | ✅ Launch applications |

## 10 · Composite & Macro Actions

| Feature | wayclick | AHK | XMBC |
|---|---|---|---|
| **Sequential composition** | ✅ `wayclick.sequence{}` | ✅ Line-by-line execution | ✅ Key sequences |
| **Parallel composition** | ✅ `wayclick.parallel{}` | ⚠️ Via threads | ❌ |
| **Delay / sleep** | ✅ `wayclick.delay{ms=N}` | ✅ Sleep N | ✅ {WAIT:n} / {WAITMS:n} |
| **Nesting** | ✅ Arbitrary depth | ✅ Functions/subroutines | ❌ |
| **Macro recording** | ❌ | ⚠️ Community tools | ❌ |
| **Loop / repeat** | ✅ Toggle/Hold modes loop | ✅ Loop construct | ✅ Sticky repeat |
| **Abort / cancel** | ✅ Toggle off / release | ✅ Hotkey off, ExitApp | ✅ {FLUSH} command |
| **Random delay** | ✅ jitter_ms on intervals | ✅ Random() function | ✅ {WAITMS:x-y} random range |

## 11 · Device Matching & Binding

| Feature | wayclick | AHK | XMBC |
|---|---|---|---|
| **Per-device binding** | ✅ VID:PID, name, phys path | ❌ (system-wide input) | ⚠️ Limited device awareness |
| **Device name matching** | ✅ Substring, case-insensitive | ❌ | ❌ |
| **VID:PID matching** | ✅ Stable across reboots | ❌ | ❌ |
| **Physical path matching** | ✅ USB topology | ❌ | ❌ |
| **Multiple simultaneous devices** | ✅ Independent threads per device | ⚠️ All devices merged | ⚠️ Single mouse focus |
| **Exclusive device grab** | ✅ EVIOCGRAB | ❌ | ❌ |
| **Hotplug detection** | ✅ 2-second scan interval | N/A | ⚠️ Basic reconnect |
| **Device enumeration tool** | ✅ wayclick-evdev-dump | ❌ | ❌ |

## 12 · Per-Application Profiles

| Feature | wayclick | AHK | XMBC |
|---|---|---|---|
| **Per-app configuration** | ❌ (planned) | ✅ #HotIf WinActive() | ✅ Full per-app profiles |
| **Per-window configuration** | ❌ | ✅ Window title/class matching | ✅ Window class/title/region |
| **Profile layers** | ❌ | ⚠️ Via script logic | ✅ Up to 10 layers per profile |
| **Auto-switch on focus** | ❌ | ✅ Context-sensitive hotkeys | ✅ Auto-activate on mouse-over |
| **Profile import/export** | N/A | ✅ Script files | ✅ XML export/import |

## 13 · IPC & Remote Control

| Feature | wayclick | AHK | XMBC |
|---|---|---|---|
| **IPC protocol** | ✅ JSON-RPC 2.0 over Unix socket | ⚠️ Possible via Win32/COM/sockets | ❌ |
| **CLI control tool** | ✅ wayclickctl | ❌ | ❌ |
| **Status query** | ✅ Trigger states, uptime, backend | ⚠️ Via script | ❌ |
| **Remote trigger firing** | ✅ `wayclickctl trigger <id>` | ⚠️ Via script | ❌ |
| **Log retrieval** | ✅ `wayclickctl logs --tail N` | ❌ | ❌ |
| **Enable / disable** | ✅ Via IPC | ✅ Suspend/Resume hotkeys | ✅ Enable/disable in tray |

## 14 · Timing & Performance

| Feature | wayclick | AHK | XMBC |
|---|---|---|---|
| **Minimum interval** | 1 ms (configurable floor) | ~1 ms (SetKeyDelay) | ~15 ms (timer resolution) |
| **Interval precision** | ✅ ms-level (1ms poll loop) | ✅ ms-level | ⚠️ 15ms Windows timer floor |
| **Configurable interval** | ✅ Per-action interval_ms | ✅ Sleep / SetKeyDelay | ✅ Repeat delay |
| **Jitter / anti-detection** | ✅ Built-in jitter_ms | ⚠️ Manual via Random() | ✅ {WAITMS:x-y} random |
| **Click hold duration** | ✅ hold_ms per auto-click | ⚠️ Manual press/sleep/release | ✅ {HOLDMS:n} |

## 15 · Security & Permissions

| Feature | wayclick | AHK | XMBC |
|---|---|---|---|
| **Config sandboxing** | ✅ Lua sandbox (no exec, no write) | ❌ Full system access | N/A (GUI-only config) |
| **IPC permissions** | ✅ Unix socket 0600 (user-only) | N/A | N/A |
| **Required groups** | `input` + `wayclick` | N/A (user-level) | N/A (user-level) |
| **udev rules** | ✅ Provided (99-wayclick.rules) | N/A | N/A |
| **No network surface by default** | ✅ Local Unix socket only | ⚠️ None by default; scripts can open network | ✅ Update checker only (outbound) |
| **No shell execution by default** | ✅ Sandboxed | ⚠️ None by default; scripts can call Run/RunWait | ✅ Only app launch (no shell) |
| **Admin required** | ❌ (user-level groups) | ❌ (user-level) | ❌ (user-level) |

## 16 · Logging & Diagnostics

| Feature | wayclick | AHK | XMBC |
|---|---|---|---|
| **Structured logging** | ✅ Ring buffer, ISO8601, levels | ❌ | ✅ Debug log file |
| **JSON log output** | ✅ `--log-json` | ❌ | ❌ |
| **Log levels** | ✅ trace/debug/info/warn/error | ❌ | ⚠️ Debug on/off |
| **Dry-run mode** | ✅ Log actions without emitting | ❌ | ❌ |
| **Permissions check** | ✅ `--check-permissions` | N/A | N/A |
| **Device monitor tool** | ✅ wayclick-evdev-dump monitor | ❌ | ❌ |

## 17 · Documentation & Ecosystem

| Feature | wayclick | AHK | XMBC |
|---|---|---|---|
| **Official docs** | ✅ Markdown (CONFIG_SCHEMA, ARCHITECTURE, SECURITY, etc.) | ✅ Extensive HTML docs site | ✅ PDF user guide |
| **Community** | Small (early project) | ✅ Very large (forums, Reddit, GitHub) | ✅ Moderate (forums, Discord) |
| **Third-party scripts/plugins** | ❌ | ✅ Huge library (awesome-ahk) | ❌ |
| **Language packs / i18n** | ❌ | ❌ (English docs) | ✅ 23+ languages |
| **Editor support** | Any text editor (Lua) | ✅ Dedicated editors, VS Code extension | N/A (GUI-based) |

## 18 · Licensing & Cost

| Feature | wayclick | AHK | XMBC |
|---|---|---|---|
| **Cost** | Free | Free | Free |
| **License** | Open source (see repo) | GNU GPLv2 (open source) | Freeware (closed source) |
| **Source code available** | ✅ Rust on GitHub | ✅ C++ on GitHub | ❌ Closed source |
| **Commercial use** | ✅ | ✅ (GPL terms) | ✅ |
| **Modification allowed** | ✅ | ✅ (GPL terms) | ❌ |

---

## Summary: When to Use What

| Scenario | Best choice | Why |
|---|---|---|
| Linux mouse automation | **wayclick** | Linux-native option in this comparison; kernel-level, Wayland-native |
| Windows general-purpose automation | **AutoHotkey** | Full scripting language, window management, GUI creation |
| Simple Windows mouse button remapping | **XMBC** | Easy GUI, per-app profiles, no scripting needed |
| Per-device button binding | **wayclick** | VID:PID matching, exclusive grab, hotplug |
| Timing randomisation / jitter | **wayclick** | Built-in jitter_ms, hold_ms, kernel-level input |
| Hotstrings / text expansion | **AutoHotkey** | Native hotstring support |
| Window-context-sensitive actions | **AHK** or **XMBC** | Both support per-app/window profiles |
| Headless / server automation | **wayclick** | systemd daemon, IPC control, no GUI needed |
| Cross-application clipboard workflow | **AutoHotkey** | Full clipboard + window + file I/O access |

---

*Generated from: wayclick codebase analysis, [AutoHotkey v2 docs](https://www.autohotkey.com/docs/v2/), [XMBC site (archived)](https://web.archive.org/web/2024/https://www.highrez.co.uk/downloads/XMouseButtonControl.htm).*
