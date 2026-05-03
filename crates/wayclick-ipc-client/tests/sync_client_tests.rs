// SPDX-License-Identifier: MIT
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::thread;
use wayclick_ipc_client::SyncClient;

/// Spawn a one-shot mock server on a temp socket. Returns (path, join handle).
/// Server reads one frame, then writes `reply`. The handle yields the request received.
fn spawn_mock_server(reply: Value) -> (PathBuf, thread::JoinHandle<Value>) {
    let dir = std::env::temp_dir();
    let pid = std::process::id();
    let nonce: u32 = rand_nonce();
    let path = dir.join(format!("wayclick-test-{pid}-{nonce}.sock"));
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).unwrap();
    let server_path = path.clone();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();

        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf).unwrap();
        let len = u32::from_be_bytes(len_buf) as usize;
        let mut payload = vec![0u8; len];
        stream.read_exact(&mut payload).unwrap();
        let received: Value = serde_json::from_slice(&payload).unwrap();

        let bytes = serde_json::to_vec(&reply).unwrap();
        let len = (bytes.len() as u32).to_be_bytes();
        stream.write_all(&len).unwrap();
        stream.write_all(&bytes).unwrap();
        stream.flush().unwrap();

        let _ = std::fs::remove_file(&server_path);
        received
    });
    (path, handle)
}

fn rand_nonce() -> u32 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .subsec_nanos()
}

#[test]
fn sync_request_roundtrip() {
    let reply = json!({"jsonrpc": "2.0", "id": 1, "result": "pong"});
    let (path, handle) = spawn_mock_server(reply.clone());
    let response = SyncClient::request(&path, "ping", None).unwrap();
    let received_request = handle.join().unwrap();

    assert_eq!(response, reply);
    assert_eq!(received_request["method"], "ping");
    assert_eq!(received_request["id"], 1);
    assert_eq!(received_request["jsonrpc"], "2.0");
}

#[test]
fn sync_request_passes_params() {
    let reply = json!({"jsonrpc": "2.0", "id": 1, "result": null});
    let (path, handle) = spawn_mock_server(reply);
    let params = json!({"id": "trig1", "press": true});
    let _ = SyncClient::request(&path, "trigger", Some(params.clone())).unwrap();
    let received = handle.join().unwrap();
    assert_eq!(received["params"], params);
}

#[test]
fn connect_with_timeout_succeeds_against_listener() {
    use std::os::unix::net::UnixListener;
    use std::time::SystemTime;
    let dir = std::env::temp_dir();
    let nonce = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .subsec_nanos();
    let path = dir.join(format!("wayclick-connect-test-{}.sock", nonce));
    let _ = std::fs::remove_file(&path);
    let _listener = UnixListener::bind(&path).unwrap();

    let stream = wayclick_ipc_client::connect_with_timeout(&path, 1000)
        .expect("should connect to live listener");
    drop(stream);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn connect_with_timeout_returns_io_error_when_socket_missing() {
    let path = std::path::PathBuf::from("/tmp/wayclick-connect-nonexistent-test.sock");
    let _ = std::fs::remove_file(&path);
    let result = wayclick_ipc_client::connect_with_timeout(&path, 1000);
    assert!(matches!(
        result,
        Err(wayclick_ipc_client::frame::IpcError::Io(_))
    ));
}

#[test]
fn connect_with_timeout_zero_means_no_timeout() {
    use std::os::unix::net::UnixListener;
    use std::time::SystemTime;
    let dir = std::env::temp_dir();
    let nonce = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .subsec_nanos();
    let path = dir.join(format!("wayclick-connect-zero-{}.sock", nonce));
    let _ = std::fs::remove_file(&path);
    let _listener = UnixListener::bind(&path).unwrap();

    let stream = wayclick_ipc_client::connect_with_timeout(&path, 0)
        .expect("zero timeout should not be rejected");
    assert_eq!(stream.read_timeout().unwrap(), None);
    assert_eq!(stream.write_timeout().unwrap(), None);
    drop(stream);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn sync_request_returns_io_error_when_socket_missing() {
    let path = PathBuf::from("/tmp/wayclick-nonexistent-test.sock");
    let _ = std::fs::remove_file(&path);
    let result = SyncClient::request(&path, "ping", None);
    assert!(matches!(
        result,
        Err(wayclick_ipc_client::frame::IpcError::Io(_))
    ));
}
