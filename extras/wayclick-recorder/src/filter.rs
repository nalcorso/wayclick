// SPDX-License-Identifier: MIT
//! Event-class filters that decide which captured input events make it
//! into the emitted Lua transcript.
//!
//! Filters are *subtractive*: by default every event is included, and each
//! `--no-*` flag drops a category. The recorder evaluates a single
//! [`FilterSet`] per event; no allocation occurs on the hot path.

/// Set of toggleable filters applied to each captured event.
///
/// Default = include everything; flip individual fields to drop a class.
#[derive(Debug, Clone, Copy, Default)]
pub struct FilterSet {
    /// When true, key press/release events (`KEY_*` codes) are dropped.
    pub no_keys: bool,
    /// When true, mouse button events (`BTN_*` codes) are dropped.
    pub no_buttons: bool,
    /// When true, mouse buttons are emitted as `wayclick.keystroke({ key = "BTN_*" })`
    /// instead of `wayclick.click_at({ x, y, ... })`. Useful for binds that
    /// should fire regardless of cursor location.
    pub no_clicks: bool,
    /// When true, wheel events are dropped.
    pub no_scroll: bool,
    /// When true, inter-event `wayclick.delay({...})` lines are not emitted.
    pub no_delays: bool,
    /// Delays shorter than this many ms are dropped (or coalesced into the
    /// next emission). `0` keeps everything.
    pub min_delay_ms: u32,
}

/// Classification of an incoming event from the IPC stream.
#[derive(Debug, Clone, Copy)]
pub enum EventClass {
    /// Keyboard key event with the given evdev code.
    Key(u16),
    /// Mouse button event with the given evdev code.
    Button(u16),
    /// Scroll wheel event.
    Scroll,
}

impl FilterSet {
    /// Returns `true` if an event with the given classification should be
    /// emitted, `false` if filtered out.
    pub fn should_emit(&self, class: EventClass) -> bool {
        match class {
            EventClass::Key(_) => !self.no_keys,
            EventClass::Button(_) => !self.no_buttons,
            EventClass::Scroll => !self.no_scroll,
        }
    }

    /// Returns `true` if a measured inter-event gap should be emitted as
    /// a `wayclick.delay({...})` line.
    pub fn should_emit_delay(&self, ms: u32) -> bool {
        !self.no_delays && ms >= self.min_delay_ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_emits_everything() {
        let f = FilterSet::default();
        assert!(f.should_emit(EventClass::Key(28)));
        assert!(f.should_emit(EventClass::Button(0x110)));
        assert!(f.should_emit(EventClass::Scroll));
        assert!(f.should_emit_delay(50));
        assert!(f.should_emit_delay(0));
    }

    #[test]
    fn no_keys_drops_keys_only() {
        let f = FilterSet {
            no_keys: true,
            ..Default::default()
        };
        assert!(!f.should_emit(EventClass::Key(28)));
        assert!(f.should_emit(EventClass::Button(0x110)));
        assert!(f.should_emit(EventClass::Scroll));
    }

    #[test]
    fn min_delay_threshold() {
        let f = FilterSet {
            min_delay_ms: 50,
            ..Default::default()
        };
        assert!(!f.should_emit_delay(49));
        assert!(f.should_emit_delay(50));
        assert!(f.should_emit_delay(1000));
    }

    #[test]
    fn no_delays_overrides_threshold() {
        let f = FilterSet {
            no_delays: true,
            min_delay_ms: 0,
            ..Default::default()
        };
        assert!(!f.should_emit_delay(0));
        assert!(!f.should_emit_delay(10_000));
    }
}
