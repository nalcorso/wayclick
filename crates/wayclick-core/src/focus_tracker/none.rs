// SPDX-License-Identifier: MIT
//! No-op fallback focus backend.

use crate::event_bus::EventBus;
use crate::focus_tracker::WindowInfo;
use crate::logger::Logger;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

/// `start` is never called — the None path simply logs and skips spawning a thread.
/// This module exists only so the auto-detection code in `mod.rs` is self-contained.
#[allow(dead_code)]
pub fn is_available() -> bool {
    false
}

#[allow(dead_code)]
pub fn start(
    _current: Arc<Mutex<Option<WindowInfo>>>,
    _event_bus: Arc<EventBus>,
    _logger: Arc<Logger>,
    _stop: Arc<AtomicBool>,
) {
    // Nothing to do — auto-detection falls through to the None path in FocusTracker::start.
}
