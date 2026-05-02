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

/// Detects and tracks the currently focused window.
///
/// Call [`FocusTracker::start`] to auto-detect the backend and begin monitoring.
/// The tracker owns a background thread that reconnects automatically if the
/// compositor socket disconnects.
pub struct FocusTracker {
    current: Arc<Mutex<Option<WindowInfo>>>,
    stop_flag: Arc<AtomicBool>,
}

impl FocusTracker {
    /// Auto-detects the available backend and starts background monitoring.
    ///
    /// The returned `Arc<FocusTracker>` can be queried with [`get_current`] and shut down
    /// with [`stop`]. Call `stop()` before the process exits to join the background thread.
    pub fn start(event_bus: Arc<EventBus>, logger: Arc<Logger>) -> Arc<Self> {
        let current: Arc<Mutex<Option<WindowInfo>>> = Arc::new(Mutex::new(None));
        let stop_flag = Arc::new(AtomicBool::new(false));

        if hyprland::is_available() {
            logger.info("Focus tracking: Hyprland backend");
            hyprland::start(current.clone(), event_bus, logger, stop_flag.clone());
        } else if sway::is_available() {
            logger.info("Focus tracking: Sway backend");
            sway::start(current.clone(), event_bus, logger, stop_flag.clone());
        } else {
            logger.info("Focus tracking: no compatible backend detected (focus will be null)");
        }

        Arc::new(FocusTracker { current, stop_flag })
    }

    /// Returns the currently focused window, or `None` if unknown.
    pub fn get_current(&self) -> Option<WindowInfo> {
        self.current
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
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
