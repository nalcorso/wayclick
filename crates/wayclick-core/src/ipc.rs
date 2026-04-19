use crate::engine::Engine;
use crate::logger::Logger;
use serde_json::{json, Value};
use std::io::{self, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
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
            let mut engine = engine.lock().unwrap();
            let new_state = engine.toggle_enabled();
            make_response(id, json!({ "enabled": new_state }))
        }

        "enable" => {
            let mut engine = engine.lock().unwrap();
            engine.set_enabled(true);
            make_response(id, json!({ "enabled": true }))
        }

        "disable" => {
            let mut engine = engine.lock().unwrap();
            engine.set_enabled(false);
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

            let mut engine = engine.lock().unwrap();
            match engine.trigger_event(trigger_id, press) {
                Ok(()) => make_response(id, json!({ "triggered": trigger_id })),
                Err(e) => make_error(id, -32000, &e.to_string()),
            }
        }

        "list_triggers" => {
            let engine = engine.lock().unwrap();
            let triggers = engine.triggers_snapshot();
            make_response(id, serde_json::to_value(&triggers).unwrap_or(json!([])))
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

            let mut engine = engine.lock().unwrap();
            engine.set_layer(layer.to_string());
            make_response(id, json!({ "layer": layer }))
        }

        "get_layer" => {
            let engine = engine.lock().unwrap();
            make_response(id, json!({ "layer": engine.current_layer() }))
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

pub struct IpcServer {
    socket_path: PathBuf,
    listener: Option<UnixListener>,
    engine: Arc<Mutex<Engine>>,
    logger: Arc<Logger>,
    shutdown: Arc<std::sync::atomic::AtomicBool>,
    connection_count: Arc<AtomicUsize>,
}

impl IpcServer {
    pub fn new(
        socket_path: PathBuf,
        engine: Arc<Mutex<Engine>>,
        logger: Arc<Logger>,
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
            shutdown: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            connection_count: Arc::new(AtomicUsize::new(0)),
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
                    let guard = ConnectionGuard {
                        counter: self.connection_count.clone(),
                    };
                    thread::spawn(move || {
                        let _guard = guard;
                        handle_client(stream, engine, logger);
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

fn handle_client(mut stream: UnixStream, engine: Arc<Mutex<Engine>>, logger: Arc<Logger>) {
    stream.set_nonblocking(false).unwrap_or_default();
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap_or_default();

    loop {
        match decode_frame(&mut stream) {
            Ok(request) => {
                let response = handle_request(&request, &engine, &logger);
                if let Err(e) = write_frame(&mut stream, &response) {
                    logger.debug(format!("IPC write error: {}", e));
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
}

/// Client-side helper: connect to daemon and send a single request, return response.
pub fn ipc_request(
    socket_path: &Path,
    method: &str,
    params: Option<Value>,
) -> Result<Value, IpcError> {
    let mut stream = UnixStream::connect(socket_path)?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;

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
            "test".into(),
        )));
        engine.lock().unwrap().set_enabled(true);

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
            "test".into(),
        )));

        let dir = tempfile::tempdir().unwrap();
        let socket_path = dir.path().join("test.sock");

        let server = IpcServer::new(socket_path.clone(), engine, logger).unwrap();
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
