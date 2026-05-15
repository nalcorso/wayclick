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
use crate::focus_tracker::{process_name_for_pid, CursorPosition, MonitorInfo, WindowInfo};
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

/// Builds a cursor-position query closure bound to the active Hyprland
/// command socket, or `None` when the socket directory is unavailable.
///
/// Each call performs a fresh connect → `j/cursorpos` → read → parse cycle;
/// Hyprland's command socket is one-shot per request and very cheap, so
/// callers can invoke this on demand (e.g. at mouse button press time)
/// without rate-limiting in this layer. Errors are mapped to `None` so
/// callers can treat the result as a simple "available / not available".
pub fn make_cursor_query() -> Option<Arc<dyn Fn() -> Option<CursorPosition> + Send + Sync>> {
    let dir = socket_dir_from_env()?;
    let cmd_path = dir.join(".socket.sock");
    Some(Arc::new(move || query_cursor_position(&cmd_path).ok()))
}

/// Sends `j/cursorpos` to the command socket and parses the JSON response.
fn query_cursor_position(socket_path: &Path) -> io::Result<CursorPosition> {
    let mut stream = UnixStream::connect(socket_path)?;
    stream.set_read_timeout(Some(Duration::from_millis(250)))?;
    stream.set_write_timeout(Some(Duration::from_millis(250)))?;
    stream.write_all(b"j/cursorpos")?;
    stream.shutdown(Shutdown::Write)?;

    let mut buf = String::new();
    stream.read_to_string(&mut buf)?;

    parse_cursor_position(buf.trim())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "unparseable cursorpos response"))
}

/// Parses a `j/cursorpos` JSON payload. Returns `None` for malformed input.
///
/// Hyprland's documented response is `{"x": <int>, "y": <int>}`. Older
/// versions sometimes returned floats; we accept both for robustness.
fn parse_cursor_position(s: &str) -> Option<CursorPosition> {
    let v: serde_json::Value = serde_json::from_str(s).ok()?;
    let x = v
        .get("x")?
        .as_i64()
        .or_else(|| v.get("x")?.as_f64().map(|f| f as i64))?;
    let y = v
        .get("y")?
        .as_i64()
        .or_else(|| v.get("y")?.as_f64().map(|f| f as i64))?;
    Some(CursorPosition {
        x: x as i32,
        y: y as i32,
    })
}

/// Returns a closure that queries Hyprland for current monitor layout via
/// `j/monitors`. The closure performs a fresh request each time, returning
/// the parsed list of monitors. Errors and parse failures map to `None`.
pub fn make_monitors_query() -> Option<Arc<dyn Fn() -> Option<Vec<MonitorInfo>> + Send + Sync>> {
    let dir = socket_dir_from_env()?;
    let cmd_path = dir.join(".socket.sock");
    Some(Arc::new(move || query_monitors(&cmd_path).ok()))
}

fn query_monitors(socket_path: &Path) -> io::Result<Vec<MonitorInfo>> {
    let mut stream = UnixStream::connect(socket_path)?;
    stream.set_read_timeout(Some(Duration::from_millis(500)))?;
    stream.set_write_timeout(Some(Duration::from_millis(500)))?;
    stream.write_all(b"j/monitors")?;
    stream.shutdown(Shutdown::Write)?;

    let mut buf = String::new();
    stream.read_to_string(&mut buf)?;

    parse_monitors(buf.trim())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "unparseable monitors response"))
}

/// Parses Hyprland's `j/monitors` JSON. Each monitor's `width`/`height` are
/// physical pixels; we convert to logical pixels by dividing by `scale`, and
/// swap W/H for 90°/270° transforms (transform values 1 and 3 per the
/// `wl_output` transform enum). Monitors missing any required field are
/// silently dropped.
fn parse_monitors(s: &str) -> Option<Vec<MonitorInfo>> {
    let v: serde_json::Value = serde_json::from_str(s).ok()?;
    let arr = v.as_array()?;
    let mut out = Vec::with_capacity(arr.len());
    for m in arr {
        let name = m.get("name")?.as_str()?.to_string();
        let description = m
            .get("description")
            .and_then(|d| d.as_str())
            .unwrap_or("")
            .to_string();
        let x = m.get("x")?.as_i64()? as i32;
        let y = m.get("y")?.as_i64()? as i32;
        let raw_w = m.get("width")?.as_i64()? as i32;
        let raw_h = m.get("height")?.as_i64()? as i32;
        // Treat missing/non-numeric `scale` as 1.0 rather than dropping the
        // monitor: older Hyprland versions and some test fixtures omit it.
        // The `?` on width/height keeps us strict for fields we actually need.
        let scale = m
            .get("scale")
            .and_then(|s| s.as_f64())
            .unwrap_or(1.0)
            .max(0.01);
        let transform = m.get("transform").and_then(|t| t.as_i64()).unwrap_or(0) as i32;

        // Hyprland reports raw physical pixels; convert to logical.
        let mut logical_width = (raw_w as f64 / scale).round() as i32;
        let mut logical_height = (raw_h as f64 / scale).round() as i32;
        if transform == 1 || transform == 3 || transform == 5 || transform == 7 {
            std::mem::swap(&mut logical_width, &mut logical_height);
        }

        out.push(MonitorInfo {
            name,
            description,
            x,
            y,
            logical_width,
            logical_height,
            scale,
            transform,
        });
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_cursor_position_int() {
        let p = parse_cursor_position(r#"{"x": 100, "y": 200}"#).unwrap();
        assert_eq!(p, CursorPosition { x: 100, y: 200 });
    }

    #[test]
    fn parse_cursor_position_float() {
        let p = parse_cursor_position(r#"{"x": 100.0, "y": 200.4}"#).unwrap();
        assert_eq!(p, CursorPosition { x: 100, y: 200 });
    }

    #[test]
    fn parse_cursor_position_missing_field() {
        assert!(parse_cursor_position(r#"{"x": 100}"#).is_none());
    }

    #[test]
    fn parse_cursor_position_malformed() {
        assert!(parse_cursor_position(r#"not json"#).is_none());
    }

    #[test]
    fn parse_monitors_payload_typical() {
        let payload = r#"[
            {"name":"HDMI-A-1","description":"AOC e2752Vq A","x":0,"y":0,"width":1920,"height":1080,"scale":1.0,"transform":0},
            {"name":"DP-2","description":"AOC e2752Vq B","x":1920,"y":0,"width":3840,"height":2160,"scale":1.5,"transform":0},
            {"name":"HDMI-A-2","description":"Other","x":4480,"y":0,"width":1080,"height":1920,"scale":1.0,"transform":3}
        ]"#;
        let mons = parse_monitors(payload).unwrap();
        assert_eq!(mons.len(), 3);
        assert_eq!(mons[1].name, "DP-2");
        // physical size scaled to logical
        assert_eq!(mons[1].logical_width, 2560);
        assert_eq!(mons[1].logical_height, 1440);
        assert_eq!(mons[1].x, 1920);
        // 270° rotation swaps W/H in logical space
        assert_eq!(mons[2].logical_width, 1920);
        assert_eq!(mons[2].logical_height, 1080);
    }

    #[test]
    fn parse_monitors_payload_missing_fields() {
        assert!(parse_monitors(r#"[{"name":"DP-2"}]"#)
            .map(|v| v.is_empty())
            .unwrap_or(true));
    }
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
