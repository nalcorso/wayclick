use crate::config::{TriggerBinding, TriggerMode};
use crate::engine::{with_engine_events, Engine};
use crate::event_bus::{EventBus, EventType};
use crate::logger::Logger;
use serde::Deserialize;
use serde_json::{json, Value};
use std::io::{self, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use thiserror::Error;

/// Maximum concurrent IPC client connections.
/// Prevents connection flood attacks from exhausting system threads.
const MAX_IPC_CONNECTIONS: usize = 32;

#[derive(Debug, Error)]
pub enum IpcError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Frame too large: {0} bytes (max 65536)")]
    FrameTooLarge(u32),
    #[error("Connection closed")]
    ConnectionClosed,
}

const MAX_FRAME_SIZE: u32 = 65536;

/// Encode a JSON-RPC frame with 4-byte big-endian length prefix.
pub fn encode_frame(payload: &Value) -> Result<Vec<u8>, IpcError> {
    let json_bytes = serde_json::to_vec(payload)?;
    let len = json_bytes.len() as u32;
    if len > MAX_FRAME_SIZE {
        return Err(IpcError::FrameTooLarge(len));
    }
    let mut frame = Vec::with_capacity(4 + json_bytes.len());
    frame.extend_from_slice(&len.to_be_bytes());
    frame.extend_from_slice(&json_bytes);
    Ok(frame)
}

/// Decode a length-prefixed JSON-RPC frame from a reader.
pub fn decode_frame(reader: &mut impl Read) -> Result<Value, IpcError> {
    let mut len_buf = [0u8; 4];
    reader.read_exact(&mut len_buf).map_err(|e| {
        if e.kind() == io::ErrorKind::UnexpectedEof {
            IpcError::ConnectionClosed
        } else {
            IpcError::Io(e)
        }
    })?;
    let len = u32::from_be_bytes(len_buf);
    if len > MAX_FRAME_SIZE {
        return Err(IpcError::FrameTooLarge(len));
    }
    let mut payload = vec![0u8; len as usize];
    reader.read_exact(&mut payload)?;
    let value: Value = serde_json::from_slice(&payload)?;
    Ok(value)
}

/// Write a frame to a writer.
pub fn write_frame(writer: &mut impl Write, payload: &Value) -> Result<(), IpcError> {
    let frame = encode_frame(payload)?;
    writer.write_all(&frame)?;
    writer.flush()?;
    Ok(())
}

fn make_response(id: Option<&Value>, result: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id.cloned().unwrap_or(Value::Null),
        "result": result,
    })
}

fn make_error(id: Option<&Value>, code: i32, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id.cloned().unwrap_or(Value::Null),
        "error": {
            "code": code,
            "message": message,
        },
    })
}

/// Handle a single JSON-RPC request.
pub fn handle_request(request: &Value, engine: &Arc<Mutex<Engine>>, logger: &Arc<Logger>) -> Value {
    let id = request.get("id");
    let method = request.get("method").and_then(|m| m.as_str()).unwrap_or("");
    let params = request.get("params");

    match method {
        "ping" => make_response(id, json!("pong")),

        "status" => {
            let engine = engine.lock().unwrap();
            let status = engine.describe_status();
            make_response(
                id,
                json!({
                    "enabled": status.enabled,
                    "dry_run": status.dry_run,
                    "trigger_count": status.trigger_count,
                    "active_triggers": status.active_triggers,
                    "backend": status.backend,
                    "config_path": status.config_path,
                    "uptime_secs": status.uptime_secs,
                    "layer": engine.current_layer(),
                }),
            )
        }

        "status_json" => {
            let engine = engine.lock().unwrap();
            let status = engine.describe_status();
            make_response(id, serde_json::to_value(&status).unwrap_or(json!(null)))
        }

        "toggle" => {
            let new_state = with_engine_events(engine, |eng| eng.toggle_enabled());
            make_response(id, json!({ "enabled": new_state }))
        }

        "enable" => {
            with_engine_events(engine, |eng| eng.set_enabled(true));
            make_response(id, json!({ "enabled": true }))
        }

        "disable" => {
            with_engine_events(engine, |eng| eng.set_enabled(false));
            make_response(id, json!({ "enabled": false }))
        }

        "trigger" => {
            let trigger_id = params
                .and_then(|p| p.get("id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let press = params
                .and_then(|p| p.get("press"))
                .and_then(|v| v.as_bool())
                .unwrap_or(true);

            if trigger_id.is_empty() {
                return make_error(id, -32602, "Missing 'id' parameter");
            }

            let trigger_id = trigger_id.to_string();
            match with_engine_events(engine, |eng| eng.trigger_event(&trigger_id, press)) {
                Ok(()) => make_response(id, json!({ "triggered": trigger_id })),
                Err(e) => make_error(id, -32000, &e.to_string()),
            }
        }

        "list_triggers" => {
            let engine = engine.lock().unwrap();
            let triggers = engine.triggers_snapshot();
            make_response(id, serde_json::to_value(&triggers).unwrap_or(json!([])))
        }

        "list_layers" => {
            let engine = engine.lock().unwrap();
            let layers = engine.available_layers();
            let current = engine.current_layer().to_string();
            make_response(id, json!({ "layers": layers, "current": current }))
        }

        "reload_config" => {
            // Config reload is handled by the daemon; IPC just signals it
            make_response(id, json!({ "reloading": true }))
        }

        "logs_tail" => {
            let n = params
                .and_then(|p| p.get("n"))
                .and_then(|v| v.as_u64())
                .unwrap_or(50) as usize;

            let entries = logger.recent(n);
            let logs: Vec<Value> = entries
                .iter()
                .map(|e| {
                    json!({
                        "timestamp": e.format_iso8601(),
                        "level": e.level.to_string(),
                        "message": e.message,
                    })
                })
                .collect();
            make_response(id, json!(logs))
        }

        "set_layer" => {
            let layer = params
                .and_then(|p| p.get("layer"))
                .and_then(|v| v.as_str())
                .unwrap_or("");

            if layer.is_empty() {
                return make_error(id, -32602, "Missing 'layer' parameter");
            }

            let layer = layer.to_string();
            with_engine_events(engine, |eng| eng.set_layer(layer.clone()));
            make_response(id, json!({ "layer": layer }))
        }

        "get_layer" => {
            let engine = engine.lock().unwrap();
            make_response(id, json!({ "layer": engine.current_layer() }))
        }

        "enable_trigger" => {
            let trigger_id = params
                .and_then(|p| p.get("id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if trigger_id.is_empty() {
                return make_error(id, -32602, "Missing 'id' parameter");
            }
            match with_engine_events(engine, |eng| eng.enable_trigger(trigger_id)) {
                Ok(()) => make_response(id, json!({ "enabled": trigger_id })),
                Err(e) => make_error(id, -32602, &e.to_string()),
            }
        }

        "disable_trigger" => {
            let trigger_id = params
                .and_then(|p| p.get("id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if trigger_id.is_empty() {
                return make_error(id, -32602, "Missing 'id' parameter");
            }
            match with_engine_events(engine, |eng| eng.disable_trigger(trigger_id)) {
                Ok(()) => make_response(id, json!({ "disabled": trigger_id })),
                Err(e) => make_error(id, -32602, &e.to_string()),
            }
        }

        _ => make_error(id, -32601, &format!("Method not found: {}", method)),
    }
}

/// RAII guard that decrements the connection counter when dropped.
struct ConnectionGuard {
    counter: Arc<AtomicUsize>,
}

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::Relaxed);
    }
}

/// Messages sent from the reader and event-forwarder threads to the writer thread.
enum WriterMsg {
    Frame(Value),
    Close,
}

/// Deserialization helper for `register_trigger` params (name and description are optional).
#[derive(Deserialize)]
struct RegisterTriggerParams {
    id: String,
    #[serde(default)]
    name: Option<String>,
    mode: TriggerMode,
    action: crate::config::ActionConfig,
    #[serde(default)]
    cooldown_ms: Option<u32>,
}

pub struct IpcServer {
    socket_path: PathBuf,
    listener: Option<UnixListener>,
    engine: Arc<Mutex<Engine>>,
    logger: Arc<Logger>,
    event_bus: Arc<EventBus>,
    shutdown: Arc<std::sync::atomic::AtomicBool>,
    connection_count: Arc<AtomicUsize>,
    connection_id_counter: Arc<AtomicU64>,
}

impl IpcServer {
    pub fn new(
        socket_path: PathBuf,
        engine: Arc<Mutex<Engine>>,
        logger: Arc<Logger>,
        event_bus: Arc<EventBus>,
    ) -> Result<Self, IpcError> {
        // Remove existing socket
        let _ = std::fs::remove_file(&socket_path);

        // Create parent directory
        if let Some(parent) = socket_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let listener = UnixListener::bind(&socket_path)?;

        // Set socket permissions to 0600
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o600);
            std::fs::set_permissions(&socket_path, perms)?;
        }

        // Set non-blocking so we can check for shutdown
        listener.set_nonblocking(true)?;

        logger.info(format!("IPC server listening on {:?}", socket_path));

        Ok(Self {
            socket_path,
            listener: Some(listener),
            engine,
            logger,
            event_bus,
            shutdown: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            connection_count: Arc::new(AtomicUsize::new(0)),
            connection_id_counter: Arc::new(AtomicU64::new(1)),
        })
    }

    pub fn shutdown_flag(&self) -> Arc<std::sync::atomic::AtomicBool> {
        self.shutdown.clone()
    }

    /// Run the IPC server, blocking the current thread. Call from a dedicated thread.
    pub fn run(&self) {
        let listener = match &self.listener {
            Some(l) => l,
            None => return,
        };

        loop {
            if self.shutdown.load(std::sync::atomic::Ordering::Relaxed) {
                break;
            }

            match listener.accept() {
                Ok((stream, _addr)) => {
                    let current = self.connection_count.fetch_add(1, Ordering::Relaxed);
                    if current >= MAX_IPC_CONNECTIONS {
                        self.connection_count.fetch_sub(1, Ordering::Relaxed);
                        self.logger.warn(format!(
                            "IPC connection rejected: {} active connections (max {})",
                            current, MAX_IPC_CONNECTIONS
                        ));
                        drop(stream);
                        continue;
                    }

                    let engine = self.engine.clone();
                    let logger = self.logger.clone();
                    let event_bus = self.event_bus.clone();
                    let conn_id = self
                        .connection_id_counter
                        .fetch_add(1, Ordering::Relaxed);
                    let guard = ConnectionGuard {
                        counter: self.connection_count.clone(),
                    };
                    thread::spawn(move || {
                        let _guard = guard;
                        handle_client(stream, engine, logger, event_bus, conn_id);
                    });
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(50));
                    continue;
                }
                Err(e) => {
                    self.logger.error(format!("IPC accept error: {}", e));
                    thread::sleep(Duration::from_millis(100));
                }
            }
        }

        self.logger.info("IPC server shutting down");
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }
}

impl Drop for IpcServer {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

fn handle_client(
    stream: UnixStream,
    engine: Arc<Mutex<Engine>>,
    logger: Arc<Logger>,
    event_bus: Arc<EventBus>,
    conn_id: u64,
) {
    // Bounded writer channel — prevents unbounded memory growth from slow writers.
    let (writer_tx, writer_rx) = std::sync::mpsc::sync_channel::<WriterMsg>(64);

    let mut stream_read = stream;
    let stream_write = match stream_read.try_clone() {
        Ok(s) => s,
        Err(e) => {
            logger.warn(format!("IPC stream clone failed, closing connection: {}", e));
            return;
        }
    };

    stream_read.set_nonblocking(false).unwrap_or_default();
    stream_read
        .set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap_or_default();

    // Writer thread: drains WriterMsg channel and sends frames to the socket.
    let writer_tx_for_writer = writer_tx.clone();
    let writer_handle = thread::spawn(move || {
        handle_client_writer(stream_write, writer_rx);
        drop(writer_tx_for_writer);
    });

    // Subscription state for this connection.
    let mut sub_stop_tx: Option<std::sync::mpsc::Sender<()>> = None;

    loop {
        match decode_frame(&mut stream_read) {
            Ok(request) => {
                let response = handle_request_with_conn(
                    &request,
                    &engine,
                    &logger,
                    conn_id,
                    &event_bus,
                    &writer_tx,
                    &mut sub_stop_tx,
                );
                if writer_tx.send(WriterMsg::Frame(response)).is_err() {
                    break;
                }
            }
            Err(IpcError::ConnectionClosed) => break,
            Err(e) => {
                logger.debug(format!("IPC read error: {}", e));
                break;
            }
        }
    }

    // Stop subscription forwarder (drops the EventBus subscriber).
    if let Some(tx) = sub_stop_tx.take() {
        let _ = tx.send(());
    }

    // Clean up all dynamic triggers registered by this connection.
    engine.lock().unwrap().cleanup_connection(conn_id);

    // Signal writer to stop and wait for it.
    let _ = writer_tx.send(WriterMsg::Close);
    writer_handle.join().ok();
}

fn handle_client_writer(mut stream: UnixStream, rx: std::sync::mpsc::Receiver<WriterMsg>) {
    for msg in rx {
        match msg {
            WriterMsg::Frame(val) => {
                if write_frame(&mut stream, &val).is_err() {
                    break;
                }
            }
            WriterMsg::Close => break,
        }
    }
}

/// Handle a JSON-RPC request with full connection context.
/// Falls through to `handle_request` for methods that don't need connection state.
fn handle_request_with_conn(
    request: &Value,
    engine: &Arc<Mutex<Engine>>,
    logger: &Arc<Logger>,
    conn_id: u64,
    event_bus: &Arc<EventBus>,
    writer_tx: &std::sync::mpsc::SyncSender<WriterMsg>,
    sub_stop_tx: &mut Option<std::sync::mpsc::Sender<()>>,
) -> Value {
    let id = request.get("id");
    let method = request.get("method").and_then(|m| m.as_str()).unwrap_or("");
    let params = request.get("params");

    match method {
        "subscribe" => {
            // Stop existing subscription before creating a new one.
            if let Some(tx) = sub_stop_tx.take() {
                let _ = tx.send(());
            }

            let filter = parse_event_filter(params);
            let event_rx = event_bus.subscribe(filter.clone());
            let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();
            *sub_stop_tx = Some(stop_tx);

            let writer_tx_clone = writer_tx.clone();
            thread::spawn(move || {
                loop {
                    if stop_rx.try_recv().is_ok() {
                        break;
                    }
                    match event_rx.recv_timeout(Duration::from_millis(50)) {
                        Ok(event) => {
                            // Re-check stop before forwarding to close the race window
                            // between unsubscribe sending the stop signal and this thread
                            // waking up from recv_timeout.
                            if stop_rx.try_recv().is_ok() {
                                break;
                            }
                            let notification = json!({
                                "jsonrpc": "2.0",
                                "id": Value::Null,
                                "method": "event",
                                "params": serde_json::to_value(&event).unwrap_or(json!(null)),
                            });
                            if writer_tx_clone.send(WriterMsg::Frame(notification)).is_err() {
                                break;
                            }
                        }
                        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                    }
                }
            });

            let subscribed_types = match &filter {
                None => json!("all"),
                Some(types) => {
                    json!(types.iter().map(|t| serde_json::to_value(t).unwrap_or(json!(null))).collect::<Vec<_>>())
                }
            };
            make_response(id, json!({ "subscribed": true, "events": subscribed_types }))
        }

        "unsubscribe" => {
            if let Some(tx) = sub_stop_tx.take() {
                let _ = tx.send(());
            }
            make_response(id, json!({ "subscribed": false }))
        }

        "register_trigger" => {
            let params = match params {
                Some(p) => p,
                None => return make_error(id, -32602, "Missing params"),
            };
            let rtp: RegisterTriggerParams = match serde_json::from_value(params.clone()) {
                Ok(p) => p,
                Err(e) => {
                    return make_error(id, -32602, &format!("Invalid params: {}", e));
                }
            };
            let trigger_id = rtp.id.clone();
            let trigger = TriggerBinding {
                id: rtp.id.clone(),
                name: rtp.name.unwrap_or_else(|| rtp.id.clone()),
                description: String::new(),
                mode: rtp.mode,
                action: rtp.action,
                cooldown_ms: rtp.cooldown_ms,
            };

            // Validation and duplicate-checking are handled inside the engine so
            // that dynamic triggers and static config are both subject to the same rules.
            match engine
                .lock()
                .unwrap()
                .register_dynamic_trigger(trigger, conn_id)
            {
                Ok(()) => make_response(id, json!({ "registered": trigger_id })),
                Err(crate::engine::EngineError::DuplicateTrigger(t)) => {
                    make_error(id, -32602, &format!("Duplicate trigger ID: {}", t))
                }
                Err(crate::engine::EngineError::InvalidConfig(msg)) => {
                    make_error(id, -32602, &format!("Validation error: {}", msg))
                }
                Err(e) => make_error(id, -32000, &e.to_string()),
            }
        }

        "unregister_trigger" => {
            let tid = params
                .and_then(|p| p.get("id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if tid.is_empty() {
                return make_error(id, -32602, "Missing 'id' parameter");
            }
            match engine
                .lock()
                .unwrap()
                .unregister_dynamic_trigger(tid, conn_id)
            {
                Ok(()) => make_response(id, json!({ "unregistered": tid })),
                Err(e) => make_error(id, -32000, &e.to_string()),
            }
        }

        "list_dynamic_triggers" => {
            let triggers = engine
                .lock()
                .unwrap()
                .dynamic_triggers_for_connection(conn_id);
            make_response(id, serde_json::to_value(&triggers).unwrap_or(json!([])))
        }

        _ => handle_request(request, engine, logger),
    }
}

/// Parse an optional event filter from subscribe params.
/// `None` params or absent/null `events` field → subscribe to all.
fn parse_event_filter(params: Option<&Value>) -> Option<Vec<EventType>> {
    let events = params?.get("events")?;
    if events.is_null() {
        return None;
    }
    let arr = events.as_array()?;
    let types: Vec<EventType> = arr
        .iter()
        .filter_map(|v| v.as_str())
        .filter_map(EventType::from_str)
        .collect();
    if types.is_empty() { None } else { Some(types) }
}

/// Client-side helper: connect to daemon socket with read/write timeouts set.
/// Returns a connected `UnixStream` that callers can use for framed IPC communication.
/// Use this for long-lived streaming connections (e.g. event subscriptions).
pub fn ipc_connect(socket_path: &Path, timeout_ms: u64) -> Result<UnixStream, IpcError> {
    let stream = UnixStream::connect(socket_path)?;
    let timeout = Duration::from_millis(timeout_ms);
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    Ok(stream)
}

/// Client-side helper: connect to daemon and send a single request, return response.
pub fn ipc_request(
    socket_path: &Path,
    method: &str,
    params: Option<Value>,
) -> Result<Value, IpcError> {
    let mut stream = ipc_connect(socket_path, 5000)?;

    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": method,
        "params": params.unwrap_or(json!(null)),
    });

    write_frame(&mut stream, &request)?;
    let response = decode_frame(&mut stream)?;
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event_bus::EventBus;

    #[test]
    fn test_frame_encode_decode() {
        let payload = json!({"jsonrpc": "2.0", "id": 1, "method": "ping"});
        let encoded = encode_frame(&payload).unwrap();

        let mut cursor = io::Cursor::new(encoded);
        let decoded = decode_frame(&mut cursor).unwrap();

        assert_eq!(payload, decoded);
    }

    #[test]
    fn test_frame_encode_decode_roundtrip() {
        let payloads = vec![
            json!({"method": "status"}),
            json!({"result": {"enabled": true, "triggers": [1,2,3]}}),
            json!({"error": {"code": -32601, "message": "not found"}}),
        ];

        for payload in payloads {
            let encoded = encode_frame(&payload).unwrap();
            let mut cursor = io::Cursor::new(encoded);
            let decoded = decode_frame(&mut cursor).unwrap();
            assert_eq!(payload, decoded);
        }
    }

    #[test]
    fn test_handle_request_ping() {
        let logger = Arc::new(crate::logger::Logger::new(
            100,
            crate::logger::LogLevel::Trace,
            false,
        ));
        logger.set_quiet(true);
        let config = crate::config::Config::default();
        let backend = Arc::new(crate::input_backend::MockBackend::new());
        let engine = Arc::new(Mutex::new(Engine::new(
            config,
            backend,
            logger.clone(),
            Arc::new(EventBus::new()),
            "test".into(),
        )));

        let request = json!({"jsonrpc": "2.0", "id": 1, "method": "ping"});
        let response = handle_request(&request, &engine, &logger);
        assert_eq!(response["result"], "pong");
    }

    #[test]
    fn test_handle_request_status() {
        let logger = Arc::new(crate::logger::Logger::new(
            100,
            crate::logger::LogLevel::Trace,
            false,
        ));
        logger.set_quiet(true);
        let config = crate::config::Config::default();
        let backend = Arc::new(crate::input_backend::MockBackend::new());
        let engine = Arc::new(Mutex::new(Engine::new(
            config,
            backend,
            logger.clone(),
            Arc::new(EventBus::new()),
            "test".into(),
        )));

        let request = json!({"jsonrpc": "2.0", "id": 1, "method": "status"});
        let response = handle_request(&request, &engine, &logger);
        let result = &response["result"];
        assert_eq!(result["enabled"], false);
    }

    #[test]
    fn test_handle_request_unknown_method() {
        let logger = Arc::new(crate::logger::Logger::new(
            100,
            crate::logger::LogLevel::Trace,
            false,
        ));
        logger.set_quiet(true);
        let config = crate::config::Config::default();
        let backend = Arc::new(crate::input_backend::MockBackend::new());
        let engine = Arc::new(Mutex::new(Engine::new(
            config,
            backend,
            logger.clone(),
            Arc::new(EventBus::new()),
            "test".into(),
        )));

        let request = json!({"jsonrpc": "2.0", "id": 1, "method": "nonexistent"});
        let response = handle_request(&request, &engine, &logger);
        assert_eq!(response["error"]["code"], -32601);
    }

    #[test]
    fn test_handle_request_trigger_unknown_id() {
        let logger = Arc::new(crate::logger::Logger::new(
            100,
            crate::logger::LogLevel::Trace,
            false,
        ));
        logger.set_quiet(true);
        let config = crate::config::Config::default();
        let backend = Arc::new(crate::input_backend::MockBackend::new());
        let engine = Arc::new(Mutex::new(Engine::new(
            config,
            backend,
            logger.clone(),
            Arc::new(EventBus::new()),
            "test".into(),
        )));
        with_engine_events(&engine, |eng| eng.set_enabled(true));

        let request =
            json!({"jsonrpc": "2.0", "id": 1, "method": "trigger", "params": {"id": "nope"}});
        let response = handle_request(&request, &engine, &logger);
        assert!(response.get("error").is_some());
    }

    #[test]
    fn test_handle_request_toggle() {
        let logger = Arc::new(crate::logger::Logger::new(
            100,
            crate::logger::LogLevel::Trace,
            false,
        ));
        logger.set_quiet(true);
        let config = crate::config::Config::default();
        let backend = Arc::new(crate::input_backend::MockBackend::new());
        let engine = Arc::new(Mutex::new(Engine::new(
            config,
            backend,
            logger.clone(),
            Arc::new(EventBus::new()),
            "test".into(),
        )));

        let request = json!({"jsonrpc": "2.0", "id": 1, "method": "toggle"});
        let response = handle_request(&request, &engine, &logger);
        assert_eq!(response["result"]["enabled"], true);

        let response = handle_request(&request, &engine, &logger);
        assert_eq!(response["result"]["enabled"], false);
    }

    #[test]
    fn test_handle_request_logs_tail() {
        let logger = Arc::new(crate::logger::Logger::new(
            100,
            crate::logger::LogLevel::Trace,
            false,
        ));
        logger.set_quiet(true);
        logger.info("test message 1");
        logger.info("test message 2");

        let config = crate::config::Config::default();
        let backend = Arc::new(crate::input_backend::MockBackend::new());
        let engine = Arc::new(Mutex::new(Engine::new(
            config,
            backend,
            logger.clone(),
            Arc::new(EventBus::new()),
            "test".into(),
        )));

        let request =
            json!({"jsonrpc": "2.0", "id": 1, "method": "logs_tail", "params": {"n": 10}});
        let response = handle_request(&request, &engine, &logger);
        let logs = response["result"].as_array().unwrap();
        assert!(logs.len() >= 2);
    }

    #[test]
    fn test_concurrent_clients() {
        let logger = Arc::new(crate::logger::Logger::new(
            100,
            crate::logger::LogLevel::Trace,
            false,
        ));
        logger.set_quiet(true);
        let config = crate::config::Config::default();
        let backend = Arc::new(crate::input_backend::MockBackend::new());
        let engine = Arc::new(Mutex::new(Engine::new(
            config,
            backend,
            logger.clone(),
            Arc::new(EventBus::new()),
            "test".into(),
        )));

        let dir = tempfile::tempdir().unwrap();
        let socket_path = dir.path().join("test.sock");

        let server = IpcServer::new(socket_path.clone(), engine, logger, Arc::new(EventBus::new())).unwrap();
        let shutdown = server.shutdown_flag();

        let server_handle = thread::spawn(move || {
            server.run();
        });

        // Wait for server to start
        thread::sleep(Duration::from_millis(100));

        // Launch 10 concurrent clients
        let mut handles = Vec::new();
        for _ in 0..10 {
            let path = socket_path.clone();
            handles.push(thread::spawn(move || {
                let response = ipc_request(&path, "ping", None).unwrap();
                assert_eq!(response["result"], "pong");
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        shutdown.store(true, std::sync::atomic::Ordering::Relaxed);
        server_handle.join().unwrap();
    }
}
