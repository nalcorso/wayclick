# wayclick-recorder

A streaming CLI that records input events from a running [wayclick](https://github.com/nalcorso/wayclick)
daemon and emits a sequence of Lua snippets that replay them. Designed to be
piped or pasted into existing Lua scripts — output is intentionally a bare
block of statements rather than a complete program.

## How it works

1. Connects to `wayclickd` over IPC using `wayclick-ipc-client`.
2. Subscribes to the event stream and filters by focused window
   (`app_id` / `title`, case-insensitive substring match).
3. On each key/button release, emits a `wayclick.keystroke{...}` or
   `wayclick.click_at{...}` line. For mouse buttons it queries the daemon's
   `get_cursor_position` IPC method at press-time, so clicks capture real
   screen coordinates (Hyprland only at present).
4. Inter-event delays become `wayclick.delay{ ms = N }` lines.
5. The recorder stops when the stop key is observed in the event stream
   (default `pause`) or on `SIGINT` / `SIGTERM`.

## Quickstart

```sh
# Record all input while a window containing "Idle" in app_id or title is focused.
wayclick-recorder --window Idle

# Same, but drop mouse-scroll lines and clamp short delays.
wayclick-recorder --window Idle --no-scroll --min-delay-ms 25

# Record everything, regardless of window focus, to a file.
wayclick-recorder --any-window --output capture.lua
```

Press the stop key (`--stop-key pause` by default) when you're done. The
recorder writes the captured Lua block to stdout (or `--output PATH`) and
exits.

## CLI

```
Targeting (one required, unless --any-window):
  --window <PATTERN>     Match app_id OR title (repeatable, case-insensitive)
  --app-id <PATTERN>     Match app_id only
  --title  <PATTERN>     Match title only
  --any-window           No window filter
  --display <NAME>       Not yet supported; errors with a clear message.

Filters (subtractive):
  --no-keys              Drop key events (KEY_*).
  --no-buttons           Drop mouse-button events (BTN_*).
  --no-clicks            Force keystroke-form for buttons (skip click_at).
  --no-scroll            Drop scroll events.
  --no-delays            Drop inter-event delay lines.
  --min-delay-ms <N>     Coalesce delays shorter than N ms to zero.

Stop key:
  --stop-key <COMBO>     Default: pause. Examples: pause, scroll_lock, f12.
  --stop-mode <MODE>     sentinel (default; key still reaches the foreground
                         app). `exclusive` is reserved for a future daemon
                         enhancement and currently errors out.

Output:
  --format <FMT>         raw (default) or script (wraps the block in a
                         register_trigger stub).
  --coord-space <SPACE>  monitor (default) emits click_at with monitor-local
                         coordinates plus monitor = "<name>"; global emits
                         global compositor coordinates. Monitor mode auto-
                         falls back to global when the daemon does not
                         support `get_monitors` (e.g. Sway / non-Hyprland).
  --output <PATH|->      Default: stdout.

Misc:
  -v, --verbose          Status messages to stderr.
  -q, --quiet            Suppress non-fatal stderr output.
```

## Example output

```lua
-- recording: app_id="org.wayland.example", title="Example"
wayclick.keystroke({ key = "f5" })
wayclick.delay({ ms = 312 })
wayclick.keystroke({ key = "a", modifiers = { "ctrl" } })
wayclick.delay({ ms = 178 })
wayclick.click_at({ x = 640, y = 412, button = "left", monitor = "DP-2", hold_ms = 42 })
wayclick.scroll({ direction = "down" })
```

## Limitations

- **No pointer-motion paths.** The daemon does not publish `REL_X`/`REL_Y`
  events, so only the destination of each click is recorded.
- **No display filtering.** `--display` is accepted only to surface a
  clear "not yet supported" error.
- **Cursor coordinates require Hyprland.** Other compositors fall back to
  emitting `wayclick.keystroke({ key = "BTN_LEFT" })` plus a one-shot
  comment noting that `click_at` isn't available.
- **`--stop-mode exclusive` is deferred.** Today the stop key is observed
  but not grabbed; it still reaches the foreground application.

## Security

- No `unsafe` code in the recorder crate.
- All resources registered with the daemon are dropped on exit; the daemon
  also cleans up state on disconnect as a safety net.
- Captured events live in a bounded ring buffer (1 M entries) to prevent
  runaway memory use during long sessions; truncation emits a comment.
- Output never echoes the socket path or environment variables.
