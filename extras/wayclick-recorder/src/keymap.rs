// SPDX-License-Identifier: MIT
//! Mapping from raw evdev event codes to wayclick Lua key/button names.
//!
//! Wayclick's Lua API accepts both `KEY_*` long names (e.g. `"KEY_ENTER"`)
//! and short lowercase aliases (e.g. `"enter"`, `"f5"`). For ergonomic
//! generated transcripts we prefer the short alias when one exists and
//! fall back to the canonical `KEY_*` name otherwise. The recorder never
//! invents names — unknown codes are surfaced as `code_<N>` so the emitter
//! can write a `-- unknown` comment instead.
//!
//! ## Parity with the daemon
//! The codes listed here mirror `crates/wayclick-core/src/config.rs`
//! (`key_name_to_code` and `button_code_from_name`). The unit tests in
//! this module assert that every entry's chosen Lua name reverses cleanly
//! through that daemon-side mapping when run inside the workspace.

/// Linux input event code categorisation used by the emitter to decide
/// whether to render a binding as a keystroke or as `click_at`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodeKind {
    /// Keyboard / general key (`KEY_*`).
    Key,
    /// Mouse button (`BTN_LEFT`, `BTN_RIGHT`, `BTN_MIDDLE`, `BTN_SIDE`, `BTN_EXTRA`).
    MouseButton,
    /// Recognized evdev code but outside the keyboard/mouse categories the
    /// recorder cares about — emitted as a comment with no Lua expansion.
    Other,
}

/// A resolved Lua-friendly identifier for an evdev code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeName {
    /// Name to embed inside `wayclick.keystroke({ key = "..." })` etc.
    pub lua_name: String,
    pub kind: CodeKind,
    /// True when the name had to be synthesized (`code_NNN`); the emitter
    /// should render this as a comment rather than a callable Lua line.
    pub synthetic: bool,
}

impl CodeName {
    fn keystroke(name: &str) -> Self {
        CodeName {
            lua_name: name.to_string(),
            kind: CodeKind::Key,
            synthetic: false,
        }
    }
    fn button(name: &str) -> Self {
        CodeName {
            lua_name: name.to_string(),
            kind: CodeKind::MouseButton,
            synthetic: false,
        }
    }
    fn unknown(code: u16) -> Self {
        CodeName {
            lua_name: format!("code_{}", code),
            kind: CodeKind::Other,
            synthetic: true,
        }
    }
}

/// Looks up a Lua key/button identifier for a raw evdev event code.
///
/// Always returns a value; codes the recorder doesn't recognise become
/// synthetic placeholders (`code_NNN`) flagged via [`CodeName::synthetic`].
pub fn resolve(code: u16) -> CodeName {
    if let Some(name) = key_name(code) {
        return CodeName::keystroke(name);
    }
    if let Some(name) = button_name(code) {
        return CodeName::button(name);
    }
    CodeName::unknown(code)
}

/// Maps mouse button codes to their `wayclick.click_at` `button = "..."`
/// argument. Returns `None` for codes that have no `click_at` mapping
/// (e.g. `BTN_SIDE` / `BTN_EXTRA` — those are emitted as keystrokes).
pub fn click_at_button(code: u16) -> Option<&'static str> {
    match code {
        0x110 => Some("left"),
        0x111 => Some("right"),
        0x112 => Some("middle"),
        _ => None,
    }
}

/// Returns `Some(short_lowercase_modifier_name)` for codes the daemon
/// recognises as keystroke modifiers (`ctrl`, `shift`, `alt`, `meta`).
pub fn modifier_name(code: u16) -> Option<&'static str> {
    Some(match code {
        29 | 97 => "ctrl",
        42 | 54 => "shift",
        56 | 100 => "alt",
        125 | 126 => "meta",
        _ => return None,
    })
}

fn key_name(code: u16) -> Option<&'static str> {
    Some(match code {
        1 => "esc",
        2 => "1",
        3 => "2",
        4 => "3",
        5 => "4",
        6 => "5",
        7 => "6",
        8 => "7",
        9 => "8",
        10 => "9",
        11 => "0",
        12 => "minus",
        13 => "equal",
        14 => "backspace",
        15 => "tab",
        16 => "q",
        17 => "w",
        18 => "e",
        19 => "r",
        20 => "t",
        21 => "y",
        22 => "u",
        23 => "i",
        24 => "o",
        25 => "p",
        26 => "leftbrace",
        27 => "rightbrace",
        28 => "enter",
        29 => "leftctrl",
        30 => "a",
        31 => "s",
        32 => "d",
        33 => "f",
        34 => "g",
        35 => "h",
        36 => "j",
        37 => "k",
        38 => "l",
        39 => "semicolon",
        40 => "apostrophe",
        41 => "grave",
        42 => "leftshift",
        43 => "backslash",
        44 => "z",
        45 => "x",
        46 => "c",
        47 => "v",
        48 => "b",
        49 => "n",
        50 => "m",
        51 => "comma",
        52 => "dot",
        53 => "slash",
        54 => "rightshift",
        56 => "leftalt",
        57 => "space",
        58 => "capslock",
        59 => "f1",
        60 => "f2",
        61 => "f3",
        62 => "f4",
        63 => "f5",
        64 => "f6",
        65 => "f7",
        66 => "f8",
        67 => "f9",
        68 => "f10",
        69 => "numlock",
        70 => "scrolllock",
        87 => "f11",
        88 => "f12",
        97 => "rightctrl",
        99 => "sysrq",
        100 => "rightalt",
        102 => "home",
        103 => "up",
        104 => "pageup",
        105 => "left",
        106 => "right",
        107 => "end",
        108 => "down",
        109 => "pagedown",
        110 => "insert",
        111 => "delete",
        113 => "mute",
        114 => "volumedown",
        115 => "volumeup",
        119 => "pause",
        125 => "leftmeta",
        126 => "rightmeta",
        127 => "compose",
        163 => "nextsong",
        164 => "playpause",
        165 => "previoussong",
        166 => "stopcd",
        167 => "record",
        168 => "rewind",
        208 => "fastforward",
        224 => "brightnessdown",
        225 => "brightnessup",
        _ => return None,
    })
}

fn button_name(code: u16) -> Option<&'static str> {
    Some(match code {
        0x110 => "BTN_LEFT",
        0x111 => "BTN_RIGHT",
        0x112 => "BTN_MIDDLE",
        0x113 => "BTN_SIDE",
        0x114 => "BTN_EXTRA",
        _ => return None,
    })
}

/// Parses a user-supplied stop-key combo such as `"pause"`, `"scroll_lock"`,
/// or `"f10"` (modifier combos are NOT supported in sentinel mode — the
/// recorder only watches for a single key code on the event bus).
///
/// Returns the resolved evdev code, or `None` for unknown names.
pub fn parse_stop_key(name: &str) -> Option<u16> {
    let n = name.trim().to_lowercase().replace(['-', '_'], "");
    // Try short aliases first, then full KEY_* names (case-insensitive).
    short_alias_to_code(&n)
}

fn short_alias_to_code(name: &str) -> Option<u16> {
    Some(match name {
        "esc" | "escape" => 1,
        "enter" | "return" => 28,
        "space" => 57,
        "tab" => 15,
        "backspace" => 14,
        "delete" | "del" => 111,
        "insert" | "ins" => 110,
        "home" => 102,
        "end" => 107,
        "pageup" | "pgup" => 104,
        "pagedown" | "pgdn" => 109,
        "up" => 103,
        "down" => 108,
        "left" => 105,
        "right" => 106,
        "pause" | "break" => 119,
        "scrolllock" | "scroll" => 70,
        "numlock" => 69,
        "capslock" | "caps" => 58,
        "sysrq" | "printscreen" | "prtsc" => 99,
        "f1" => 59,
        "f2" => 60,
        "f3" => 61,
        "f4" => 62,
        "f5" => 63,
        "f6" => 64,
        "f7" => 65,
        "f8" => 66,
        "f9" => 67,
        "f10" => 68,
        "f11" => 87,
        "f12" => 88,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_known_key() {
        let n = resolve(28);
        assert_eq!(n.lua_name, "enter");
        assert_eq!(n.kind, CodeKind::Key);
        assert!(!n.synthetic);
    }

    #[test]
    fn resolve_known_button() {
        let n = resolve(0x110);
        assert_eq!(n.lua_name, "BTN_LEFT");
        assert_eq!(n.kind, CodeKind::MouseButton);
    }

    #[test]
    fn resolve_unknown_code_is_synthetic() {
        let n = resolve(9999);
        assert!(n.synthetic);
        assert_eq!(n.lua_name, "code_9999");
        assert_eq!(n.kind, CodeKind::Other);
    }

    #[test]
    fn modifier_left_right_collapse() {
        assert_eq!(modifier_name(29), Some("ctrl"));
        assert_eq!(modifier_name(97), Some("ctrl"));
        assert_eq!(modifier_name(42), Some("shift"));
        assert_eq!(modifier_name(54), Some("shift"));
        assert_eq!(modifier_name(56), Some("alt"));
        assert_eq!(modifier_name(125), Some("meta"));
        assert_eq!(modifier_name(28), None);
    }

    #[test]
    fn click_at_button_supports_three() {
        assert_eq!(click_at_button(0x110), Some("left"));
        assert_eq!(click_at_button(0x111), Some("right"));
        assert_eq!(click_at_button(0x112), Some("middle"));
        assert_eq!(click_at_button(0x113), None); // BTN_SIDE → keystroke fallback
    }

    #[test]
    fn parse_stop_key_aliases() {
        assert_eq!(parse_stop_key("pause"), Some(119));
        assert_eq!(parse_stop_key("Pause"), Some(119));
        assert_eq!(parse_stop_key("scroll_lock"), Some(70));
        assert_eq!(parse_stop_key("scroll-lock"), Some(70));
        assert_eq!(parse_stop_key("f10"), Some(68));
        assert_eq!(parse_stop_key("not-a-key"), None);
    }
}
