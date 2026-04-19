use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("Lua error: {0}")]
    Lua(String),
    #[error("Validation error: {0}")]
    Validation(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Duplicate trigger ID: {0}")]
    DuplicateTrigger(String),
    #[error("Unknown trigger reference: {0}")]
    UnknownTriggerRef(String),
    #[error("Invalid key name: {0}")]
    InvalidKey(String),
    #[error("Missing required field: {0}")]
    MissingField(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalOptions {
    pub dry_run: bool,
    pub socket_path: Option<String>,
    pub log_capacity: usize,
    pub min_interval_ms: u32,
}

impl Default for GlobalOptions {
    fn default() -> Self {
        Self {
            dry_run: true,
            socket_path: None,
            log_capacity: 512,
            min_interval_ms: 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TriggerMode {
    Toggle,
    Hold,
    OneShot,
}

impl TriggerMode {
    pub fn from_str_mode(s: &str) -> Result<Self, ConfigError> {
        match s.to_lowercase().as_str() {
            "toggle" => Ok(TriggerMode::Toggle),
            "hold" => Ok(TriggerMode::Hold),
            "oneshot" => Ok(TriggerMode::OneShot),
            _ => Err(ConfigError::Validation(format!(
                "Unknown trigger mode: '{}'. Expected 'toggle', 'hold', or 'oneshot'",
                s
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    Button4,
    Button5,
}

impl MouseButton {
    pub fn from_str_name(s: &str) -> Result<Self, ConfigError> {
        match s.to_lowercase().as_str() {
            "left" => Ok(MouseButton::Left),
            "right" => Ok(MouseButton::Right),
            "middle" => Ok(MouseButton::Middle),
            "button4" => Ok(MouseButton::Button4),
            "button5" => Ok(MouseButton::Button5),
            _ => Err(ConfigError::Validation(format!(
                "Unknown mouse button: '{}'. Expected 'left', 'right', 'middle', 'button4', or 'button5'",
                s
            ))),
        }
    }

    /// Returns the Linux input event code for this button.
    pub fn event_code(&self) -> u16 {
        match self {
            MouseButton::Left => 0x110,    // BTN_LEFT
            MouseButton::Right => 0x111,   // BTN_RIGHT
            MouseButton::Middle => 0x112,  // BTN_MIDDLE
            MouseButton::Button4 => 0x113, // BTN_SIDE
            MouseButton::Button5 => 0x114, // BTN_EXTRA
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScrollDirection {
    Up,
    Down,
    Left,
    Right,
}

impl ScrollDirection {
    pub fn from_str_name(s: &str) -> Result<Self, ConfigError> {
        match s.to_lowercase().as_str() {
            "up" => Ok(ScrollDirection::Up),
            "down" => Ok(ScrollDirection::Down),
            "left" => Ok(ScrollDirection::Left),
            "right" => Ok(ScrollDirection::Right),
            _ => Err(ConfigError::Validation(format!(
                "Unknown scroll direction: '{}'",
                s
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CompositeMode {
    Parallel,
    Sequence,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ActionConfig {
    AutoClick {
        button: MouseButton,
        interval_ms: u32,
        duration_ms: Option<u32>,
        jitter_ms: u32,
        hold_ms: u32,
    },
    KeyPress {
        key_name: String,
        key_code: u32,
        interval_ms: u32,
        duration_ms: Option<u32>,
        jitter_ms: u32,
    },
    ScrollWheel {
        direction: ScrollDirection,
        amount: i32,
        interval_ms: u32,
        duration_ms: Option<u32>,
        jitter_ms: u32,
    },
    MouseMove {
        dx: i32,
        dy: i32,
        interval_ms: u32,
        duration_ms: Option<u32>,
        jitter_ms: u32,
    },
    MouseMoveAbsolute {
        x: i32,
        y: i32,
    },
    ClickAt {
        x: i32,
        y: i32,
        button: MouseButton,
        hold_ms: u32,
    },
    Drag {
        from_x: i32,
        from_y: i32,
        to_x: i32,
        to_y: i32,
        button: MouseButton,
        duration_ms: u32,
    },
    SetLayer {
        layer: String,
    },
    Composite {
        mode: CompositeMode,
        actions: Vec<ActionConfig>,
    },
    Delay {
        duration_ms: u32,
    },
    NoOp,
}

impl ActionConfig {
    pub fn type_name(&self) -> &str {
        match self {
            ActionConfig::AutoClick { .. } => "auto_click",
            ActionConfig::KeyPress { .. } => "key_press",
            ActionConfig::ScrollWheel { .. } => "scroll",
            ActionConfig::MouseMove { .. } => "mouse_move",
            ActionConfig::MouseMoveAbsolute { .. } => "mouse_move_abs",
            ActionConfig::ClickAt { .. } => "click_at",
            ActionConfig::Drag { .. } => "drag",
            ActionConfig::SetLayer { .. } => "set_layer",
            ActionConfig::Composite { mode, .. } => match mode {
                CompositeMode::Parallel => "parallel",
                CompositeMode::Sequence => "sequence",
            },
            ActionConfig::Delay { .. } => "delay",
            ActionConfig::NoOp => "noop",
        }
    }

    /// Returns true if this action type should only be used in OneShot/Sequence contexts,
    /// not as the root action of a Toggle/Hold trigger.
    pub fn is_oneshot_only(&self) -> bool {
        matches!(
            self,
            ActionConfig::SetLayer { .. }
                | ActionConfig::ClickAt { .. }
                | ActionConfig::Drag { .. }
                | ActionConfig::MouseMoveAbsolute { .. }
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerBinding {
    pub id: String,
    pub name: String,
    pub description: String,
    pub mode: TriggerMode,
    pub action: ActionConfig,
    pub cooldown_ms: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DeviceMatch {
    ByPath { path: String },
    ByName { contains: String },
    ByVidPid { vendor: u16, product: u16 },
    ByPhys { contains: String },
    Any { matchers: Vec<DeviceMatch> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ButtonBinding {
    /// Resolved evdev event codes (length > 1 = chord)
    pub codes: Vec<u16>,
    /// Original code names for display/serialization (e.g., ["BTN_SIDE", "BTN_EXTRA"])
    pub code_names: Vec<String>,
    pub trigger_id: String,
    /// Optional trigger fired on long-press (hold_threshold_ms must also be set)
    pub hold_trigger_id: Option<String>,
    /// Hold duration threshold in ms (tap fires trigger_id, hold fires hold_trigger_id)
    pub hold_threshold_ms: Option<u32>,
    /// Layer filter — None means active in all layers
    pub layer: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceBinding {
    pub device_match: DeviceMatch,
    pub button_bindings: Vec<ButtonBinding>,
    pub exclusive: bool,
}

/// Rule for automatic profile/layer switching based on active window.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileRule {
    pub name: String,
    /// Regex pattern matched against window app_id/class
    pub match_app: Option<String>,
    /// Regex pattern matched against window title
    pub match_title: Option<String>,
    /// Layer to switch to when this profile matches
    pub layer: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    pub options: GlobalOptions,
    pub triggers: Vec<TriggerBinding>,
    pub device_bindings: Vec<DeviceBinding>,
    pub profile_rules: Vec<ProfileRule>,
}

// Config derives Default: all fields have sensible defaults (empty vecs, GlobalOptions::default()).

/// Normalize a key name to a Linux input event code.
/// Supports: "space" -> KEY_SPACE, "a" -> KEY_A, "KEY_SPACE" -> KEY_SPACE, "F1" -> KEY_F1
pub fn normalize_key_name(raw: &str) -> Result<(String, u32), ConfigError> {
    let upper = raw.to_uppercase();
    let key_name = if upper.starts_with("KEY_") {
        upper.clone()
    } else {
        format!("KEY_{}", upper)
    };

    let code = key_name_to_code(&key_name).ok_or_else(|| {
        ConfigError::InvalidKey(format!(
            "Unknown key: '{}' (resolved to '{}')",
            raw, key_name
        ))
    })?;

    Ok((key_name, code))
}

/// Maps KEY_* names to Linux input event codes.
pub fn key_name_to_code(name: &str) -> Option<u32> {
    Some(match name {
        "KEY_ESC" => 1,
        "KEY_1" => 2,
        "KEY_2" => 3,
        "KEY_3" => 4,
        "KEY_4" => 5,
        "KEY_5" => 6,
        "KEY_6" => 7,
        "KEY_7" => 8,
        "KEY_8" => 9,
        "KEY_9" => 10,
        "KEY_0" => 11,
        "KEY_MINUS" => 12,
        "KEY_EQUAL" => 13,
        "KEY_BACKSPACE" => 14,
        "KEY_TAB" => 15,
        "KEY_Q" => 16,
        "KEY_W" => 17,
        "KEY_E" => 18,
        "KEY_R" => 19,
        "KEY_T" => 20,
        "KEY_Y" => 21,
        "KEY_U" => 22,
        "KEY_I" => 23,
        "KEY_O" => 24,
        "KEY_P" => 25,
        "KEY_LEFTBRACE" => 26,
        "KEY_RIGHTBRACE" => 27,
        "KEY_ENTER" => 28,
        "KEY_LEFTCTRL" => 29,
        "KEY_A" => 30,
        "KEY_S" => 31,
        "KEY_D" => 32,
        "KEY_F" => 33,
        "KEY_G" => 34,
        "KEY_H" => 35,
        "KEY_J" => 36,
        "KEY_K" => 37,
        "KEY_L" => 38,
        "KEY_SEMICOLON" => 39,
        "KEY_APOSTROPHE" => 40,
        "KEY_GRAVE" => 41,
        "KEY_LEFTSHIFT" => 42,
        "KEY_BACKSLASH" => 43,
        "KEY_Z" => 44,
        "KEY_X" => 45,
        "KEY_C" => 46,
        "KEY_V" => 47,
        "KEY_B" => 48,
        "KEY_N" => 49,
        "KEY_M" => 50,
        "KEY_COMMA" => 51,
        "KEY_DOT" => 52,
        "KEY_SLASH" => 53,
        "KEY_RIGHTSHIFT" => 54,
        "KEY_LEFTALT" => 56,
        "KEY_SPACE" => 57,
        "KEY_CAPSLOCK" => 58,
        "KEY_F1" => 59,
        "KEY_F2" => 60,
        "KEY_F3" => 61,
        "KEY_F4" => 62,
        "KEY_F5" => 63,
        "KEY_F6" => 64,
        "KEY_F7" => 65,
        "KEY_F8" => 66,
        "KEY_F9" => 67,
        "KEY_F10" => 68,
        "KEY_F11" => 87,
        "KEY_F12" => 88,
        "KEY_RIGHTCTRL" => 97,
        "KEY_RIGHTALT" => 100,
        "KEY_HOME" => 102,
        "KEY_UP" => 103,
        "KEY_PAGEUP" => 104,
        "KEY_LEFT" => 105,
        "KEY_RIGHT" => 106,
        "KEY_END" => 107,
        "KEY_DOWN" => 108,
        "KEY_PAGEDOWN" => 109,
        "KEY_INSERT" => 110,
        "KEY_DELETE" => 111,
        // Media keys
        "KEY_MUTE" => 113,
        "KEY_VOLUMEDOWN" => 114,
        "KEY_VOLUMEUP" => 115,
        "KEY_NEXTSONG" => 163,
        "KEY_PLAYPAUSE" => 164,
        "KEY_PREVIOUSSONG" => 165,
        "KEY_STOPCD" => 166,
        "KEY_RECORD" => 167,
        "KEY_REWIND" => 168,
        "KEY_FASTFORWARD" => 208,
        // Convenience aliases (without underscores)
        "KEY_PLAY_PAUSE" => 164,
        "KEY_NEXT_SONG" => 163,
        "KEY_PREVIOUS_SONG" => 165,
        "KEY_STOP_CD" => 166,
        "KEY_VOLUME_UP" => 115,
        "KEY_VOLUME_DOWN" => 114,
        "KEY_FAST_FORWARD" => 208,
        // Screen brightness
        "KEY_BRIGHTNESSDOWN" => 224,
        "KEY_BRIGHTNESSUP" => 225,
        _ => return None,
    })
}

/// Map button code names like "BTN_LEFT", "BTN_SIDE" etc to Linux event codes.
pub fn button_code_from_name(name: &str) -> Option<u16> {
    // Delegate to unified resolver, but only accept BTN_* names
    let upper = name.to_uppercase();
    if upper.starts_with("BTN_") {
        trigger_code_from_name(name)
    } else {
        None
    }
}

/// Unified trigger code resolver: accepts both BTN_* and KEY_* names,
/// returning the Linux evdev event code as u16.
pub fn trigger_code_from_name(name: &str) -> Option<u16> {
    let upper = name.to_uppercase();
    Some(match upper.as_str() {
        // BTN_* codes
        "BTN_LEFT" => 0x110,
        "BTN_RIGHT" => 0x111,
        "BTN_MIDDLE" => 0x112,
        "BTN_SIDE" => 0x113,
        "BTN_EXTRA" => 0x114,
        "BTN_FORWARD" => 0x115,
        "BTN_BACK" => 0x116,
        "BTN_TASK" => 0x117,
        _ => {
            // Try KEY_* resolution
            if upper.starts_with("KEY_") {
                return key_name_to_code(&upper).map(|c| c as u16);
            }
            // Try bare name → KEY_ prefix
            let key_name = format!("KEY_{}", upper);
            return key_name_to_code(&key_name).map(|c| c as u16);
        }
    })
}

/// Validate a fully loaded config.
pub fn validate_config(config: &Config) -> Result<(), Vec<ConfigError>> {
    let mut errors = Vec::new();

    // Check for duplicate trigger IDs
    let mut seen_ids = std::collections::HashSet::new();
    for trigger in &config.triggers {
        if !seen_ids.insert(&trigger.id) {
            errors.push(ConfigError::DuplicateTrigger(trigger.id.clone()));
        }
    }

    // Check that all device binding trigger_ids reference existing triggers
    let trigger_ids: std::collections::HashSet<&str> =
        config.triggers.iter().map(|t| t.id.as_str()).collect();

    for binding in &config.device_bindings {
        for btn in &binding.button_bindings {
            if !trigger_ids.contains(btn.trigger_id.as_str()) {
                errors.push(ConfigError::UnknownTriggerRef(btn.trigger_id.clone()));
            }
            if let Some(ref hold_id) = btn.hold_trigger_id {
                if !trigger_ids.contains(hold_id.as_str()) {
                    errors.push(ConfigError::UnknownTriggerRef(hold_id.clone()));
                }
            }
        }
    }

    // Validate intervals
    fn validate_action_intervals(
        action: &ActionConfig,
        min_interval: u32,
        errors: &mut Vec<ConfigError>,
    ) {
        match action {
            ActionConfig::AutoClick { interval_ms, .. }
            | ActionConfig::KeyPress { interval_ms, .. }
            | ActionConfig::ScrollWheel { interval_ms, .. }
            | ActionConfig::MouseMove { interval_ms, .. } => {
                if *interval_ms < min_interval {
                    errors.push(ConfigError::Validation(format!(
                        "interval_ms {} is below minimum {}",
                        interval_ms, min_interval
                    )));
                }
            }
            ActionConfig::Composite { actions, .. } => {
                for a in actions {
                    validate_action_intervals(a, min_interval, errors);
                }
            }
            ActionConfig::Delay { .. }
            | ActionConfig::NoOp
            | ActionConfig::MouseMoveAbsolute { .. }
            | ActionConfig::ClickAt { .. }
            | ActionConfig::Drag { .. }
            | ActionConfig::SetLayer { .. } => {}
        }
    }

    for trigger in &config.triggers {
        validate_action_intervals(&trigger.action, config.options.min_interval_ms, &mut errors);
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// Resolve the default IPC socket path.
pub fn default_socket_path() -> PathBuf {
    if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
        PathBuf::from(runtime_dir).join("wayclick.sock")
    } else {
        PathBuf::from("/tmp").join(format!(
            "wayclick-{}.sock",
            // SAFETY: getuid() is always safe — it reads the real UID with no side effects.
            unsafe { libc::getuid() }
        ))
    }
}

/// Get the effective socket path from config or default.
pub fn effective_socket_path(config: &Config) -> PathBuf {
    match &config.options.socket_path {
        Some(p) if !p.is_empty() => PathBuf::from(p),
        _ => default_socket_path(),
    }
}

// Avoid direct libc dependency; use nix instead for getuid if needed,
// but for socket path we can use a simpler approach
mod libc {
    extern "C" {
        pub fn getuid() -> u32;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trigger_mode_from_str() {
        assert_eq!(
            TriggerMode::from_str_mode("toggle").unwrap(),
            TriggerMode::Toggle
        );
        assert_eq!(
            TriggerMode::from_str_mode("hold").unwrap(),
            TriggerMode::Hold
        );
        assert_eq!(
            TriggerMode::from_str_mode("oneshot").unwrap(),
            TriggerMode::OneShot
        );
        assert!(TriggerMode::from_str_mode("invalid").is_err());
    }

    #[test]
    fn test_mouse_button_from_str() {
        assert_eq!(
            MouseButton::from_str_name("left").unwrap(),
            MouseButton::Left
        );
        assert_eq!(
            MouseButton::from_str_name("RIGHT").unwrap(),
            MouseButton::Right
        );
        assert_eq!(
            MouseButton::from_str_name("middle").unwrap(),
            MouseButton::Middle
        );
        assert_eq!(
            MouseButton::from_str_name("button4").unwrap(),
            MouseButton::Button4
        );
        assert_eq!(
            MouseButton::from_str_name("button5").unwrap(),
            MouseButton::Button5
        );
        assert!(MouseButton::from_str_name("invalid").is_err());
    }

    #[test]
    fn test_scroll_direction_from_str() {
        assert_eq!(
            ScrollDirection::from_str_name("up").unwrap(),
            ScrollDirection::Up
        );
        assert_eq!(
            ScrollDirection::from_str_name("down").unwrap(),
            ScrollDirection::Down
        );
        assert_eq!(
            ScrollDirection::from_str_name("LEFT").unwrap(),
            ScrollDirection::Left
        );
        assert!(ScrollDirection::from_str_name("diagonal").is_err());
    }

    #[test]
    fn test_normalize_key_name() {
        let (name, code) = normalize_key_name("space").unwrap();
        assert_eq!(name, "KEY_SPACE");
        assert_eq!(code, 57);

        let (name, code) = normalize_key_name("a").unwrap();
        assert_eq!(name, "KEY_A");
        assert_eq!(code, 30);

        let (name, code) = normalize_key_name("KEY_SPACE").unwrap();
        assert_eq!(name, "KEY_SPACE");
        assert_eq!(code, 57);

        let (name, code) = normalize_key_name("F1").unwrap();
        assert_eq!(name, "KEY_F1");
        assert_eq!(code, 59);

        assert!(normalize_key_name("NOT_A_KEY_9999").is_err());
    }

    #[test]
    fn test_button_code_from_name() {
        assert_eq!(button_code_from_name("BTN_LEFT"), Some(0x110));
        assert_eq!(button_code_from_name("BTN_SIDE"), Some(0x113));
        assert_eq!(button_code_from_name("BTN_EXTRA"), Some(0x114));
        assert_eq!(button_code_from_name("INVALID"), None);
    }

    #[test]
    fn test_validate_config_ok() {
        let config = Config {
            options: GlobalOptions::default(),
            triggers: vec![TriggerBinding {
                id: "test".into(),
                name: "Test".into(),
                description: String::new(),
                mode: TriggerMode::Toggle,
                action: ActionConfig::AutoClick {
                    button: MouseButton::Left,
                    interval_ms: 50,
                    duration_ms: None,
                    jitter_ms: 0,
                    hold_ms: 0,
                },
                cooldown_ms: None,
            }],
            device_bindings: vec![],
            profile_rules: vec![],
        };
        assert!(validate_config(&config).is_ok());
    }

    #[test]
    fn test_validate_config_duplicate_trigger_id() {
        let trigger = TriggerBinding {
            id: "dup".into(),
            name: "Dup".into(),
            description: String::new(),
            mode: TriggerMode::Toggle,
            action: ActionConfig::NoOp,
            cooldown_ms: None,
        };
        let config = Config {
            options: GlobalOptions::default(),
            triggers: vec![trigger.clone(), trigger],
            device_bindings: vec![],
            profile_rules: vec![],
        };
        let errs = validate_config(&config).unwrap_err();
        assert!(errs
            .iter()
            .any(|e| matches!(e, ConfigError::DuplicateTrigger(_))));
    }

    #[test]
    fn test_validate_config_unknown_trigger_ref() {
        let config = Config {
            options: GlobalOptions::default(),
            triggers: vec![],
            device_bindings: vec![DeviceBinding {
                device_match: DeviceMatch::ByName {
                    contains: "test".into(),
                },
                button_bindings: vec![ButtonBinding {
                    codes: vec![0x110],
                    code_names: vec!["BTN_LEFT".into()],
                    trigger_id: "nonexistent".into(),
                    hold_trigger_id: None,
                    hold_threshold_ms: None,
                    layer: None,
                }],
                exclusive: false,
            }],
            profile_rules: vec![],
        };
        let errs = validate_config(&config).unwrap_err();
        assert!(errs
            .iter()
            .any(|e| matches!(e, ConfigError::UnknownTriggerRef(_))));
    }

    #[test]
    fn test_action_type_name() {
        assert_eq!(ActionConfig::NoOp.type_name(), "noop");
        assert_eq!(
            ActionConfig::Delay { duration_ms: 100 }.type_name(),
            "delay"
        );
        let ac = ActionConfig::AutoClick {
            button: MouseButton::Left,
            interval_ms: 50,
            duration_ms: None,
            jitter_ms: 0,
            hold_ms: 0,
        };
        assert_eq!(ac.type_name(), "auto_click");
    }

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert!(config.options.dry_run);
        assert_eq!(config.options.log_capacity, 512);
        assert_eq!(config.options.min_interval_ms, 1);
        assert!(config.triggers.is_empty());
        assert!(config.device_bindings.is_empty());
    }
}
