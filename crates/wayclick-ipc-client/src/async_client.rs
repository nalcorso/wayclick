// SPDX-License-Identifier: MIT
//! Background-thread streaming client for the wayclick daemon.
//!
//! [`AsyncClient::connect`] spawns a background thread that:
//! 1. Connects to the daemon's Unix socket.
//! 2. Performs the wayclick handshake: `status` → `list_triggers` → `subscribe` → `get_focus`.
//! 3. Emits an [`IpcMessage::Connected`] with initial state, then enters a
//!    non-blocking event loop that dispatches server events as typed [`IpcMessage`]s.
//! 4. Reconnects automatically (with a 2-second backoff) on disconnect, until
//!    a [`IpcCommand::Shutdown`] is received or the message channel is dropped.
//!
//! Outbound calls go through [`AsyncClient::send`] / [`AsyncClient::send_json`],
//! which enqueue [`IpcCommand::Send`] for the background thread to write.
//! Responses to user-issued requests appear on the message channel only when
//! they have a recognised event shape — for arbitrary request/response use
//! [`crate::SyncClient`] instead.

use crate::frame::IpcError;
use crate::types::{FocusedWindow, ServiceStatus, TriggerInfo};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvError, Sender, TryRecvError};
use std::time::Duration;

const MAX_FRAME: usize = crate::frame::MAX_FRAME_SIZE as usize;
const RECONNECT_BACKOFF: Duration = Duration::from_secs(2);
const PING_INTERVAL: Duration = Duration::from_secs(20);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
const READ_POLL_SLEEP: Duration = Duration::from_millis(10);

/// Events emitted by the background IPC thread.
#[derive(Debug, Clone)]
pub enum IpcMessage {
    /// Successful connection + initial state.
    Connected {
        status: ServiceStatus,
        triggers: Vec<TriggerInfo>,
        initial_focus: Option<FocusedWindow>,
    },
    /// Connection lost. The background thread will attempt to reconnect.
    Disconnected,
    TriggerActivated(String),
    TriggerDeactivated(String),
    RawInput {
        code: u16,
        value: i32,
        device_name: String,
    },
    LayerChanged {
        from: String,
        to: String,
    },
    EnabledChanged(bool),
    ConfigReloaded,
    /// Refreshed list of triggers (sent automatically after a `ConfigReloaded`).
    TriggerListUpdated(Vec<TriggerInfo>),
    FocusChanged(Option<FocusedWindow>),
    ScrollReceived {
        delta_x: i32,
        delta_y: i32,
    },
}

/// Outbound commands sent from the user to the background thread.
#[derive(Debug, Clone)]
pub enum IpcCommand {
    /// Send a raw JSON value to the daemon (typically a JSON-RPC request).
    Send(Value),
    /// Cleanly shut down the background thread.
    Shutdown,
}

/// Streaming client. Drop the value or call [`AsyncClient::shutdown`] to stop the background thread.
pub struct AsyncClient {
    msg_rx: Receiver<IpcMessage>,
    cmd_tx: Sender<IpcCommand>,
}

impl AsyncClient {
    /// Spawn the background thread and return a client handle.
    /// Connection happens asynchronously; observe [`IpcMessage::Connected`] /
    /// [`IpcMessage::Disconnected`] on the message channel.
    pub fn connect(socket_path: PathBuf) -> Result<AsyncClient, IpcError> {
        let (msg_tx, msg_rx) = mpsc::channel::<IpcMessage>();
        let (cmd_tx, cmd_rx) = mpsc::channel::<IpcCommand>();

        std::thread::spawn(move || run_supervisor(socket_path, msg_tx, cmd_rx));

        Ok(AsyncClient { msg_rx, cmd_tx })
    }

    /// Enqueue a JSON-RPC request. Non-blocking; failure means the background thread has exited.
    pub fn send(&self, method: &str, params: Option<Value>) -> Result<(), IpcError> {
        let value = json!({
            "jsonrpc": "2.0",
            "id": null,
            "method": method,
            "params": params.unwrap_or(json!(null)),
        });
        self.send_json(value)
    }

    /// Enqueue an arbitrary JSON value (for fully custom request shapes).
    pub fn send_json(&self, value: Value) -> Result<(), IpcError> {
        self.cmd_tx
            .send(IpcCommand::Send(value))
            .map_err(|_| IpcError::ConnectionClosed)
    }

    /// Receive the next event, blocking until one is available or the thread exits.
    pub fn recv(&self) -> Result<IpcMessage, IpcError> {
        self.msg_rx.recv().map_err(|_: RecvError| IpcError::ConnectionClosed)
    }

    /// Receive the next event without blocking. Returns `Ok(None)` if no event is queued.
    pub fn try_recv(&self) -> Result<Option<IpcMessage>, IpcError> {
        match self.msg_rx.try_recv() {
            Ok(msg) => Ok(Some(msg)),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => Err(IpcError::ConnectionClosed),
        }
    }

    /// Request a clean shutdown. The background thread will exit on the next loop iteration.
    pub fn shutdown(&self) {
        let _ = self.cmd_tx.send(IpcCommand::Shutdown);
    }
}

// ─── Supervisor: reconnects on disconnect, exits on Shutdown ──────────────────

fn run_supervisor(
    socket_path: PathBuf,
    msg_tx: Sender<IpcMessage>,
    cmd_rx: Receiver<IpcCommand>,
) {
    loop {
        let shutdown = run_connection(&socket_path, &msg_tx, &cmd_rx);
        if shutdown || msg_tx.send(IpcMessage::Disconnected).is_err() {
            break;
        }
        std::thread::sleep(RECONNECT_BACKOFF);
    }
}

/// Returns true on Shutdown, false on connection loss.
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
    stream.set_read_timeout(Some(HANDSHAKE_TIMEOUT)).ok()?;
    stream.set_write_timeout(Some(HANDSHAKE_TIMEOUT)).ok()?;

    // ── Handshake ─────────────────────────────────────────────────────────
    let status_req = json!({"jsonrpc":"2.0","id":1,"method":"status","params":null});
    if !write_frame_blocking(&mut stream, &status_req) {
        return None;
    }
    let status_val = read_one_blocking(&mut stream)?;
    let status = parse_status(&status_val);

    let trig_req = json!({"jsonrpc":"2.0","id":2,"method":"list_triggers","params":null});
    if !write_frame_blocking(&mut stream, &trig_req) {
        return None;
    }
    let trig_val = read_one_blocking(&mut stream)?;
    let triggers = parse_triggers(&trig_val);

    let sub_req = json!({"jsonrpc":"2.0","id":3,"method":"subscribe","params":null});
    if !write_frame_blocking(&mut stream, &sub_req) {
        return None;
    }
    let _ = read_one_blocking(&mut stream)?;

    let focus_req = json!({"jsonrpc":"2.0","id":4,"method":"get_focus","params":null});
    let initial_focus = if write_frame_blocking(&mut stream, &focus_req) {
        read_one_blocking(&mut stream)
            .and_then(|v| {
                v.get("result")?
                    .get("window")
                    .map(|w| parse_focused_window(Some(w)))
            })
            .flatten()
    } else {
        None
    };

    let _ = msg_tx.send(IpcMessage::Connected {
        status,
        triggers,
        initial_focus,
    });

    // ── Event loop (non-blocking) ─────────────────────────────────────────
    stream.set_nonblocking(true).ok();
    stream.set_read_timeout(None).ok();
    stream.set_write_timeout(None).ok();

    let mut buf: Vec<u8> = Vec::with_capacity(4096);
    let mut tmp = [0u8; 4096];
    let mut next_id: u64 = 100;
    let mut pending_list_triggers: Option<u64> = None;
    let mut last_ping = std::time::Instant::now();

    loop {
        match stream.read(&mut tmp) {
            Ok(0) => return None,
            Ok(n) => buf.extend_from_slice(&tmp[..n]),
            Err(e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(_) => return None,
        }

        for frame in drain_frames(&mut buf) {
            if let Some(pending_id) = pending_list_triggers {
                if frame.get("id").and_then(|v| v.as_u64()) == Some(pending_id) {
                    let updated = parse_triggers(&frame);
                    let _ = msg_tx.send(IpcMessage::TriggerListUpdated(updated));
                    pending_list_triggers = None;
                    continue;
                }
            }
            if let Some(msg) = frame_to_message(&frame) {
                let is_reload = matches!(msg, IpcMessage::ConfigReloaded);
                let _ = msg_tx.send(msg);
                if is_reload && pending_list_triggers.is_none() {
                    let id = next_id;
                    next_id += 1;
                    let req = json!({"jsonrpc":"2.0","id":id,"method":"list_triggers","params":null});
                    if write_frame_nonblocking(&mut stream, &req) {
                        pending_list_triggers = Some(id);
                    }
                }
            }
        }

        loop {
            match cmd_rx.try_recv() {
                Ok(IpcCommand::Send(value)) => {
                    if !write_frame_nonblocking(&mut stream, &value) {
                        return None;
                    }
                }
                Ok(IpcCommand::Shutdown) => return Some(true),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return Some(true),
            }
        }

        if last_ping.elapsed() >= PING_INTERVAL {
            let req = json!({"jsonrpc":"2.0","id":null,"method":"ping","params":null});
            write_frame_nonblocking(&mut stream, &req);
            last_ping = std::time::Instant::now();
        }

        std::thread::sleep(READ_POLL_SLEEP);
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn write_frame_blocking(stream: &mut UnixStream, val: &Value) -> bool {
    match crate::frame::encode_frame(val) {
        Ok(frame) => stream.write_all(&frame).is_ok(),
        Err(_) => false,
    }
}

fn write_frame_nonblocking(stream: &mut UnixStream, val: &Value) -> bool {
    let frame = match crate::frame::encode_frame(val) {
        Ok(f) => f,
        Err(_) => return false,
    };
    stream.set_nonblocking(false).ok();
    stream.set_write_timeout(Some(Duration::from_secs(2))).ok();
    let ok = stream.write_all(&frame).is_ok();
    stream.set_nonblocking(true).ok();
    stream.set_write_timeout(None).ok();
    ok
}

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

fn drain_frames(buf: &mut Vec<u8>) -> Vec<Value> {
    let mut out = Vec::new();
    loop {
        if buf.len() < 4 {
            break;
        }
        let len = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
        if len > MAX_FRAME {
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

fn parse_status(v: &Value) -> ServiceStatus {
    let r = v.get("result").unwrap_or(v);
    serde_json::from_value(r.clone()).unwrap_or_default()
}

fn parse_triggers(v: &Value) -> Vec<TriggerInfo> {
    let r = v.get("result").cloned().unwrap_or(Value::Null);
    serde_json::from_value(r).unwrap_or_default()
}

fn parse_focused_window(val: Option<&Value>) -> Option<FocusedWindow> {
    let v = val?;
    if v.is_null() {
        return None;
    }
    serde_json::from_value(v.clone()).ok()
}

fn frame_to_message(val: &Value) -> Option<IpcMessage> {
    if val.get("method").and_then(|m| m.as_str()) != Some("event") {
        return None;
    }
    let params = val.get("params")?;
    let event_type = params.get("type").and_then(|t| t.as_str())?;

    match event_type {
        "trigger_activated" => Some(IpcMessage::TriggerActivated(
            params.get("trigger_id")?.as_str()?.to_string(),
        )),
        "trigger_deactivated" => Some(IpcMessage::TriggerDeactivated(
            params.get("trigger_id")?.as_str()?.to_string(),
        )),
        "layer_changed" => Some(IpcMessage::LayerChanged {
            from: str_or_default(params, "from"),
            to: str_or_default(params, "to"),
        }),
        "enabled_changed" => Some(IpcMessage::EnabledChanged(
            params.get("enabled")?.as_bool()?,
        )),
        "config_reloaded" => Some(IpcMessage::ConfigReloaded),
        "input_received" => Some(IpcMessage::RawInput {
            code: params.get("code")?.as_u64()? as u16,
            value: params.get("value")?.as_i64()? as i32,
            device_name: str_or_default(params, "device_name"),
        }),
        "scroll_received" => Some(IpcMessage::ScrollReceived {
            delta_x: params.get("delta_x")?.as_i64()? as i32,
            delta_y: params.get("delta_y")?.as_i64()? as i32,
        }),
        "focus_changed" => Some(IpcMessage::FocusChanged(parse_focused_window(
            params.get("window"),
        ))),
        _ => None,
    }
}

fn str_or_default(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string()
}
