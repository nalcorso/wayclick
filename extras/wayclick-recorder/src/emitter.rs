// SPDX-License-Identifier: MIT
//! Translates captured input events into Lua snippets the user can paste
//! into existing wayclick scripts.
//!
//! The emitter is event-driven and stateful: it pairs press/release events
//! into `wayclick.keystroke` / `wayclick.click_at` calls, tracks held
//! modifiers so they collapse into a single `modifiers = { ... }` table,
//! and renders inter-event delays.
//!
//! Output is **not required to be syntactically valid Lua**. Events that
//! can't be paired (e.g. an orphan press) are emitted as `-- comment`
//! lines so no information is silently lost. The user is expected to
//! paste this into a larger script and clean up comments as needed.

use crate::filter::{EventClass, FilterSet};
use crate::keymap::{self, click_at_button, modifier_name, CodeKind, CodeName};
use std::io::{self, Write};
use wayclick_ipc_client::types::{CursorPosition, MonitorInfo};

/// Output format. Currently only `Raw` is fully supported; `Script` wraps
/// the raw output in a stub `wayclick.register_trigger { ... }` skeleton
/// with TODO placeholders.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    /// Bare Lua statements, one per line. Default.
    Raw,
    /// Wraps the raw block in a `wayclick.register_trigger` skeleton.
    Script,
}

/// Coordinate space used when emitting `click_at` statements.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoordSpace {
    /// Emit `click_at({ x = local_x, y = local_y, monitor = "DP-2" })`
    /// using coordinates **relative to the containing monitor**.
    /// Falls back to `Global` automatically when no monitor list was
    /// supplied (e.g. daemon doesn't support `get_monitors`).
    Monitor,
    /// Emit `click_at({ x = global_x, y = global_y })` in the
    /// compositor's global pixel layout.
    Global,
}

/// Source-of-truth event passed to the emitter. Constructed by the
/// recorder's event loop from the IPC stream.
#[derive(Debug, Clone)]
pub enum CapturedEvent {
    /// Key or button press/release. `value` is 1 for press, 0 for release.
    Input {
        code: u16,
        value: i32,
        timestamp_ms: u64,
    },
    /// Wheel detent. `delta_x`/`delta_y` follow evdev REL_HWHEEL/REL_WHEEL
    /// conventions (positive = right/up).
    Scroll {
        delta_x: i32,
        delta_y: i32,
        timestamp_ms: u64,
    },
}

impl CapturedEvent {
    #[allow(dead_code)]
    fn timestamp_ms(&self) -> u64 {
        match self {
            CapturedEvent::Input { timestamp_ms, .. }
            | CapturedEvent::Scroll { timestamp_ms, .. } => *timestamp_ms,
        }
    }
}

/// State kept across events to pair press+release pairs.
#[derive(Debug, Clone)]
struct PendingPress {
    code: u16,
    name: CodeName,
    press_ts_ms: u64,
    /// Cursor position captured at press time for mouse buttons; `None` for keys
    /// or when the daemon can't report cursor position.
    cursor: Option<CursorPosition>,
    /// Set of held modifier names at the moment of press (for keystroke pairing).
    modifiers_at_press: Vec<&'static str>,
}

/// Renders [`CapturedEvent`]s as Lua statements onto a [`Write`] sink.
///
/// The emitter intentionally keeps **no** ring buffer of past events —
/// the recorder is responsible for back-pressuring its event source.
/// Memory use is O(held-keys).
pub struct Emitter<W: Write> {
    out: W,
    filter: FilterSet,
    format: OutputFormat,
    coord_space: CoordSpace,
    /// Monitor layout used to classify clicks when `coord_space ==
    /// Monitor`. Empty when the daemon doesn't expose monitor info — the
    /// emitter then falls back to global coords automatically.
    monitors: Vec<MonitorInfo>,
    /// Codes currently held down (any kind). Used both for modifier
    /// detection and for press/release pairing.
    pressed: Vec<PendingPress>,
    /// Timestamp of the previous *emitted* event, used to compute delays.
    last_emit_ts_ms: Option<u64>,
    /// Whether we've already warned (in-band, as a comment) that cursor
    /// position isn't available from the daemon. Rate-limited to once
    /// per recording session.
    cursor_warning_emitted: bool,
    /// Whether we've already warned (in-band, as a comment) that one or
    /// more clicks fell outside every known monitor. Rate-limited.
    off_monitor_warning_emitted: bool,
    /// Counter of statements written, for the optional `Script` wrapper.
    statement_count: u32,
}

impl<W: Write> Emitter<W> {
    pub fn new(out: W, filter: FilterSet, format: OutputFormat) -> Self {
        Self::with_coords(out, filter, format, CoordSpace::Global, Vec::new())
    }

    /// Construct an emitter with explicit coordinate-space configuration.
    /// When `coord_space == Monitor` and `monitors` is non-empty, clicks
    /// are emitted as `click_at({ x=local, y=local, monitor="..." })`.
    /// Otherwise behaviour is identical to [`Emitter::new`] (global coords).
    pub fn with_coords(
        out: W,
        filter: FilterSet,
        format: OutputFormat,
        coord_space: CoordSpace,
        monitors: Vec<MonitorInfo>,
    ) -> Self {
        Emitter {
            out,
            filter,
            format,
            coord_space,
            monitors,
            pressed: Vec::with_capacity(8),
            last_emit_ts_ms: None,
            cursor_warning_emitted: false,
            off_monitor_warning_emitted: false,
            statement_count: 0,
        }
    }

    /// Writes the format prologue (currently only `Script` emits a header).
    pub fn begin(&mut self) -> io::Result<()> {
        if self.format == OutputFormat::Script {
            writeln!(self.out, "-- BEGIN wayclick-recorder transcript")?;
            writeln!(self.out, "wayclick.register_trigger {{")?;
            writeln!(self.out, "  id = \"TODO-recorded\",")?;
            writeln!(self.out, "  name = \"Recorded macro\",")?;
            writeln!(self.out, "  mode = \"oneshot\",")?;
            writeln!(self.out, "  action = wayclick.sequence {{")?;
        }
        Ok(())
    }

    /// Writes the format epilogue.
    pub fn end(&mut self) -> io::Result<()> {
        // Flush any orphan presses as comments so the user sees them.
        let orphans: Vec<PendingPress> = self.pressed.drain(..).collect();
        for p in orphans {
            writeln!(
                self.out,
                "-- orphan press: {} (no matching release captured)",
                p.name.lua_name
            )?;
        }
        if self.format == OutputFormat::Script {
            writeln!(self.out, "  }},")?;
            writeln!(self.out, "}}")?;
            writeln!(self.out, "-- END wayclick-recorder transcript")?;
        }
        self.out.flush()?;
        Ok(())
    }

    /// Feeds one captured event into the emitter. Cursor position should be
    /// supplied only for `Input` events whose `value == 1` and whose `code`
    /// is a mouse button; the recorder is responsible for querying the
    /// daemon at the press instant. The emitter does no IPC.
    pub fn push(&mut self, ev: &CapturedEvent, cursor: Option<CursorPosition>) -> io::Result<()> {
        match ev {
            CapturedEvent::Input {
                code,
                value,
                timestamp_ms,
            } => self.push_input(*code, *value, *timestamp_ms, cursor),
            CapturedEvent::Scroll {
                delta_x,
                delta_y,
                timestamp_ms,
            } => self.push_scroll(*delta_x, *delta_y, *timestamp_ms),
        }
    }

    /// Emits a one-line comment into the transcript (e.g. a runtime warning).
    /// Does not advance the delay clock.
    pub fn comment<S: AsRef<str>>(&mut self, msg: S) -> io::Result<()> {
        writeln!(self.out, "-- {}", msg.as_ref())
    }

    fn push_input(
        &mut self,
        code: u16,
        value: i32,
        ts_ms: u64,
        cursor: Option<CursorPosition>,
    ) -> io::Result<()> {
        let name = keymap::resolve(code);
        let class = match name.kind {
            CodeKind::Key | CodeKind::Other => EventClass::Key(code),
            CodeKind::MouseButton => EventClass::Button(code),
        };

        if !self.filter.should_emit(class) {
            // Still track pressed state for modifier-pairing accuracy of
            // *other* keys, but skip emission.
            if value == 1 {
                self.note_pressed(code, name, ts_ms, cursor);
            } else if value == 0 {
                self.pressed.retain(|p| p.code != code);
            }
            return Ok(());
        }

        if value == 1 {
            // Modifiers are *only* recorded as held state — we never emit
            // a standalone keystroke for them on press. Their release will
            // pop them off `pressed` and they will appear as a `modifiers`
            // list on whatever non-modifier press happens between.
            //
            // For non-modifier presses, write the inter-event delay now
            // (relative to the previous emission's release time). This way
            // the captured delay reflects the user's "thinking time"
            // between releasing one key/button and pressing the next.
            if modifier_name(code).is_none() {
                self.write_delay_before(ts_ms)?;
            }
            self.note_pressed(code, name, ts_ms, cursor);
            return Ok(());
        }

        if value == 0 {
            // Find the matching press. If none, emit a comment.
            let pos = self.pressed.iter().rposition(|p| p.code == code);
            let press = match pos {
                Some(idx) => self.pressed.remove(idx),
                None => {
                    writeln!(
                        self.out,
                        "-- orphan release: {} (no matching press)",
                        name.lua_name
                    )?;
                    return Ok(());
                }
            };

            // Modifier releases are silent — they only matter as state.
            if modifier_name(code).is_some() {
                return Ok(());
            }

            self.emit_press_release(&press, ts_ms, cursor)?;
            self.last_emit_ts_ms = Some(ts_ms);
            self.statement_count += 1;
            return Ok(());
        }

        // value == 2 (key repeat) — wayclick filters these on the bus side,
        // but be defensive.
        Ok(())
    }

    fn note_pressed(
        &mut self,
        code: u16,
        name: CodeName,
        ts_ms: u64,
        cursor: Option<CursorPosition>,
    ) {
        let modifiers_at_press = self.current_modifiers();
        // Drop any prior press of the same code (no release was observed).
        self.pressed.retain(|p| p.code != code);
        self.pressed.push(PendingPress {
            code,
            name,
            press_ts_ms: ts_ms,
            cursor,
            modifiers_at_press,
        });
    }

    fn current_modifiers(&self) -> Vec<&'static str> {
        let mut mods: Vec<&'static str> = self
            .pressed
            .iter()
            .filter_map(|p| modifier_name(p.code))
            .collect();
        mods.sort_unstable();
        mods.dedup();
        mods
    }

    fn emit_press_release(
        &mut self,
        press: &PendingPress,
        release_ts_ms: u64,
        release_cursor: Option<CursorPosition>,
    ) -> io::Result<()> {
        match press.name.kind {
            CodeKind::Other => {
                writeln!(
                    self.out,
                    "-- unknown evdev code {} (recorded as press+release)",
                    press.code
                )?;
            }
            CodeKind::Key => {
                self.emit_keystroke(press)?;
            }
            CodeKind::MouseButton => {
                self.emit_button(press, release_ts_ms, release_cursor)?;
            }
        }
        Ok(())
    }

    fn emit_keystroke(&mut self, press: &PendingPress) -> io::Result<()> {
        let mut line = format!(
            "wayclick.keystroke({{ key = \"{}\"",
            lua_escape(&press.name.lua_name)
        );
        if !press.modifiers_at_press.is_empty() {
            line.push_str(", modifiers = { ");
            for (i, m) in press.modifiers_at_press.iter().enumerate() {
                if i > 0 {
                    line.push_str(", ");
                }
                line.push('"');
                line.push_str(m);
                line.push('"');
            }
            line.push_str(" }");
        }
        line.push_str(" })");
        writeln!(self.out, "{}", self.indent_line(&line))
    }

    fn emit_button(
        &mut self,
        press: &PendingPress,
        release_ts_ms: u64,
        release_cursor: Option<CursorPosition>,
    ) -> io::Result<()> {
        let hold_ms = release_ts_ms.saturating_sub(press.press_ts_ms);

        // Prefer the cursor captured at *press* time (most semantically
        // correct for "click here") but fall back to the release-time
        // sample if the press-time query failed.
        let cursor = press.cursor.or(release_cursor);

        let click_target = click_at_button(press.code);
        let emit_as_click = !self.filter.no_clicks && click_target.is_some() && cursor.is_some();

        if !press.modifiers_at_press.is_empty() && emit_as_click {
            // `click_at` doesn't accept a `modifiers` field today (verified
            // against config.rs). Surface the held modifiers as a comment so
            // the user can decide whether to wrap the click manually.
            let mods = press.modifiers_at_press.join(", ");
            writeln!(
                self.out,
                "{}",
                self.indent_line(&format!("-- modifiers held: {}", mods))
            )?;
        }

        if emit_as_click {
            let pos = cursor.expect("checked above");
            let button = click_target.expect("checked above");

            // Decide between monitor-local and global coords.
            let (x_out, y_out, monitor_name) = if self.coord_space == CoordSpace::Monitor
                && !self.monitors.is_empty()
            {
                match self.monitors.iter().find(|m| m.contains(pos.x, pos.y)) {
                    Some(m) => (pos.x - m.x, pos.y - m.y, Some(m.name.clone())),
                    None => {
                        // Click landed outside every known monitor — emit
                        // global coords with a one-time explanatory comment.
                        if !self.off_monitor_warning_emitted {
                            writeln!(
                                self.out,
                                "{}",
                                self.indent_line(
                                    "-- click outside known monitor layout; falling back to global coordinates"
                                )
                            )?;
                            self.off_monitor_warning_emitted = true;
                        }
                        (pos.x, pos.y, None)
                    }
                }
            } else {
                (pos.x, pos.y, None)
            };

            let mut line = format!(
                "wayclick.click_at({{ x = {}, y = {}, button = \"{}\"",
                x_out, y_out, button
            );
            if let Some(name) = monitor_name {
                line.push_str(&format!(", monitor = \"{}\"", lua_escape(&name)));
            }
            if hold_ms > 1 {
                line.push_str(&format!(", hold_ms = {}", hold_ms.min(u32::MAX as u64)));
            }
            line.push_str(" })");
            writeln!(self.out, "{}", self.indent_line(&line))?;
            return Ok(());
        }

        // Fallback: emit as a keystroke using the raw BTN_* name.
        if cursor.is_none()
            && click_target.is_some()
            && !self.filter.no_clicks
            && !self.cursor_warning_emitted
        {
            writeln!(
                self.out,
                "{}",
                self.indent_line(
                    "-- cursor position unavailable; falling back to keystroke (install a Hyprland-compatible compositor for click_at output)"
                )
            )?;
            self.cursor_warning_emitted = true;
        }

        // Construct a keystroke line with modifiers (since this is the keystroke path).
        let mut tmp = PendingPress {
            modifiers_at_press: press.modifiers_at_press.clone(),
            ..press.clone()
        };
        // Keystroke uses the resolved button name (e.g. "BTN_LEFT") which
        // the daemon's `keystroke` action accepts.
        tmp.name = CodeName {
            lua_name: press.name.lua_name.clone(),
            kind: CodeKind::Key,
            synthetic: false,
        };
        self.emit_keystroke(&tmp)
    }

    fn push_scroll(&mut self, delta_x: i32, delta_y: i32, ts_ms: u64) -> io::Result<()> {
        if !self.filter.should_emit(EventClass::Scroll) {
            return Ok(());
        }
        self.write_delay_before(ts_ms)?;

        for d in 0..delta_y.unsigned_abs() {
            let _ = d;
            let dir = if delta_y > 0 { "up" } else { "down" };
            writeln!(
                self.out,
                "{}",
                self.indent_line(&format!("wayclick.scroll({{ direction = \"{}\" }})", dir))
            )?;
            self.statement_count += 1;
        }
        for d in 0..delta_x.unsigned_abs() {
            let _ = d;
            let dir = if delta_x > 0 { "right" } else { "left" };
            writeln!(
                self.out,
                "{}",
                self.indent_line(&format!("wayclick.scroll({{ direction = \"{}\" }})", dir))
            )?;
            self.statement_count += 1;
        }
        self.last_emit_ts_ms = Some(ts_ms);
        Ok(())
    }

    fn write_delay_before(&mut self, now_ms: u64) -> io::Result<()> {
        if let Some(prev) = self.last_emit_ts_ms {
            let gap = now_ms.saturating_sub(prev);
            let gap_u32: u32 = gap.try_into().unwrap_or(u32::MAX);
            if self.filter.should_emit_delay(gap_u32) && gap > 0 {
                writeln!(
                    self.out,
                    "{}",
                    self.indent_line(&format!("wayclick.delay({{ ms = {} }})", gap_u32))
                )?;
            }
        }
        Ok(())
    }

    fn indent_line(&self, line: &str) -> String {
        if self.format == OutputFormat::Script {
            format!("    {}", line)
        } else {
            line.to_string()
        }
    }

    /// Number of Lua statements (excluding comments) written so far.
    pub fn statement_count(&self) -> u32 {
        self.statement_count
    }
}

/// Escapes a string for embedding inside a Lua `"..."` literal. Wayclick
/// key names are ASCII-only by construction, but defensively escape `"` and `\`.
fn lua_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            c if c.is_control() => out.push_str(&format!("\\{}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(t: u64) -> u64 {
        t
    }

    fn emit_all<F: FnOnce(&mut Emitter<Vec<u8>>) -> io::Result<()>>(
        filter: FilterSet,
        f: F,
    ) -> String {
        let buf: Vec<u8> = Vec::new();
        let mut em = Emitter::new(buf, filter, OutputFormat::Raw);
        em.begin().unwrap();
        f(&mut em).unwrap();
        em.end().unwrap();
        String::from_utf8(em.out).unwrap()
    }

    fn emit_with_monitors<F: FnOnce(&mut Emitter<Vec<u8>>) -> io::Result<()>>(
        filter: FilterSet,
        monitors: Vec<MonitorInfo>,
        f: F,
    ) -> String {
        let buf: Vec<u8> = Vec::new();
        let mut em = Emitter::with_coords(
            buf,
            filter,
            OutputFormat::Raw,
            CoordSpace::Monitor,
            monitors,
        );
        em.begin().unwrap();
        f(&mut em).unwrap();
        em.end().unwrap();
        String::from_utf8(em.out).unwrap()
    }

    fn test_monitors() -> Vec<MonitorInfo> {
        vec![
            MonitorInfo {
                name: "HDMI-A-1".into(),
                description: "left".into(),
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
                scale: 1.0,
                transform: 0,
            },
            MonitorInfo {
                name: "DP-2".into(),
                description: "centre".into(),
                x: 1920,
                y: 0,
                width: 2560,
                height: 1440,
                scale: 1.5,
                transform: 0,
            },
        ]
    }

    #[test]
    fn keystroke_pair_emits_lua() {
        let out = emit_all(FilterSet::default(), |em| {
            em.push(
                &CapturedEvent::Input {
                    code: 28,
                    value: 1,
                    timestamp_ms: ms(100),
                },
                None,
            )?;
            em.push(
                &CapturedEvent::Input {
                    code: 28,
                    value: 0,
                    timestamp_ms: ms(150),
                },
                None,
            )?;
            Ok(())
        });
        assert!(
            out.contains("wayclick.keystroke({ key = \"enter\" })"),
            "got: {out}"
        );
    }

    #[test]
    fn delay_between_two_keystrokes() {
        let out = emit_all(FilterSet::default(), |em| {
            em.push(
                &CapturedEvent::Input {
                    code: 28,
                    value: 1,
                    timestamp_ms: 100,
                },
                None,
            )?;
            em.push(
                &CapturedEvent::Input {
                    code: 28,
                    value: 0,
                    timestamp_ms: 110,
                },
                None,
            )?;
            em.push(
                &CapturedEvent::Input {
                    code: 30,
                    value: 1,
                    timestamp_ms: 250,
                },
                None,
            )?;
            em.push(
                &CapturedEvent::Input {
                    code: 30,
                    value: 0,
                    timestamp_ms: 260,
                },
                None,
            )?;
            Ok(())
        });
        assert!(out.contains("wayclick.delay({ ms = 140 })"), "got: {out}");
    }

    #[test]
    fn delay_below_min_is_dropped() {
        let f = FilterSet {
            min_delay_ms: 50,
            ..Default::default()
        };
        let out = emit_all(f, |em| {
            em.push(
                &CapturedEvent::Input {
                    code: 28,
                    value: 1,
                    timestamp_ms: 100,
                },
                None,
            )?;
            em.push(
                &CapturedEvent::Input {
                    code: 28,
                    value: 0,
                    timestamp_ms: 110,
                },
                None,
            )?;
            em.push(
                &CapturedEvent::Input {
                    code: 30,
                    value: 1,
                    timestamp_ms: 130,
                },
                None,
            )?;
            em.push(
                &CapturedEvent::Input {
                    code: 30,
                    value: 0,
                    timestamp_ms: 140,
                },
                None,
            )?;
            Ok(())
        });
        assert!(
            !out.contains("wayclick.delay"),
            "delay should be dropped, got: {out}"
        );
    }

    #[test]
    fn modifier_collapses_into_keystroke() {
        let out = emit_all(FilterSet::default(), |em| {
            // ctrl down, a down, a up, ctrl up
            em.push(
                &CapturedEvent::Input {
                    code: 29,
                    value: 1,
                    timestamp_ms: 100,
                },
                None,
            )?;
            em.push(
                &CapturedEvent::Input {
                    code: 30,
                    value: 1,
                    timestamp_ms: 110,
                },
                None,
            )?;
            em.push(
                &CapturedEvent::Input {
                    code: 30,
                    value: 0,
                    timestamp_ms: 130,
                },
                None,
            )?;
            em.push(
                &CapturedEvent::Input {
                    code: 29,
                    value: 0,
                    timestamp_ms: 140,
                },
                None,
            )?;
            Ok(())
        });
        assert!(
            out.contains(r#"wayclick.keystroke({ key = "a", modifiers = { "ctrl" } })"#),
            "got: {out}"
        );
        // Modifier itself should not appear as a standalone keystroke.
        assert!(!out.contains(r#"key = "leftctrl""#), "got: {out}");
    }

    #[test]
    fn click_at_emitted_with_cursor() {
        let out = emit_all(FilterSet::default(), |em| {
            em.push(
                &CapturedEvent::Input {
                    code: 0x110,
                    value: 1,
                    timestamp_ms: 100,
                },
                Some(CursorPosition { x: 42, y: 84 }),
            )?;
            em.push(
                &CapturedEvent::Input {
                    code: 0x110,
                    value: 0,
                    timestamp_ms: 150,
                },
                None,
            )?;
            Ok(())
        });
        assert!(
            out.contains(r#"wayclick.click_at({ x = 42, y = 84, button = "left", hold_ms = 50 })"#),
            "got: {out}"
        );
    }

    #[test]
    fn click_falls_back_to_keystroke_without_cursor() {
        let out = emit_all(FilterSet::default(), |em| {
            em.push(
                &CapturedEvent::Input {
                    code: 0x110,
                    value: 1,
                    timestamp_ms: 100,
                },
                None,
            )?;
            em.push(
                &CapturedEvent::Input {
                    code: 0x110,
                    value: 0,
                    timestamp_ms: 150,
                },
                None,
            )?;
            Ok(())
        });
        assert!(
            out.contains(r#"wayclick.keystroke({ key = "BTN_LEFT" })"#),
            "got: {out}"
        );
        assert!(out.contains("cursor position unavailable"), "got: {out}");
    }

    #[test]
    fn no_clicks_filter_forces_keystroke() {
        let f = FilterSet {
            no_clicks: true,
            ..Default::default()
        };
        let out = emit_all(f, |em| {
            em.push(
                &CapturedEvent::Input {
                    code: 0x110,
                    value: 1,
                    timestamp_ms: 100,
                },
                Some(CursorPosition { x: 1, y: 2 }),
            )?;
            em.push(
                &CapturedEvent::Input {
                    code: 0x110,
                    value: 0,
                    timestamp_ms: 110,
                },
                None,
            )?;
            Ok(())
        });
        assert!(
            out.contains(r#"wayclick.keystroke({ key = "BTN_LEFT" })"#),
            "got: {out}"
        );
        assert!(!out.contains("click_at"), "got: {out}");
    }

    #[test]
    fn scroll_emits_per_detent() {
        let out = emit_all(FilterSet::default(), |em| {
            em.push(
                &CapturedEvent::Scroll {
                    delta_x: 0,
                    delta_y: 3,
                    timestamp_ms: 100,
                },
                None,
            )?;
            Ok(())
        });
        let up_count = out
            .matches(r#"wayclick.scroll({ direction = "up" })"#)
            .count();
        assert_eq!(up_count, 3, "got: {out}");
    }

    #[test]
    fn orphan_release_is_commented() {
        let out = emit_all(FilterSet::default(), |em| {
            em.push(
                &CapturedEvent::Input {
                    code: 28,
                    value: 0,
                    timestamp_ms: 100,
                },
                None,
            )?;
            Ok(())
        });
        assert!(out.contains("-- orphan release: enter"), "got: {out}");
    }

    #[test]
    fn orphan_press_flushed_on_end() {
        let out = emit_all(FilterSet::default(), |em| {
            em.push(
                &CapturedEvent::Input {
                    code: 28,
                    value: 1,
                    timestamp_ms: 100,
                },
                None,
            )?;
            Ok(())
        });
        assert!(out.contains("-- orphan press: enter"), "got: {out}");
    }

    #[test]
    fn no_keys_filter_drops_keystrokes() {
        let f = FilterSet {
            no_keys: true,
            ..Default::default()
        };
        let out = emit_all(f, |em| {
            em.push(
                &CapturedEvent::Input {
                    code: 28,
                    value: 1,
                    timestamp_ms: 100,
                },
                None,
            )?;
            em.push(
                &CapturedEvent::Input {
                    code: 28,
                    value: 0,
                    timestamp_ms: 110,
                },
                None,
            )?;
            Ok(())
        });
        assert!(!out.contains("keystroke"), "got: {out}");
    }

    #[test]
    fn click_at_emits_monitor_local_coords() {
        let out = emit_with_monitors(FilterSet::default(), test_monitors(), |em| {
            // (4103, 1370) is inside DP-2 (origin 1920,0, size 2560x1440).
            em.push(
                &CapturedEvent::Input {
                    code: 0x110,
                    value: 1,
                    timestamp_ms: 100,
                },
                Some(CursorPosition { x: 4103, y: 1370 }),
            )?;
            em.push(
                &CapturedEvent::Input {
                    code: 0x110,
                    value: 0,
                    timestamp_ms: 110,
                },
                None,
            )?;
            Ok(())
        });
        // Expect local coords (4103-1920, 1370-0) = (2183, 1370) on DP-2.
        assert!(
            out.contains(r#"x = 2183, y = 1370, button = "left", monitor = "DP-2""#),
            "got: {out}"
        );
    }

    #[test]
    fn click_outside_monitors_emits_global_with_warning() {
        let out = emit_with_monitors(FilterSet::default(), test_monitors(), |em| {
            // Outside both monitors.
            em.push(
                &CapturedEvent::Input {
                    code: 0x110,
                    value: 1,
                    timestamp_ms: 100,
                },
                Some(CursorPosition { x: 9999, y: 9999 }),
            )?;
            em.push(
                &CapturedEvent::Input {
                    code: 0x110,
                    value: 0,
                    timestamp_ms: 110,
                },
                None,
            )?;
            Ok(())
        });
        assert!(out.contains("outside known monitor layout"), "got: {out}");
        assert!(
            out.contains(r#"x = 9999, y = 9999"#) && !out.contains("monitor ="),
            "got: {out}"
        );
    }
}
