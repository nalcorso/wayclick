// SPDX-License-Identifier: MIT
//! Pointer-only backend abstraction.
//!
//! The legacy [`InputBackend`](crate::input_backend::InputBackend) trait
//! advertises pointer ops (`move_absolute`, `click`, `scroll`, …) alongside
//! keyboard ops. On a multi-monitor Wayland setup the uinput-tablet pointer
//! has fundamental coordinate-mapping limitations — a single absolute-axis
//! device cannot address a non-rectangular union of monitors and the
//! compositor cannot route its events to the correct output, so clicks land
//! on the wrong monitor on layouts wider than one screen. The
//! `zwlr_virtual_pointer_v1` protocol fixes this by talking to the
//! compositor directly in global compositor pixels. We route pointer ops
//! through a dedicated `PointerBackend` that can be replaced at runtime by
//! the Wayland implementation while keyboards stay on uinput.
//!
//! Keyboards stay on uinput unconditionally.
//!
//! ## Integration shape
//!
//! [`RoutedBackend`] implements [`InputBackend`] but delegates pointer ops
//! to a [`PointerBackend`] and keyboard ops to the original [`InputBackend`].
//! This lets the engine continue to take a single `Arc<dyn InputBackend>`
//! while the daemon wires in a `zwlr_virtual_pointer_v1` pointer at startup
//! (with the existing uinput backend as the keyboard half + as the pointer
//! fallback if the Wayland protocol is unavailable).

use crate::config::{MouseButton, ScrollDirection};
use crate::input_backend::{BackendError, InputBackend};
use std::sync::Arc;

/// Trait covering only the pointer subset of input.
///
/// Implementations must be thread-safe and may serialise calls internally.
pub trait PointerBackend: Send + Sync {
    fn click(&self, button: MouseButton) -> Result<(), BackendError>;
    fn mouse_press(&self, button: MouseButton) -> Result<(), BackendError>;
    fn mouse_release(&self, button: MouseButton) -> Result<(), BackendError>;
    fn scroll(&self, direction: ScrollDirection, amount: i32) -> Result<(), BackendError>;
    fn move_relative(&self, dx: i32, dy: i32) -> Result<(), BackendError>;
    /// Move the pointer to the given coordinates **in global compositor pixels**.
    fn move_absolute(&self, x: i32, y: i32) -> Result<(), BackendError>;
    /// Implementation name for diagnostics (`"uinput"`, `"wlr-virtual-pointer"`, …).
    fn name(&self) -> &str;
}

/// Adapter that delegates pointer ops to an existing [`InputBackend`].
///
/// Used as the fallback when `zwlr_virtual_pointer_v1` is not available.
pub struct InputBackendPointerAdapter {
    inner: Arc<dyn InputBackend>,
}

impl InputBackendPointerAdapter {
    pub fn new(inner: Arc<dyn InputBackend>) -> Self {
        Self { inner }
    }
}

impl PointerBackend for InputBackendPointerAdapter {
    fn click(&self, button: MouseButton) -> Result<(), BackendError> {
        self.inner.click(button)
    }
    fn mouse_press(&self, button: MouseButton) -> Result<(), BackendError> {
        self.inner.mouse_press(button)
    }
    fn mouse_release(&self, button: MouseButton) -> Result<(), BackendError> {
        self.inner.mouse_release(button)
    }
    fn scroll(&self, direction: ScrollDirection, amount: i32) -> Result<(), BackendError> {
        self.inner.scroll(direction, amount)
    }
    fn move_relative(&self, dx: i32, dy: i32) -> Result<(), BackendError> {
        self.inner.move_relative(dx, dy)
    }
    fn move_absolute(&self, x: i32, y: i32) -> Result<(), BackendError> {
        self.inner.move_absolute(x, y)
    }
    fn name(&self) -> &str {
        // Tag the adapter so logs distinguish it from a "pure" pointer backend.
        "uinput-adapter"
    }
}

/// Composite `InputBackend` that routes pointer ops to a [`PointerBackend`]
/// and keyboard ops + frame forwarding to a wrapped [`InputBackend`].
pub struct RoutedBackend {
    keyboard: Arc<dyn InputBackend>,
    pointer: Arc<dyn PointerBackend>,
    name: String,
}

impl RoutedBackend {
    pub fn new(keyboard: Arc<dyn InputBackend>, pointer: Arc<dyn PointerBackend>) -> Self {
        let name = format!(
            "routed(keyboard={},pointer={})",
            keyboard.name(),
            pointer.name()
        );
        Self {
            keyboard,
            pointer,
            name,
        }
    }

    pub fn pointer_name(&self) -> &str {
        self.pointer.name()
    }

    pub fn keyboard_name(&self) -> &str {
        self.keyboard.name()
    }
}

impl InputBackend for RoutedBackend {
    fn init(&mut self) -> Result<(), BackendError> {
        // Both halves are expected to be initialised by their owner before
        // being wrapped — but allow re-init for the keyboard via interior
        // mutability if the impl supports it. The trait method takes `&mut
        // self` but we only hold Arcs; treat as no-op.
        Ok(())
    }

    fn click(&self, button: MouseButton) -> Result<(), BackendError> {
        self.pointer.click(button)
    }
    fn mouse_press(&self, button: MouseButton) -> Result<(), BackendError> {
        self.pointer.mouse_press(button)
    }
    fn mouse_release(&self, button: MouseButton) -> Result<(), BackendError> {
        self.pointer.mouse_release(button)
    }
    fn scroll(&self, direction: ScrollDirection, amount: i32) -> Result<(), BackendError> {
        self.pointer.scroll(direction, amount)
    }
    fn move_relative(&self, dx: i32, dy: i32) -> Result<(), BackendError> {
        self.pointer.move_relative(dx, dy)
    }
    fn move_absolute(&self, x: i32, y: i32) -> Result<(), BackendError> {
        self.pointer.move_absolute(x, y)
    }

    fn key_press(&self, key_code: u32) -> Result<(), BackendError> {
        self.keyboard.key_press(key_code)
    }
    fn key_release(&self, key_code: u32) -> Result<(), BackendError> {
        self.keyboard.key_release(key_code)
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn forward_frame(&self, events: &[(u16, u16, i32)]) -> Result<(), BackendError> {
        // Frame forwarding is uinput-specific (re-emitting evdev events from
        // exclusive-grab devices). Always send it through the keyboard
        // backend, which is the uinput device.
        self.keyboard.forward_frame(events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input_backend::{BackendCall, MockBackend};

    #[test]
    fn adapter_delegates_to_inner() {
        let mock = Arc::new(MockBackend::new());
        let calls = mock.calls_clone();
        let adapter = InputBackendPointerAdapter::new(mock.clone() as Arc<dyn InputBackend>);
        adapter.click(MouseButton::Left).unwrap();
        adapter.move_absolute(100, 200).unwrap();
        adapter
            .scroll(crate::config::ScrollDirection::Down, 3)
            .unwrap();
        let recorded = calls.lock().unwrap().clone();
        assert!(recorded.contains(&BackendCall::Click(MouseButton::Left)));
        assert!(recorded.contains(&BackendCall::MoveAbsolute(100, 200)));
        assert!(recorded.contains(&BackendCall::Scroll(
            crate::config::ScrollDirection::Down,
            3
        )));
    }

    #[test]
    fn adapter_name_is_distinct() {
        let mock = Arc::new(MockBackend::new()) as Arc<dyn InputBackend>;
        let adapter = InputBackendPointerAdapter::new(mock);
        assert_eq!(adapter.name(), "uinput-adapter");
    }

    #[test]
    fn routed_backend_splits_pointer_and_keyboard() {
        let keyboard_mock = Arc::new(MockBackend::new());
        let pointer_mock = Arc::new(MockBackend::new());
        let kb_calls = keyboard_mock.calls_clone();
        let ptr_calls = pointer_mock.calls_clone();

        let pointer_adapter = Arc::new(InputBackendPointerAdapter::new(
            pointer_mock.clone() as Arc<dyn InputBackend>
        )) as Arc<dyn PointerBackend>;
        let routed = RoutedBackend::new(
            keyboard_mock.clone() as Arc<dyn InputBackend>,
            pointer_adapter,
        );

        routed.key_press(57).unwrap();
        routed.key_release(57).unwrap();
        routed.click(MouseButton::Left).unwrap();
        routed.move_absolute(4103, 1370).unwrap();
        routed
            .scroll(crate::config::ScrollDirection::Down, 1)
            .unwrap();

        let kb = kb_calls.lock().unwrap().clone();
        let ptr = ptr_calls.lock().unwrap().clone();
        assert_eq!(
            kb,
            vec![BackendCall::KeyPress(57), BackendCall::KeyRelease(57)]
        );
        assert_eq!(
            ptr,
            vec![
                BackendCall::Click(MouseButton::Left),
                BackendCall::MoveAbsolute(4103, 1370),
                BackendCall::Scroll(crate::config::ScrollDirection::Down, 1),
            ]
        );
    }

    #[test]
    fn routed_backend_name_includes_both_halves() {
        let keyboard = Arc::new(MockBackend::new()) as Arc<dyn InputBackend>;
        let pointer = Arc::new(InputBackendPointerAdapter::new(
            Arc::new(MockBackend::new()) as Arc<dyn InputBackend>,
        )) as Arc<dyn PointerBackend>;
        let routed = RoutedBackend::new(keyboard, pointer);
        assert!(routed.name().contains("keyboard="));
        assert!(routed.name().contains("pointer="));
    }
}
