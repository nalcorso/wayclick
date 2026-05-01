use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tempfile::TempDir;

use wayclick_core::config::Config;
use wayclick_core::engine::Engine;
use wayclick_core::event_bus::EventBus;
use wayclick_core::input_backend::{BackendCall, InputBackend, MockBackend};
use wayclick_core::ipc::{decode_frame, ipc_request, write_frame, IpcServer};
use wayclick_core::logger::{LogLevel, Logger};

pub struct TestDaemon {
    pub engine: Arc<Mutex<Engine>>,
    pub logger: Arc<Logger>,
    pub socket_path: PathBuf,
    pub backend_calls: Arc<Mutex<Vec<BackendCall>>>,
    shutdown: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
    _dir: TempDir,
}

impl TestDaemon {
    pub fn new(config: Config) -> Self {
        let logger = Arc::new(Logger::new(100, LogLevel::Trace, false));
        logger.set_quiet(true);

        let event_bus = Arc::new(EventBus::new());

        // Capture backend_calls BEFORE erasing the type to Arc<dyn InputBackend>.
        let mock = MockBackend::new();
        let backend_calls = mock.calls_clone();
        let backend: Arc<dyn InputBackend> = Arc::new(mock);

        let engine = Arc::new(Mutex::new(Engine::new(
            config,
            backend,
            logger.clone(),
            event_bus.clone(),
            "test".into(),
        )));

        let dir = tempfile::tempdir().unwrap();
        let socket_path = dir.path().join("test.sock");

        let server = IpcServer::new(socket_path.clone(), engine.clone(), logger.clone(), event_bus, None)
            .unwrap();
        let shutdown = server.shutdown_flag();

        let handle = thread::spawn(move || {
            server.run();
        });

        Self {
            engine,
            logger,
            socket_path,
            backend_calls,
            shutdown,
            handle: Some(handle),
            _dir: dir,
        }
    }

    pub fn teardown(mut self) {
        self.do_shutdown();
    }

    fn do_shutdown(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }

    /// Single-shot IPC call — opens a new connection, sends the request, returns the response.
    pub fn ipc(&self, method: &str, params: Option<Value>) -> Value {
        ipc_request(&self.socket_path, method, params).expect("ipc_request failed")
    }

    /// Open a persistent UnixStream with a 5-second read timeout.
    pub fn connect(&self) -> UnixStream {
        let sock = UnixStream::connect(&self.socket_path).unwrap();
        sock.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        sock
    }
}

impl Drop for TestDaemon {
    fn drop(&mut self) {
        self.do_shutdown();
    }
}

/// Poll `f()` every 10 ms until it returns `true` or `timeout` elapses.
/// Returns `true` if the condition was met, `false` on timeout.
pub fn poll_until(timeout: Duration, mut f: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if f() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(10));
    }
}

/// Send a single framed JSON-RPC request on `sock` and return the decoded response.
pub fn ipc_call_raw(sock: &mut UnixStream, id: u64, method: &str, params: Value) -> Value {
    let req = json!({"jsonrpc":"2.0","id":id,"method":method,"params":params});
    write_frame(sock, &req).unwrap();
    decode_frame(sock).unwrap()
}

/// Drain frames from `sock` until a frame matching `event_type` arrives or `timeout` elapses.
/// Distinguishes event frames (method == "event") from response frames (non-null id) by shape.
/// Returns the event `params` on match, or `None` on timeout or connection close.
pub fn wait_for_event(sock: &mut UnixStream, event_type: &str, timeout: Duration) -> Option<Value> {
    let deadline = Instant::now() + timeout;
    loop {
        let remaining = match deadline.checked_duration_since(Instant::now()) {
            Some(r) if r > Duration::ZERO => r,
            _ => return None,
        };
        sock.set_read_timeout(Some(remaining.max(Duration::from_millis(1)))).unwrap();
        match decode_frame(sock) {
            Ok(msg) => {
                if msg.get("method").and_then(|v| v.as_str()) == Some("event") {
                    if msg["params"]["type"].as_str() == Some(event_type) {
                        return Some(msg["params"].clone());
                    }
                    // Different event type — keep draining
                }
                // Response frame (non-null id) — keep draining
            }
            Err(_) => return None,
        }
    }
}
