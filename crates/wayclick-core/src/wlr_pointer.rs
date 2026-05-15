// SPDX-License-Identifier: MIT
//! `zwlr_virtual_pointer_v1` PointerBackend.
//!
//! Hosts a dedicated Wayland-client thread that owns the connection and the
//! protocol object. Pointer ops are sent over a channel; the thread issues
//! the corresponding requests and flushes. Each high-level op terminates
//! with a `frame()` request, which the compositor treats as the boundary of
//! one logical event group (see wlr-virtual-pointer protocol).
//!
//! ## Coordinate model
//!
//! `motion_absolute` takes `(x, y, x_extent, y_extent)` and the compositor
//! scales `(x/x_extent, y/y_extent)` across its layout. We set the extents
//! to the bounding box of the current monitor layout (queried from the
//! daemon's `FocusTracker`) and pass `x`/`y` directly as global compositor
//! pixels, which is exactly what the recorder and the engine work in.

use crate::config::{MouseButton, ScrollDirection};
use crate::input_backend::BackendError;
use crate::logger::Logger;
use crate::mutex_ext::MutexExt;
use crate::pointer_backend::PointerBackend;

use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle};

use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1,
    zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
};

/// Snapshot of the compositor layout used to scale `motion_absolute`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DesktopExtents {
    pub width: u32,
    pub height: u32,
}

impl DesktopExtents {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width: width.max(1),
            height: height.max(1),
        }
    }
}

#[derive(Debug)]
enum PointerCmd {
    MoveAbsolute(i32, i32),
    MoveRelative(i32, i32),
    Button(u32, bool), // (button, pressed)
    Scroll(ScrollDirection, i32),
    /// Notifies the worker that `extents` was updated by the owner. The
    /// actual values are read from the shared `Arc<Mutex<DesktopExtents>>`
    /// before every `motion_absolute`, so this is purely a wake-up signal
    /// (the worker doesn't currently sleep between commands, but keeping
    /// the variant lets future changes add coalescing without an API break).
    UpdateExtents,
    Shutdown,
}

/// `PointerBackend` backed by `zwlr_virtual_pointer_v1`.
pub struct WlrVirtualPointer {
    cmd_tx: mpsc::Sender<PointerCmd>,
    thread: Mutex<Option<JoinHandle<()>>>,
    extents: Arc<Mutex<DesktopExtents>>,
    name: String,
}

impl WlrVirtualPointer {
    /// Try to construct a wlr-virtual-pointer backend by connecting to the
    /// Wayland display and binding the manager global. Returns an error if
    /// no Wayland display is reachable or the protocol is not advertised.
    pub fn try_new(
        logger: Arc<Logger>,
        initial_extents: DesktopExtents,
    ) -> Result<Self, BackendError> {
        // Connect on the calling thread first so we can fail fast with a
        // useful error rather than asynchronously inside the worker.
        let conn = Connection::connect_to_env()
            .map_err(|e| BackendError::Other(format!("wayland connect: {}", e)))?;

        let (globals, mut event_queue) = registry_queue_init::<RegistryProbe>(&conn)
            .map_err(|e| BackendError::Other(format!("wayland registry init: {}", e)))?;
        let qh = event_queue.handle();

        // Probe for the manager. This call also dispatches pending events.
        let mut probe = RegistryProbe;
        event_queue
            .roundtrip(&mut probe)
            .map_err(|e| BackendError::Other(format!("wayland roundtrip: {}", e)))?;

        let manager: ZwlrVirtualPointerManagerV1 = globals.bind(&qh, 1..=2, ()).map_err(|e| {
            BackendError::Other(format!(
                "zwlr_virtual_pointer_manager_v1 not advertised by compositor: {}",
                e
            ))
        })?;

        // Try to grab the first seat (optional — the manager will also accept
        // create_virtual_pointer with a null seat, but most compositors prefer
        // a real seat).
        let seat: Option<WlSeat> = globals.bind(&qh, 1..=9, ()).ok();

        let pointer: ZwlrVirtualPointerV1 = manager.create_virtual_pointer(seat.as_ref(), &qh, ());

        // Flush so the compositor sees the object before we send requests.
        event_queue
            .roundtrip(&mut probe)
            .map_err(|e| BackendError::Other(format!("wayland post-create roundtrip: {}", e)))?;

        let extents = Arc::new(Mutex::new(initial_extents));
        let (cmd_tx, cmd_rx) = mpsc::channel::<PointerCmd>();

        let logger_thread = logger.clone();
        let extents_thread = extents.clone();
        let thread = thread::Builder::new()
            .name("wayclick-wlr-pointer".into())
            .spawn(move || {
                run_pointer_thread(
                    conn,
                    event_queue,
                    pointer,
                    manager,
                    cmd_rx,
                    extents_thread,
                    logger_thread,
                );
            })
            .map_err(|e| BackendError::Other(format!("spawn wayland thread: {}", e)))?;

        logger.info(format!(
            "WlrVirtualPointer initialised (extents {}x{})",
            initial_extents.width, initial_extents.height
        ));

        Ok(Self {
            cmd_tx,
            thread: Mutex::new(Some(thread)),
            extents,
            name: "wlr-virtual-pointer".to_string(),
        })
    }

    /// Update the desktop bounding box used for `motion_absolute` scaling.
    /// Call when the monitor layout changes (hot-plug, resolution change).
    pub fn set_extents(&self, extents: DesktopExtents) {
        *self.extents.lock_or_recover() = extents;
        let _ = self.cmd_tx.send(PointerCmd::UpdateExtents);
    }

    fn send(&self, cmd: PointerCmd) -> Result<(), BackendError> {
        self.cmd_tx
            .send(cmd)
            .map_err(|_| BackendError::Other("wlr-virtual-pointer thread terminated".into()))
    }
}

impl Drop for WlrVirtualPointer {
    fn drop(&mut self) {
        let _ = self.cmd_tx.send(PointerCmd::Shutdown);
        // Bound the join so a wedged Wayland thread (e.g. compositor stopped
        // responding to flush) can't hang daemon shutdown. We poll the join
        // handle at small intervals up to `JOIN_TIMEOUT`, then give up and
        // leak the thread — losing the pointer device cleanup is preferable
        // to never exiting the process.
        const JOIN_TIMEOUT: Duration = Duration::from_millis(500);
        const POLL: Duration = Duration::from_millis(20);

        let handle = self.thread.lock_or_recover().take();
        if let Some(handle) = handle {
            let deadline = Instant::now() + JOIN_TIMEOUT;
            while !handle.is_finished() {
                if Instant::now() >= deadline {
                    // Detach: thread keeps running but daemon shutdown proceeds.
                    return;
                }
                thread::sleep(POLL);
            }
            let _ = handle.join();
        }
    }
}

impl PointerBackend for WlrVirtualPointer {
    fn click(&self, button: MouseButton) -> Result<(), BackendError> {
        let code = button.event_code() as u32;
        self.send(PointerCmd::Button(code, true))?;
        self.send(PointerCmd::Button(code, false))
    }

    fn mouse_press(&self, button: MouseButton) -> Result<(), BackendError> {
        self.send(PointerCmd::Button(button.event_code() as u32, true))
    }

    fn mouse_release(&self, button: MouseButton) -> Result<(), BackendError> {
        self.send(PointerCmd::Button(button.event_code() as u32, false))
    }

    fn scroll(&self, direction: ScrollDirection, amount: i32) -> Result<(), BackendError> {
        self.send(PointerCmd::Scroll(direction, amount))
    }

    fn move_relative(&self, dx: i32, dy: i32) -> Result<(), BackendError> {
        self.send(PointerCmd::MoveRelative(dx, dy))
    }

    fn move_absolute(&self, x: i32, y: i32) -> Result<(), BackendError> {
        self.send(PointerCmd::MoveAbsolute(x, y))
    }

    fn name(&self) -> &str {
        &self.name
    }
}

/// Worker-thread loop.
#[allow(clippy::too_many_arguments)]
fn run_pointer_thread(
    conn: Connection,
    mut event_queue: EventQueue<RegistryProbe>,
    pointer: ZwlrVirtualPointerV1,
    _manager: ZwlrVirtualPointerManagerV1,
    cmd_rx: mpsc::Receiver<PointerCmd>,
    extents: Arc<Mutex<DesktopExtents>>,
    logger: Arc<Logger>,
) {
    // We don't actually receive events from zwlr_virtual_pointer_v1, but we
    // still need to flush the connection and handle disconnects.
    let start = Instant::now();
    let mut probe = RegistryProbe;
    let time_ms = || -> u32 { start.elapsed().as_millis() as u32 };

    while let Ok(cmd) = cmd_rx.recv() {
        let result: Result<(), String> = match cmd {
            PointerCmd::Shutdown => {
                pointer.destroy();
                let _ = event_queue.flush();
                logger.info("WlrVirtualPointer thread shutting down");
                return;
            }
            PointerCmd::UpdateExtents => Ok(()), // already reflected via Arc<Mutex>
            PointerCmd::MoveAbsolute(x, y) => {
                let ext = *extents.lock_or_recover();
                // Clamp negatives to zero (e.g. clicks above the top of the union
                // rect are not representable in motion_absolute).
                let xu = x.max(0) as u32;
                let yu = y.max(0) as u32;
                pointer.motion_absolute(time_ms(), xu, yu, ext.width, ext.height);
                pointer.frame();
                event_queue.flush().map_err(|e| e.to_string())
            }
            PointerCmd::MoveRelative(dx, dy) => {
                pointer.motion(time_ms(), dx as f64, dy as f64);
                pointer.frame();
                event_queue.flush().map_err(|e| e.to_string())
            }
            PointerCmd::Button(code, pressed) => {
                let state = if pressed {
                    wayland_client::protocol::wl_pointer::ButtonState::Pressed
                } else {
                    wayland_client::protocol::wl_pointer::ButtonState::Released
                };
                pointer.button(time_ms(), code, state);
                pointer.frame();
                event_queue.flush().map_err(|e| e.to_string())
            }
            PointerCmd::Scroll(direction, amount) => {
                // wl_pointer.axis: 0 = vertical, 1 = horizontal.
                // wl_fixed values are conventionally ±15.0 per wheel notch.
                let axis = match direction {
                    ScrollDirection::Up | ScrollDirection::Down => {
                        wayland_client::protocol::wl_pointer::Axis::VerticalScroll
                    }
                    ScrollDirection::Left | ScrollDirection::Right => {
                        wayland_client::protocol::wl_pointer::Axis::HorizontalScroll
                    }
                };
                let sign = match direction {
                    ScrollDirection::Down | ScrollDirection::Right => 1.0,
                    ScrollDirection::Up | ScrollDirection::Left => -1.0,
                };
                let magnitude = amount.unsigned_abs() as f64;
                let value = sign * magnitude * 15.0;
                let steps = (sign * magnitude) as i32;
                pointer.axis_source(wayland_client::protocol::wl_pointer::AxisSource::Wheel);
                pointer.axis_discrete(time_ms(), axis, value, steps);
                pointer.frame();
                event_queue.flush().map_err(|e| e.to_string())
            }
        };

        if let Err(e) = result {
            logger.warn(format!(
                "WlrVirtualPointer flush error: {} — pointer output may be lost until \
                 the compositor recovers; restart wayclickd if the issue persists",
                e
            ));
            // Try to drain any pending events so the next iteration is sane.
            // A protocol error here is logged and the thread continues — when
            // the compositor closes the connection, the next flush will fail
            // again and the supervisor (daemon) restart is the recovery path.
            if let Err(e) = event_queue.dispatch_pending(&mut probe) {
                logger.error(format!(
                    "WlrVirtualPointer dispatch_pending error after flush failure: {} — \
                     Wayland connection is likely broken",
                    e
                ));
            }
        }

        // Don't block forever in dispatch — but we do want to surface protocol
        // errors promptly. Read all queued events without blocking.
        if let Err(e) = event_queue.dispatch_pending(&mut probe) {
            logger.warn(format!(
                "WlrVirtualPointer dispatch_pending error: {} — Wayland connection may be \
                 unhealthy",
                e
            ));
        }
        // Keep the connection variable alive (avoids "unused" lint clarification).
        let _ = &conn;
    }
}

/// Registry probe that doesn't care about specific globals — we use
/// `registry_queue_init` + `globals.bind()` instead of manual matching.
struct RegistryProbe;

impl Dispatch<WlRegistry, GlobalListContents> for RegistryProbe {
    fn event(
        _state: &mut Self,
        _proxy: &WlRegistry,
        _event: <WlRegistry as Proxy>::Event,
        _data: &GlobalListContents,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrVirtualPointerManagerV1, ()> for RegistryProbe {
    fn event(
        _state: &mut Self,
        _proxy: &ZwlrVirtualPointerManagerV1,
        _event: <ZwlrVirtualPointerManagerV1 as Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrVirtualPointerV1, ()> for RegistryProbe {
    fn event(
        _state: &mut Self,
        _proxy: &ZwlrVirtualPointerV1,
        _event: <ZwlrVirtualPointerV1 as Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlSeat, ()> for RegistryProbe {
    fn event(
        _state: &mut Self,
        _proxy: &WlSeat,
        _event: <WlSeat as Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_extents_clamps_zero() {
        let e = DesktopExtents::new(0, 0);
        assert_eq!(e.width, 1);
        assert_eq!(e.height, 1);
    }

    #[test]
    #[ignore = "mutates WAYLAND_DISPLAY/XDG_RUNTIME_DIR; race-prone under \
                `cargo test` parallel execution. Run with `--ignored` in a \
                serial harness (e.g. `cargo test -- --ignored --test-threads=1`)."]
    fn try_new_without_wayland_display_errors() {
        // Force no WAYLAND_DISPLAY by pointing it at a bogus value.
        let prev = std::env::var("WAYLAND_DISPLAY").ok();
        let prev_runtime = std::env::var("XDG_RUNTIME_DIR").ok();
        // Use unsafe due to env mutation in Rust 2024-edition style; safe here
        // because tests run single-threaded inside this fn.
        std::env::set_var("WAYLAND_DISPLAY", "wayclick-test-bogus");
        std::env::set_var("XDG_RUNTIME_DIR", "/tmp/wayclick-nonexistent");
        let logger = Arc::new(Logger::new(100, crate::logger::LogLevel::Trace, false));
        logger.set_quiet(true);
        let result = WlrVirtualPointer::try_new(logger, DesktopExtents::new(1920, 1080));
        assert!(
            result.is_err(),
            "expected error when no compositor reachable"
        );
        // Restore env for other tests.
        match prev {
            Some(v) => std::env::set_var("WAYLAND_DISPLAY", v),
            None => std::env::remove_var("WAYLAND_DISPLAY"),
        }
        match prev_runtime {
            Some(v) => std::env::set_var("XDG_RUNTIME_DIR", v),
            None => std::env::remove_var("XDG_RUNTIME_DIR"),
        }
    }
}
