// SPDX-License-Identifier: MIT
//! Typed views over JSON-RPC responses returned by the wayclick daemon.
//!
//! These types are loose Serde wrappers — fields use `#[serde(default)]`
//! and missing fields default rather than failing deserialization, so that
//! older or newer daemon versions remain partially compatible with this
//! client. Tighten this later if a stricter contract is wanted.

use serde::{Deserialize, Serialize};

/// Service-wide status returned by the `status` / `status_json` methods.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ServiceStatus {
    /// Whether the daemon is currently active (running triggers).
    pub enabled: bool,
    /// Total number of triggers configured.
    pub trigger_count: usize,
    /// IDs of currently-active (held / latched) triggers.
    pub active_triggers: Vec<String>,
    /// Active layer name (e.g. `"default"`).
    pub layer: String,
    /// Daemon uptime in seconds.
    pub uptime_secs: u64,
    /// Whether dry-run mode is enabled (input is read but not synthesized).
    pub dry_run: bool,
}

/// A single trigger as reported by `list_triggers`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TriggerInfo {
    /// Stable identifier used to address the trigger over IPC.
    pub id: String,
    /// Human-friendly name from config.
    pub name: String,
    /// Trigger mode (e.g. `"oneshot"`, `"toggle"`, `"hold"`).
    pub mode: String,
    /// Whether the trigger is currently active.
    pub active: bool,
    /// Lifetime activation count.
    pub activate_count: u64,
    /// Whether the user has enabled this trigger (separate from runtime activation).
    pub user_enabled: bool,
    /// Whether the trigger was created at runtime via IPC (vs. from config).
    pub dynamic: bool,
}

/// Currently-focused window.
///
/// This is the inner shape — the daemon's `get_focus` method returns
/// `{"window": <FocusedWindow | null>}`, and `focus_changed` events carry
/// the same shape under `params.window`. Callers must extract that field
/// before deserializing.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FocusedWindow {
    pub app_id: String,
    pub title: String,
    pub process_name: Option<String>,
    /// Backend that produced this focus information (e.g. `"hyprland"`).
    pub backend: String,
    /// True when the window is an XWayland surface.
    pub xwayland: bool,
}

/// Cursor position returned by the daemon's `get_cursor_position` IPC method.
///
/// Coordinates are in logical screen pixels. Only available when the daemon's
/// focus-tracker backend supports cursor reporting (Hyprland today; Sway/none
/// surface a `-32001 Unsupported` JSON-RPC error instead).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct CursorPosition {
    pub x: i32,
    pub y: i32,
}

/// One monitor as reported by the daemon's `get_monitors` IPC method.
///
/// Coordinates are in the compositor's logical pixel layout (post-scale,
/// post-transform). `(x, y)` is the top-left corner; `(width, height)` is
/// the on-screen size. Only available when the focus-tracker backend
/// supports monitor reporting (Hyprland today).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MonitorInfo {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    #[serde(default = "default_scale")]
    pub scale: f64,
    #[serde(default)]
    pub transform: i32,
}

fn default_scale() -> f64 {
    1.0
}

impl MonitorInfo {
    /// `true` when `(gx, gy)` (compositor-global logical pixels) falls
    /// inside this monitor's rectangle.
    pub fn contains(&self, gx: i32, gy: i32) -> bool {
        gx >= self.x
            && gy >= self.y
            && gx < self.x.saturating_add(self.width)
            && gy < self.y.saturating_add(self.height)
    }
}
