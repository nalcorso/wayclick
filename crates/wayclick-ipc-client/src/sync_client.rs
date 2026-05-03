// SPDX-License-Identifier: MIT
//! Blocking request/response client.
//!
//! Use [`SyncClient::request`] for one-shot calls (CLI tools, TUI status polls).
//! For event subscriptions or long-lived streaming, use the async client instead.

use crate::frame::{decode_frame, write_frame, IpcError};
use serde_json::{json, Value};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

/// Default read/write timeout used by [`SyncClient::request`]: 5 seconds.
const DEFAULT_TIMEOUT_MS: u64 = 5000;

/// Stateless namespace for blocking calls. The daemon does not require a
/// persistent connection for simple request/response — each call connects,
/// sends, receives, and disconnects.
pub struct SyncClient;

impl SyncClient {
    /// Send a single JSON-RPC request and return the parsed response.
    ///
    /// The request id is fixed at `1`. If you need event streaming or
    /// pipelined requests, use [`crate::async_client::AsyncClient`].
    pub fn request(
        socket_path: &Path,
        method: &str,
        params: Option<Value>,
    ) -> Result<Value, IpcError> {
        Self::with_timeout(socket_path, method, params, DEFAULT_TIMEOUT_MS)
    }

    /// Like [`request`](Self::request) but with a configurable read/write timeout in milliseconds.
    pub fn with_timeout(
        socket_path: &Path,
        method: &str,
        params: Option<Value>,
        timeout_ms: u64,
    ) -> Result<Value, IpcError> {
        let mut stream = UnixStream::connect(socket_path)?;
        let timeout = Duration::from_millis(timeout_ms);
        stream.set_read_timeout(Some(timeout))?;
        stream.set_write_timeout(Some(timeout))?;

        let request = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params.unwrap_or(json!(null)),
        });

        write_frame(&mut stream, &request)?;
        decode_frame(&mut stream)
    }
}

/// Connect to a daemon socket with read/write timeouts configured.
///
/// Use this for streaming clients that need to issue ad-hoc requests and
/// read events on a long-lived connection — for example, hand-rolled
/// subscribe loops that don't fit the [`AsyncClient`](crate::AsyncClient)
/// handshake. Most callers want [`SyncClient::request`] or [`AsyncClient`] instead.
///
/// `timeout_ms` is applied to both reads and writes. A value of `0` means
/// "no timeout" — the stream is connected without any timeout configured,
/// suitable for callers that will switch the stream to non-blocking mode
/// or set their own timeouts afterward. For non-zero values the call
/// returns an error if the OS rejects the duration.
pub fn connect_with_timeout(socket_path: &Path, timeout_ms: u64) -> Result<UnixStream, IpcError> {
    let stream = UnixStream::connect(socket_path)?;
    if timeout_ms != 0 {
        let timeout = Duration::from_millis(timeout_ms);
        stream.set_read_timeout(Some(timeout))?;
        stream.set_write_timeout(Some(timeout))?;
    }
    Ok(stream)
}
