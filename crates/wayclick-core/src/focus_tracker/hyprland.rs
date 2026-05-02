// SPDX-License-Identifier: MIT
//! Hyprland compositor focus backend.
//!
//! Uses Hyprland's native IPC sockets:
//! - `.socket.sock`  — command socket; one-shot request/response.
//! - `.socket2.sock` — event socket; push stream of `EVENTNAME>>DATA\n` lines.
//!
//! # Protocol details
//! **Command socket**: Send raw bytes `b"j/activewindow"` (no newline, no length prefix),
//! call `shutdown(Write)` to signal end of request, then read the JSON response until EOF.
//!
//! **Event socket**: Read lines indefinitely. Each line: `eventname>>data`.
//! Relevant events: `activewindow>>class,title` and `activewindowv2>>address`.
//! Both are used only as triggers to re-query the command socket for full window info.
//!
//! A 500ms read timeout is set on the event socket so the background thread checks the
//! stop flag regularly without blocking indefinitely.

use crate::event_bus::{Event, EventBus};
use crate::focus_tracker::{process_name_for_pid, WindowInfo};
use crate::logger::Logger;
use serde::Deserialize;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub fn is_available() -> bool {
    std::env::var("HYPRLAND_INSTANCE_SIGNATURE").is_ok()
}

pub fn start(
    current: Arc<Mutex<Option<WindowInfo>>>,
    event_bus: Arc<EventBus>,
    logger: Arc<Logger>,
    stop: Arc<AtomicBool>,
) {
    let socket_dir = match socket_dir_from_env() {
        Some(d) => d,
        None => {
            logger.warn("Focus tracking: Hyprland socket dir not found");
            return;
        }
    };

    std::thread::spawn(move || {
        run_tracker(socket_dir, current, event_bus, logger, stop);
    });
}

/// Returns the Hyprland socket directory: `/run/user/<uid>/hypr/<signature>/`.
fn socket_dir_from_env() -> Option<PathBuf> {
    let sig = std::env::var("HYPRLAND_INSTANCE_SIGNATURE").ok()?;
    let uid = nix::unistd::getuid();
    let path = PathBuf::from(format!("/run/user/{}/hypr/{}", uid, sig));
    if path.exists() {
        Some(path)
    } else {
        None
    }
}

fn run_tracker(
    socket_dir: PathBuf,
    current: Arc<Mutex<Option<WindowInfo>>>,
    event_bus: Arc<EventBus>,
    logger: Arc<Logger>,
    stop: Arc<AtomicBool>,
) {
    // Populate initial focused window state.
    let cmd_path = socket_dir.join(".socket.sock");
    match query_active_window(&cmd_path) {
        Ok(Some(window)) => {
            *current.lock().unwrap_or_else(|e| e.into_inner()) = Some(window);
        }
        Ok(None) => {}
        Err(e) => {
            logger.debug(format!("Hyprland focus: initial query failed: {e}"));
        }
    }

    // Listen for focus change events and reconnect on disconnect.
    let event_path = socket_dir.join(".socket2.sock");
    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }

        match listen_events(&event_path, &cmd_path, &current, &event_bus, &stop) {
            Ok(()) => break, // stop requested
            Err(e) => {
                if !stop.load(Ordering::Relaxed) {
                    logger.debug(format!(
                        "Hyprland focus: event socket error ({e}), reconnecting…"
                    ));
                }
                std::thread::sleep(Duration::from_millis(500));
            }
        }
    }
}

/// Listens on `.socket2.sock` until an error or stop is requested.
/// On `activewindow` or `activewindowv2` events, re-queries the command socket for full info.
fn listen_events(
    event_path: &Path,
    cmd_path: &Path,
    current: &Arc<Mutex<Option<WindowInfo>>>,
    event_bus: &Arc<EventBus>,
    stop: &Arc<AtomicBool>,
) -> io::Result<()> {
    let stream = UnixStream::connect(event_path)?;
    stream.set_read_timeout(Some(Duration::from_millis(500)))?;
    let reader = BufReader::new(stream);

    for line in reader.lines() {
        if stop.load(Ordering::Relaxed) {
            return Ok(());
        }
        let line = match line {
            Ok(l) => l,
            Err(e)
                if e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::TimedOut =>
            {
                continue;
            }
            Err(e) => return Err(e),
        };

        if line.starts_with("activewindow>>") || line.starts_with("activewindowv2>>") {
            let new_window_opt = query_active_window(cmd_path).unwrap_or(None);
            let previous = {
                let mut guard = current.lock().unwrap_or_else(|e| e.into_inner());
                let prev = guard.clone();
                *guard = new_window_opt.clone();
                prev
            };
            // Only publish if the focused window actually changed.
            if new_window_opt != previous {
                event_bus.publish(&Event::focus_changed(new_window_opt, previous));
            }
        }
    }

    Err(io::Error::new(
        io::ErrorKind::UnexpectedEof,
        "socket closed",
    ))
}

/// Sends `j/activewindow` to the command socket and parses the JSON response.
/// Returns `Ok(None)` when no window is focused (empty object response).
fn query_active_window(socket_path: &Path) -> io::Result<Option<WindowInfo>> {
    let mut stream = UnixStream::connect(socket_path)?;
    stream.write_all(b"j/activewindow")?;
    stream.shutdown(Shutdown::Write)?;

    let mut buf = String::new();
    stream.read_to_string(&mut buf)?;

    let v: serde_json::Value = serde_json::from_str(buf.trim())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    // When no window is focused, Hyprland returns `{}`.
    if v.as_object().map(|o| o.is_empty()).unwrap_or(false) {
        return Ok(None);
    }

    Ok(Some(parse_window_info(&v)))
}

/// Raw Hyprland window JSON (fields we care about).
#[derive(Debug, Deserialize)]
struct HyprWindow {
    #[serde(default)]
    class: String,
    #[serde(default)]
    title: String,
    pid: Option<u32>,
    #[serde(default)]
    xwayland: bool,
}

fn parse_window_info(v: &serde_json::Value) -> WindowInfo {
    let w: HyprWindow = serde_json::from_value(v.clone()).unwrap_or(HyprWindow {
        class: String::new(),
        title: String::new(),
        pid: None,
        xwayland: false,
    });

    let process_name = w.pid.and_then(process_name_for_pid);

    // For XWayland windows the WM_CLASS (in `class`) is the authoritative identifier.
    // For native Wayland windows, `class` and `app_id` are typically the same value.
    let app_id = w.class.clone();
    let class = if w.class.is_empty() {
        None
    } else {
        Some(w.class)
    };

    WindowInfo {
        app_id,
        title: w.title,
        pid: w.pid,
        process_name,
        backend: "hyprland".to_string(),
        class,
        xwayland: w.xwayland,
    }
}
