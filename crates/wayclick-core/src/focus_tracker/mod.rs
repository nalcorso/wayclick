// SPDX-License-Identifier: MIT
//! Focused window detection for Wayclick.
//!
//! Auto-detects the available compositor backend (Hyprland, Sway, or None) and tracks
//! which process/window currently has input focus. Focus changes are published to the
//! event bus and the current focused window is queryable via IPC (`get_focus` method).
//!
//! # Backend priority
//! 1. Hyprland (`$HYPRLAND_INSTANCE_SIGNATURE`)
//! 2. Sway (`$SWAYSOCK`)
//! 3. None (always available — returns `null`)
//!
//! # Phase 2 (not yet implemented)
//! - `wlr-foreign-toplevel` (wlroots compositors via Wayland protocol)
//! - `x11rb` (X11 sessions)
//! - GNOME/KDE via DBus

pub(super) mod hyprland;
pub(super) mod none;
pub(super) mod sway;

use crate::event_bus::EventBus;
use crate::logger::Logger;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Information about the currently focused window.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WindowInfo {
    /// Stable app identifier: Wayland `app_id`, X11 `WM_CLASS` instance, or `"gamescope"`.
    ///
    /// For XWayland windows under Hyprland, this is the X11 WM_CLASS (more reliable than
    /// the XWayland app_id which may be an address string).
    ///
    /// Note on Gamescope: Hyprland surfaces Gamescope as `app_id: "gamescope"` with the
    /// active game reflected in `title`. Deep Gamescope introspection (nested X11) is out
    /// of scope for the current implementation.
    pub app_id: String,

    /// Current window title. Changes frequently (browser tabs, terminals).
    /// Prefer `app_id` for trigger conditions.
    pub title: String,

    /// Process ID, if the backend provides it.
    pub pid: Option<u32>,

    /// Process name read from `/proc/<pid>/comm` when `pid` is known.
    pub process_name: Option<String>,

    /// Which backend detected this focus: `"hyprland"`, `"sway"`, `"x11"`, `"wlr"`, `"none"`.
    pub backend: String,

    /// Additional class identifier (X11 WM_CLASS class, or mirrors `app_id` on Wayland).
    pub class: Option<String>,

    /// Whether this is an XWayland window (Hyprland provides this directly).
    #[serde(default)]
    pub xwayland: bool,
}

/// Cursor position reported by the compositor backend.
///
/// Coordinates are in logical screen pixels. Returned by
/// [`FocusTracker::cursor_position`] when the active backend supports it
/// (currently Hyprland only); other backends return `None`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct CursorPosition {
    pub x: i32,
    pub y: i32,
}

/// Information about a single compositor output (monitor).
///
/// Coordinates are in **logical compositor pixels**: `(x, y)` is the
/// top-left of the monitor in the global compositor layout, and
/// `(logical_width, logical_height)` is the size of the monitor in logical
/// pixels (already adjusted for `scale` and `transform`).
///
/// The bounding box `[x, x + logical_width) × [y, y + logical_height)` is
/// what the `zwlr_virtual_pointer_v1` `motion_absolute` request maps to.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MonitorInfo {
    pub name: String,
    pub description: String,
    pub x: i32,
    pub y: i32,
    pub logical_width: i32,
    pub logical_height: i32,
    pub scale: f64,
    /// `wl_output` transform enum (0..7).
    pub transform: i32,
}

impl MonitorInfo {
    /// Returns `true` when the given global compositor pixel lies inside
    /// this monitor's logical rectangle.
    pub fn contains(&self, gx: i32, gy: i32) -> bool {
        gx >= self.x
            && gx < self.x.saturating_add(self.logical_width)
            && gy >= self.y
            && gy < self.y.saturating_add(self.logical_height)
    }
}

/// Detects and tracks the currently focused window.
///
/// Call [`FocusTracker::start`] to auto-detect the backend and begin monitoring.
/// The tracker owns a background thread that reconnects automatically if the
/// compositor socket disconnects.
pub struct FocusTracker {
    current: Arc<Mutex<Option<WindowInfo>>>,
    stop_flag: Arc<AtomicBool>,
    /// Backend-specific cursor query function. `None` when no backend supports
    /// cursor reporting. The function returns `Ok(Some(pos))` on success,
    /// `Ok(None)` when the position is momentarily unavailable, and `Err(())`
    /// on a transient query failure (caller should treat both as "unavailable").
    cursor_query: Option<Arc<dyn Fn() -> Option<CursorPosition> + Send + Sync>>,
    /// Backend-specific monitor layout query. `None` when no backend
    /// supports monitor reporting (currently Hyprland only).
    monitors_query: Option<Arc<dyn Fn() -> Option<Vec<MonitorInfo>> + Send + Sync>>,
}

impl FocusTracker {
    /// Auto-detects the available backend and starts background monitoring.
    ///
    /// The returned `Arc<FocusTracker>` can be queried with [`get_current`] and shut down
    /// with [`stop`]. Call `stop()` before the process exits to join the background thread.
    pub fn start(event_bus: Arc<EventBus>, logger: Arc<Logger>) -> Arc<Self> {
        let current: Arc<Mutex<Option<WindowInfo>>> = Arc::new(Mutex::new(None));
        let stop_flag = Arc::new(AtomicBool::new(false));
        let mut cursor_query: Option<Arc<dyn Fn() -> Option<CursorPosition> + Send + Sync>> = None;
        let mut monitors_query: Option<Arc<dyn Fn() -> Option<Vec<MonitorInfo>> + Send + Sync>> =
            None;

        if hyprland::is_available() {
            logger.info("Focus tracking: Hyprland backend");
            hyprland::start(current.clone(), event_bus, logger, stop_flag.clone());
            cursor_query = hyprland::make_cursor_query();
            monitors_query = hyprland::make_monitors_query();
        } else if sway::is_available() {
            logger.info("Focus tracking: Sway backend");
            sway::start(current.clone(), event_bus, logger, stop_flag.clone());
        } else {
            logger.info("Focus tracking: no compatible backend detected (focus will be null)");
        }

        Arc::new(FocusTracker {
            current,
            stop_flag,
            cursor_query,
            monitors_query,
        })
    }

    /// Returns the currently focused window, or `None` if unknown.
    pub fn get_current(&self) -> Option<WindowInfo> {
        self.current
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Query the compositor for the current cursor position.
    ///
    /// Returns `None` when the active backend does not support cursor
    /// reporting (Sway, `none`) or when the query failed transiently
    /// (compositor socket unreachable, etc.). Callers must treat `None`
    /// as a graceful "unavailable" — no error path needed.
    pub fn cursor_position(&self) -> Option<CursorPosition> {
        self.cursor_query.as_ref().and_then(|q| q())
    }

    /// Returns `true` when the active backend supports cursor position
    /// queries. Useful for one-shot capability checks (e.g. to print a
    /// startup warning) so callers don't have to call `cursor_position`
    /// just to probe support.
    pub fn supports_cursor_position(&self) -> bool {
        self.cursor_query.is_some()
    }

    /// Query the compositor for the current monitor layout.
    ///
    /// Returns `None` when the active backend does not support monitor
    /// reporting (Sway, `none`) or when the query failed transiently.
    pub fn monitors(&self) -> Option<Vec<MonitorInfo>> {
        self.monitors_query.as_ref().and_then(|q| q())
    }

    /// Returns the monitor matching `name_or_description` if any. Names are
    /// matched exactly (case-insensitive); descriptions are matched only if
    /// no name matches. Ambiguous description matches return `None`.
    pub fn monitor_info(&self, name_or_description: &str) -> Option<MonitorInfo> {
        let mons = self.monitors()?;
        let needle = name_or_description.trim();
        if needle.is_empty() {
            return None;
        }
        let needle_lc = needle.to_ascii_lowercase();
        if let Some(m) = mons.iter().find(|m| m.name.eq_ignore_ascii_case(needle)) {
            return Some(m.clone());
        }
        let matches: Vec<&MonitorInfo> = mons
            .iter()
            .filter(|m| m.description.to_ascii_lowercase().contains(&needle_lc))
            .collect();
        if matches.len() == 1 {
            return Some(matches[0].clone());
        }
        None
    }

    /// Returns `true` when the active backend supports monitor queries.
    pub fn supports_monitors(&self) -> bool {
        self.monitors_query.is_some()
    }

    /// Signals the background monitoring thread to stop.
    pub fn stop(&self) {
        self.stop_flag.store(true, Ordering::Relaxed);
    }
}

/// Reads the process name from `/proc/<pid>/comm`.
pub(super) fn process_name_for_pid(pid: u32) -> Option<String> {
    std::fs::read_to_string(format!("/proc/{pid}/comm"))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}
