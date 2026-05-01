//! Sway/i3-compatible compositor focus backend.
//!
//! Uses the i3 binary IPC protocol over `$SWAYSOCK`:
//!
//! ```text
//! Frame: "i3-ipc" (6 bytes magic) | length: u32 LE | type: u32 LE | payload: JSON
//! Message types: SUBSCRIBE=2, GET_TREE=4
//! Event types (MSB set): EVENT_WINDOW=0x80000003
//! ```
//!
//! On startup, a one-shot `GET_TREE` query finds the currently focused leaf node.
//! A persistent connection subscribes to `["window"]` events; on `change="focus"`,
//! the container info is parsed directly from the event payload.

use crate::event_bus::{Event, EventBus};
use crate::focus_tracker::{process_name_for_pid, WindowInfo};
use crate::logger::Logger;
use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const MAGIC: &[u8; 6] = b"i3-ipc";
const TYPE_SUBSCRIBE: u32 = 2;
const TYPE_GET_TREE: u32 = 4;
const EVENT_WINDOW: u32 = 0x80000003;

pub fn is_available() -> bool {
    std::env::var("SWAYSOCK").is_ok()
}

pub fn start(
    current: Arc<Mutex<Option<WindowInfo>>>,
    event_bus: Arc<EventBus>,
    logger: Arc<Logger>,
    stop: Arc<AtomicBool>,
) {
    let socket_path = match std::env::var("SWAYSOCK") {
        Ok(p) => PathBuf::from(p),
        Err(_) => {
            logger.warn("Focus tracking: SWAYSOCK not set");
            return;
        }
    };

    std::thread::spawn(move || {
        run_tracker(socket_path, current, event_bus, logger, stop);
    });
}

fn run_tracker(
    socket_path: PathBuf,
    current: Arc<Mutex<Option<WindowInfo>>>,
    event_bus: Arc<EventBus>,
    logger: Arc<Logger>,
    stop: Arc<AtomicBool>,
) {
    // Populate initial focused window via GET_TREE.
    if let Ok(window_opt) = get_focused_window(&socket_path) {
        *current.lock().unwrap_or_else(|e| e.into_inner()) = window_opt;
    }

    // Subscribe to window events and reconnect on disconnect.
    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }

        match subscribe_window_events(&socket_path, &current, &event_bus, &stop) {
            Ok(()) => break, // stop requested
            Err(e) => {
                if !stop.load(Ordering::Relaxed) {
                    logger.debug(format!("Sway focus: socket error ({e}), reconnecting…"));
                }
                std::thread::sleep(Duration::from_millis(500));
            }
        }
    }
}

// ─── i3 IPC framing ────────────────────────────────────────────────────────

fn send_message(stream: &mut UnixStream, msg_type: u32, payload: &[u8]) -> io::Result<()> {
    let len = payload.len() as u32;
    stream.write_all(MAGIC)?;
    stream.write_all(&len.to_le_bytes())?;
    stream.write_all(&msg_type.to_le_bytes())?;
    stream.write_all(payload)?;
    Ok(())
}

fn read_message(stream: &mut UnixStream) -> io::Result<(u32, Vec<u8>)> {
    let mut header = [0u8; 14]; // 6 magic + 4 len + 4 type
    stream.read_exact(&mut header)?;

    if &header[0..6] != MAGIC {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "bad i3-ipc magic"));
    }

    let length = u32::from_le_bytes(header[6..10].try_into().unwrap()) as usize;
    let msg_type = u32::from_le_bytes(header[10..14].try_into().unwrap());

    let mut payload = vec![0u8; length];
    stream.read_exact(&mut payload)?;
    Ok((msg_type, payload))
}

// ─── Window info parsing ────────────────────────────────────────────────────

fn window_info_from_container(node: &serde_json::Value) -> WindowInfo {
    let app_id = node
        .get("app_id")
        .and_then(|a| a.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            // XWayland: fall back to window_properties.instance
            node.get("window_properties")
                .and_then(|wp| wp.get("instance"))
                .and_then(|i| i.as_str())
                .unwrap_or("")
        })
        .to_string();

    let title = node
        .get("name")
        .and_then(|n| n.as_str())
        .unwrap_or("")
        .to_string();

    let pid: Option<u32> = node
        .get("pid")
        .and_then(|p| p.as_u64())
        .map(|p| p as u32);

    let process_name = pid.and_then(process_name_for_pid);

    let class = node
        .get("window_properties")
        .and_then(|wp| wp.get("class"))
        .and_then(|c| c.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let xwayland = node
        .get("type")
        .and_then(|t| t.as_str())
        .map(|t| t == "xwayland_view")
        .unwrap_or(false);

    WindowInfo {
        app_id,
        title,
        pid,
        process_name,
        backend: "sway".to_string(),
        class,
        xwayland,
    }
}

/// Recursively finds the focused leaf node in a Sway tree.
fn find_focused_node(node: &serde_json::Value) -> Option<&serde_json::Value> {
    if node.get("focused").and_then(|f| f.as_bool()).unwrap_or(false) {
        // Only return leaf nodes (actual windows, not containers)
        let node_type = node.get("type").and_then(|t| t.as_str()).unwrap_or("");
        if matches!(node_type, "con" | "floating_con" | "xwayland_view") {
            if node.get("app_id").is_some() || node.get("window_properties").is_some() {
                return Some(node);
            }
        }
    }

    for child_key in &["nodes", "floating_nodes"] {
        if let Some(children) = node.get(child_key).and_then(|c| c.as_array()) {
            for child in children {
                if let Some(found) = find_focused_node(child) {
                    return Some(found);
                }
            }
        }
    }

    None
}

// ─── One-shot GET_TREE ──────────────────────────────────────────────────────

fn get_focused_window(socket_path: &Path) -> io::Result<Option<WindowInfo>> {
    let mut stream = UnixStream::connect(socket_path)?;
    send_message(&mut stream, TYPE_GET_TREE, b"")?;
    let (_msg_type, payload) = read_message(&mut stream)?;

    let tree: serde_json::Value = serde_json::from_slice(&payload)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    Ok(find_focused_node(&tree).map(window_info_from_container))
}

// ─── Persistent subscription ────────────────────────────────────────────────

fn subscribe_window_events(
    socket_path: &Path,
    current: &Arc<Mutex<Option<WindowInfo>>>,
    event_bus: &Arc<EventBus>,
    stop: &Arc<AtomicBool>,
) -> io::Result<()> {
    let mut stream = UnixStream::connect(socket_path)?;
    stream.set_read_timeout(Some(Duration::from_millis(500)))?;

    // Subscribe to window events.
    send_message(&mut stream, TYPE_SUBSCRIBE, b"[\"window\"]")?;

    loop {
        if stop.load(Ordering::Relaxed) {
            return Ok(());
        }

        let (msg_type, payload) = match read_message(&mut stream) {
            Ok(m) => m,
            Err(e)
                if e.kind() == io::ErrorKind::WouldBlock
                    || e.kind() == io::ErrorKind::TimedOut =>
            {
                continue;
            }
            Err(e) => return Err(e),
        };

        if msg_type != EVENT_WINDOW {
            continue;
        }

        let ev: serde_json::Value = match serde_json::from_slice(&payload) {
            Ok(v) => v,
            Err(_) => continue,
        };

        // We only care about focus events.
        if ev.get("change").and_then(|c| c.as_str()) != Some("focus") {
            continue;
        }

        let container = match ev.get("container") {
            Some(c) => c,
            None => continue,
        };

        let new_window = window_info_from_container(container);
        let previous = {
            let mut guard = current.lock().unwrap_or_else(|e| e.into_inner());
            let prev = guard.clone();
            *guard = Some(new_window.clone());
            prev
        };

        if Some(&new_window) != previous.as_ref() {
            event_bus.publish(&Event::focus_changed(Some(new_window), previous));
        }
    }
}
