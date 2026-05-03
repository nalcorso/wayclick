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
