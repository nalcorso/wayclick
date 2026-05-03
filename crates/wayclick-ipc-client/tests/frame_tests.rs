// SPDX-License-Identifier: MIT
use serde_json::json;
use std::io::Cursor;
use wayclick_ipc_client::frame::{decode_frame, encode_frame, IpcError, MAX_FRAME_SIZE};

#[test]
fn encode_decode_roundtrip_simple() {
    let payload = json!({"jsonrpc": "2.0", "id": 1, "method": "ping"});
    let encoded = encode_frame(&payload).unwrap();
    let mut cursor = Cursor::new(encoded);
    let decoded = decode_frame(&mut cursor).unwrap();
    assert_eq!(payload, decoded);
}

#[test]
fn encode_decode_roundtrip_varied_payloads() {
    let payloads = vec![
        json!(null),
        json!({"method": "status"}),
        json!({"result": {"enabled": true, "triggers": [1,2,3]}}),
        json!({"error": {"code": -32601, "message": "not found"}}),
        json!([1, 2, 3, "four", null, {"nested": true}]),
    ];
    for payload in payloads {
        let encoded = encode_frame(&payload).unwrap();
        let mut cursor = Cursor::new(encoded);
        let decoded = decode_frame(&mut cursor).unwrap();
        assert_eq!(payload, decoded);
    }
}

#[test]
fn encode_rejects_oversized_frame() {
    // Build a string that, after JSON-encoding, exceeds MAX_FRAME_SIZE.
    let big = "x".repeat((MAX_FRAME_SIZE as usize) + 100);
    let payload = json!(big);
    match encode_frame(&payload) {
        Err(IpcError::FrameTooLarge(n)) => assert!(n > MAX_FRAME_SIZE),
        other => panic!("expected FrameTooLarge, got {:?}", other),
    }
}

#[test]
fn decode_rejects_oversized_length_prefix() {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&(MAX_FRAME_SIZE + 1).to_be_bytes());
    let mut cursor = Cursor::new(bytes);
    match decode_frame(&mut cursor) {
        Err(IpcError::FrameTooLarge(n)) => assert_eq!(n, MAX_FRAME_SIZE + 1),
        other => panic!("expected FrameTooLarge, got {:?}", other),
    }
}

#[test]
fn decode_returns_connection_closed_on_eof() {
    let mut cursor = Cursor::new(Vec::<u8>::new());
    assert!(matches!(
        decode_frame(&mut cursor),
        Err(IpcError::ConnectionClosed)
    ));
}

#[test]
fn decode_returns_io_error_on_truncated_payload() {
    // Length prefix says 100 bytes but only 4 available
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&100u32.to_be_bytes());
    bytes.extend_from_slice(b"only");
    let mut cursor = Cursor::new(bytes);
    assert!(matches!(decode_frame(&mut cursor), Err(IpcError::Io(_))));
}
