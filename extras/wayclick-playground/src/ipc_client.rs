// IPC client for wayclick-playground.
// Runs a background std::thread that maintains a connection to the wayclick daemon
// over its Unix socket, subscribes to all events, and exposes channel APIs to the
// macroquad main loop.
//
// The connection thread uses non-blocking I/O with a manual accumulator buffer to
// avoid partial-frame corruption that would occur with read_exact + read_timeout.
//
// Wire protocol: 4-byte big-endian length prefix + UTF-8 JSON payload (JSON-RPC 2.0).

use serde_json::{json, Value};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::time::Duration;

// ─── Public types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ServiceStatus {
    pub enabled: bool,
    #[allow(dead_code)]
    pub trigger_count: usize,
    #[allow(dead_code)]
    pub active_triggers: usize,
    pub layer: String,
    #[allow(dead_code)]
    pub uptime_secs: u64,
    pub dry_run: bool,
}

#[derive(Debug, Clone)]
pub struct TriggerInfo {
    pub id: String,
    pub name: String,
    pub mode: String,
    pub active: bool,
    pub activate_count: u64,
    pub user_enabled: bool,
    #[allow(dead_code)]
    pub dynamic: bool,
}

/// A focused window as reported by the wayclick daemon.
#[derive(Debug, Clone)]
pub struct FocusedWindow {
    pub app_id: String,
    pub title: String,
    pub process_name: Option<String>,
    #[allow(dead_code)]
    pub backend: String,
    pub xwayland: bool,
}

/// Messages sent from the IPC thread to the main (macroquad) thread.
#[derive(Debug)]
pub enum IpcMessage {
    Connected {
        status: ServiceStatus,
        triggers: Vec<TriggerInfo>,
        initial_focus: Option<FocusedWindow>,
    },
    Disconnected,
    TriggerActivated(String),
    TriggerDeactivated(String),
    RawInput {
        code: u16,
        value: i32,
        #[allow(dead_code)]
        device_name: String,
    },
    LayerChanged {
        #[allow(dead_code)]
        from: String,
        to: String,
    },
    EnabledChanged(bool),
    ConfigReloaded,
    TriggerListUpdated(Vec<TriggerInfo>),
    FocusChanged(Option<FocusedWindow>),
}

/// Commands sent from the main (macroquad) thread to the IPC background thread.
#[allow(dead_code)]
pub enum IpcCommand {
    FireTrigger(String),
    EnableTrigger(String),
    DisableTrigger(String),
    RefreshTriggers,
    Shutdown,
}

// ─── Socket path resolution ───────────────────────────────────────────────────

fn default_socket_path() -> PathBuf {
    if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR") {
        PathBuf::from(dir).join("wayclick.sock")
    } else {
        // Fallback: read UID from /proc/self/status
        let uid = read_proc_uid().unwrap_or(1000);
        PathBuf::from(format!("/tmp/wayclick-{uid}.sock"))
    }
}

fn read_proc_uid() -> Option<u32> {
    std::fs::read_to_string("/proc/self/status")
        .ok()?
        .lines()
        .find(|l| l.starts_with("Uid:"))?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}

// ─── Frame encoding helpers (duplicates ipc.rs logic, no dep on wayclick-core) ─

const MAX_FRAME: usize = 65536;

fn encode_frame(val: &Value) -> Option<Vec<u8>> {
    let json_bytes = serde_json::to_vec(val).ok()?;
    if json_bytes.len() > MAX_FRAME {
        return None;
    }
    let len = json_bytes.len() as u32;
    let mut frame = Vec::with_capacity(4 + json_bytes.len());
    frame.extend_from_slice(&len.to_be_bytes());
    frame.extend_from_slice(&json_bytes);
    Some(frame)
}

fn write_frame(stream: &mut UnixStream, val: &Value) -> bool {
    match encode_frame(val) {
        Some(frame) => stream.write_all(&frame).is_ok(),
        None => false,
    }
}

/// Parse complete JSON-RPC frames from `buf`, consuming each on success.
/// Returns decoded Values in order; stops when buffer is incomplete.
fn drain_frames(buf: &mut Vec<u8>) -> Vec<Value> {
    let mut out = Vec::new();
    loop {
        if buf.len() < 4 {
            break;
        }
        let len = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
        if len > MAX_FRAME {
            // Framing error — caller should drop the connection
            buf.clear();
            break;
        }
        if buf.len() < 4 + len {
            break;
        }
        if let Ok(val) = serde_json::from_slice::<Value>(&buf[4..4 + len]) {
            out.push(val);
        }
        buf.drain(..4 + len);
    }
    out
}

// ─── Response / event parsing ────────────────────────────────────────────────

fn parse_status(v: &Value) -> ServiceStatus {
    let r = v.get("result").unwrap_or(v);
    ServiceStatus {
        enabled: r.get("enabled").and_then(|v| v.as_bool()).unwrap_or(false),
        trigger_count: r
            .get("trigger_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize,
        active_triggers: r
            .get("active_triggers")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize,
        layer: r
            .get("layer")
            .and_then(|v| v.as_str())
            .unwrap_or("default")
            .to_string(),
        uptime_secs: r.get("uptime_secs").and_then(|v| v.as_u64()).unwrap_or(0),
        dry_run: r.get("dry_run").and_then(|v| v.as_bool()).unwrap_or(false),
    }
}

fn parse_triggers(v: &Value) -> Vec<TriggerInfo> {
    let arr = v
        .get("result")
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();
    arr.iter()
        .filter_map(|t| {
            Some(TriggerInfo {
                id: t.get("id")?.as_str()?.to_string(),
                name: t
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                mode: t
                    .get("mode")
                    .and_then(|v| v.as_str())
                    .unwrap_or("oneshot")
                    .to_string(),
                active: t.get("active").and_then(|v| v.as_bool()).unwrap_or(false),
                activate_count: t
                    .get("activate_count")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0),
                user_enabled: t
                    .get("user_enabled")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true),
                dynamic: t
                    .get("dynamic")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
            })
        })
        .collect()
}

/// Parse a `FocusedWindow` from a JSON value. Returns `None` if value is null/absent.
fn parse_focused_window(val: Option<&Value>) -> Option<FocusedWindow> {
    let obj = val?.as_object()?;
    let app_id = obj.get("app_id")?.as_str()?.to_string();
    Some(FocusedWindow {
        app_id,
        title: obj
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        process_name: obj
            .get("process_name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        backend: obj
            .get("backend")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string(),
        xwayland: obj
            .get("xwayland")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
    })
}

/// Convert a server-sent event frame to an IpcMessage. Returns None for unrecognised frames.
fn frame_to_message(val: &Value) -> Option<IpcMessage> {
    // Only unsolicited events have method = "event"
    if val.get("method").and_then(|m| m.as_str()) != Some("event") {
        return None;
    }
    let params = val.get("params")?;
    let event_type = params.get("type").and_then(|t| t.as_str())?;

    match event_type {
        "trigger_activated" => {
            let id = params.get("trigger_id")?.as_str()?.to_string();
            Some(IpcMessage::TriggerActivated(id))
        }
        "trigger_deactivated" => {
            let id = params.get("trigger_id")?.as_str()?.to_string();
            Some(IpcMessage::TriggerDeactivated(id))
        }
        "layer_changed" => {
            let from = params
                .get("from")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let to = params
                .get("to")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            Some(IpcMessage::LayerChanged { from, to })
        }
        "enabled_changed" => {
            let enabled = params.get("enabled")?.as_bool()?;
            Some(IpcMessage::EnabledChanged(enabled))
        }
        "config_reloaded" => Some(IpcMessage::ConfigReloaded),
        "input_received" => {
            let code = params.get("code")?.as_u64()? as u16;
            let value = params.get("value")?.as_i64()? as i32;
            let device_name = params
                .get("device_name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            Some(IpcMessage::RawInput {
                code,
                value,
                device_name,
            })
        }
        "focus_changed" => {
            let window = parse_focused_window(params.get("window"));
            Some(IpcMessage::FocusChanged(window))
        }
        _ => None,
    }
}

// ─── Connection lifecycle ─────────────────────────────────────────────────────

/// Run one full connection: handshake, subscribe, then event loop.
/// Returns false when the connection is lost (reconnect needed).
/// Returns true when Shutdown command received.
fn run_connection(
    socket_path: &PathBuf,
    msg_tx: &Sender<IpcMessage>,
    cmd_rx: &Receiver<IpcCommand>,
) -> bool {
    run_connection_inner(socket_path, msg_tx, cmd_rx).unwrap_or(false)
}

fn run_connection_inner(
    socket_path: &PathBuf,
    msg_tx: &Sender<IpcMessage>,
    cmd_rx: &Receiver<IpcCommand>,
) -> Option<bool> {
    let mut stream = UnixStream::connect(socket_path).ok()?;

    // ── Handshake (blocking, 5s timeout) ──────────────────────────────────
    let timeout = Duration::from_secs(5);
    stream.set_read_timeout(Some(timeout)).ok()?;
    stream.set_write_timeout(Some(timeout)).ok()?;

    // 1. Request status
    let status_req = json!({"jsonrpc":"2.0","id":1,"method":"status","params":null});
    if !write_frame(&mut stream, &status_req) {
        return None;
    }
    let status_val = read_one_blocking(&mut stream)?;
    let status = parse_status(&status_val);

    // 2. Request trigger list
    let trig_req = json!({"jsonrpc":"2.0","id":2,"method":"list_triggers","params":null});
    if !write_frame(&mut stream, &trig_req) {
        return None;
    }
    let trig_val = read_one_blocking(&mut stream)?;
    let triggers = parse_triggers(&trig_val);

    // 3. Subscribe to all events (after initial state is captured)
    let sub_req = json!({"jsonrpc":"2.0","id":3,"method":"subscribe","params":null});
    if !write_frame(&mut stream, &sub_req) {
        return None;
    }
    // Drain the subscribe response (we don't need its content)
    let _ = read_one_blocking(&mut stream)?;

    // 4. Query current focused window
    let focus_req = json!({"jsonrpc":"2.0","id":4,"method":"get_focus","params":null});
    let initial_focus = if write_frame(&mut stream, &focus_req) {
        read_one_blocking(&mut stream)
            .and_then(|v| v.get("result")?.get("window").map(|w| parse_focused_window(Some(w))))
            .flatten()
    } else {
        None
    };

    // Signal successful connection
    let _ = msg_tx.send(IpcMessage::Connected {
        status,
        triggers,
        initial_focus,
    });

    // ── Non-blocking event loop ───────────────────────────────────────────
    stream.set_nonblocking(true).ok();
    stream.set_read_timeout(None).ok();
    stream.set_write_timeout(None).ok();

    let mut buf: Vec<u8> = Vec::with_capacity(4096);
    let mut tmp = [0u8; 4096];
    // Request ID counter for in-flight requests during the event loop
    let mut next_id: u64 = 100;
    let mut pending_list_triggers: Option<u64> = None;
    // Keepalive: send a ping every 20s to prevent server-side idle timeouts.
    let mut last_ping = std::time::Instant::now();
    const PING_INTERVAL: Duration = Duration::from_secs(20);

    loop {
        // ── Read available data ───────────────────────────────────────────
        match stream.read(&mut tmp) {
            Ok(0) => return None, // connection closed
            Ok(n) => buf.extend_from_slice(&tmp[..n]),
            Err(e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                // No data available right now — normal in non-blocking mode
            }
            Err(_) => return None,
        }

        // ── Parse and dispatch complete frames ────────────────────────────
        for frame in drain_frames(&mut buf) {
            // Check if this is a response to a pending list_triggers request
            if let Some(pending_id) = pending_list_triggers {
                if frame.get("id").and_then(|v| v.as_u64()) == Some(pending_id) {
                    let updated = parse_triggers(&frame);
                    let _ = msg_tx.send(IpcMessage::TriggerListUpdated(updated));
                    pending_list_triggers = None;
                    continue;
                }
            }
            // Otherwise process as a subscription event
            if let Some(msg) = frame_to_message(&frame) {
                // On config reload, also queue a trigger list refresh
                let is_reload = matches!(msg, IpcMessage::ConfigReloaded);
                let _ = msg_tx.send(msg);
                if is_reload && pending_list_triggers.is_none() {
                    let id = next_id;
                    next_id += 1;
                    let req =
                        json!({"jsonrpc":"2.0","id":id,"method":"list_triggers","params":null});
                    if write_frame_nonblocking(&mut stream, &req) {
                        pending_list_triggers = Some(id);
                    }
                }
            }
        }

        // ── Drain commands from main thread ───────────────────────────────
        loop {
            match cmd_rx.try_recv() {
                Ok(IpcCommand::FireTrigger(trigger_id)) => {
                    let req = json!({
                        "jsonrpc": "2.0",
                        "id": null,
                        "method": "trigger",
                        "params": { "id": trigger_id, "press": true }
                    });
                    if !write_frame_nonblocking(&mut stream, &req) {
                        return None;
                    }
                }
                Ok(IpcCommand::EnableTrigger(trigger_id)) => {
                    let req = json!({
                        "jsonrpc": "2.0",
                        "id": null,
                        "method": "enable_trigger",
                        "params": { "id": trigger_id }
                    });
                    if !write_frame_nonblocking(&mut stream, &req) {
                        return None;
                    }
                }
                Ok(IpcCommand::DisableTrigger(trigger_id)) => {
                    let req = json!({
                        "jsonrpc": "2.0",
                        "id": null,
                        "method": "disable_trigger",
                        "params": { "id": trigger_id }
                    });
                    if !write_frame_nonblocking(&mut stream, &req) {
                        return None;
                    }
                }
                Ok(IpcCommand::RefreshTriggers) if pending_list_triggers.is_none() => {
                    let id = next_id;
                    next_id += 1;
                    let req =
                        json!({"jsonrpc":"2.0","id":id,"method":"list_triggers","params":null});
                    if write_frame_nonblocking(&mut stream, &req) {
                        pending_list_triggers = Some(id);
                    }
                }
                Ok(IpcCommand::RefreshTriggers) => {} // already pending
                Ok(IpcCommand::Shutdown) => return Some(true),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return Some(true),
            }
        }

        // ── Keepalive ping ─────────────────────────────────────────────────
        if last_ping.elapsed() >= PING_INTERVAL {
            let req = json!({"jsonrpc":"2.0","id":null,"method":"ping","params":null});
            // Ignore write failure — a genuine socket error will surface on the next read.
            write_frame_nonblocking(&mut stream, &req);
            last_ping = std::time::Instant::now();
        }

        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Read a single framed response in blocking mode.
/// Returns Some(value) or None on any error.
fn read_one_blocking(stream: &mut UnixStream) -> Option<Value> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).ok()?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > MAX_FRAME {
        return None;
    }
    let mut payload = vec![0u8; len];
    stream.read_exact(&mut payload).ok()?;
    serde_json::from_slice(&payload).ok()
}

/// Write a frame to a non-blocking socket.
/// Small frames rarely block, but we tolerate WouldBlock with a brief retry.
fn write_frame_nonblocking(stream: &mut UnixStream, val: &Value) -> bool {
    let frame = match encode_frame(val) {
        Some(f) => f,
        None => return false,
    };
    // Temporarily switch to blocking for the write (small frames are instant)
    stream.set_nonblocking(false).ok();
    stream.set_write_timeout(Some(Duration::from_secs(2))).ok();
    let ok = stream.write_all(&frame).is_ok();
    stream.set_nonblocking(true).ok();
    stream.set_write_timeout(None).ok();
    ok
}

// ─── Public API ───────────────────────────────────────────────────────────────

/// Spawn the IPC background thread. Returns (msg_rx, cmd_tx).
/// The caller should call `try_recv()` on `msg_rx` each frame and send commands via `cmd_tx`.
pub fn spawn_ipc_thread() -> (Receiver<IpcMessage>, Sender<IpcCommand>) {
    let (msg_tx, msg_rx) = mpsc::channel::<IpcMessage>();
    let (cmd_tx, cmd_rx) = mpsc::channel::<IpcCommand>();

    std::thread::spawn(move || {
        let socket_path = default_socket_path();
        loop {
            let shutdown = run_connection(&socket_path, &msg_tx, &cmd_rx);
            if shutdown || msg_tx.send(IpcMessage::Disconnected).is_err() {
                break;
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    });

    (msg_rx, cmd_tx)
}
