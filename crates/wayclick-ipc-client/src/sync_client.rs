// SPDX-License-Identifier: MIT
//! Blocking request/response client.
//!
//! Use [`SyncClient::request`] for one-shot calls (CLI tools, TUI status polls).
//! For event subscriptions or long-lived streaming, use the async client instead.

use crate::frame::{decode_frame, write_frame, IpcError};
use crate::types::{CursorPosition, MonitorInfo};
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

    /// Typed helper: query the daemon for the current cursor position.
    ///
    /// Returns:
    /// - `Ok(Some(pos))` when the backend reported coordinates.
    /// - `Ok(None)` when the daemon's focus-tracker backend does not
    ///   support cursor reporting (JSON-RPC error code `-32001`,
    ///   *Unsupported*) or the position is momentarily unavailable
    ///   (`-32000`, *Internal error*).
    /// - `Err(IpcError::Rpc { .. })` for any other JSON-RPC error code.
    /// - `Err(IpcError::MalformedResponse(..))` when the daemon response
    ///   has neither `result` nor `error`.
    /// - `Err(IpcError::..)` on transport / framing failures.
    ///
    /// Callers should treat `Ok(None)` as "cursor reporting unavailable"
    /// (best-effort capability) and propagate the error variants.
    pub fn get_cursor_position(socket_path: &Path) -> Result<Option<CursorPosition>, IpcError> {
        let response = Self::request(socket_path, "get_cursor_position", None)?;
        match classify_rpc_response(&response) {
            RpcOutcome::Result(result) => {
                let pos: CursorPosition =
                    serde_json::from_value(result.clone()).map_err(IpcError::Json)?;
                Ok(Some(pos))
            }
            RpcOutcome::Unavailable => Ok(None),
            RpcOutcome::Error { code, message } => Err(IpcError::Rpc { code, message }),
            RpcOutcome::Malformed => Err(IpcError::MalformedResponse(response.to_string())),
        }
    }

    /// Convenience wrapper for `get_monitors`.
    ///
    /// Returns:
    /// - `Ok(Some(monitors))` when the backend reported a layout (possibly empty).
    /// - `Ok(None)` when the daemon's focus-tracker backend does not
    ///   support monitor reporting (JSON-RPC error code `-32001`) or the
    ///   layout is momentarily unavailable (`-32000`).
    /// - `Err(IpcError::Rpc { .. })` for any other JSON-RPC error code.
    /// - `Err(IpcError::MalformedResponse(..))` when the daemon response
    ///   has neither `result` nor `error`.
    /// - `Err(IpcError::..)` on transport / framing failures.
    ///
    /// The recorder uses this to classify each click into a monitor and
    /// emit monitor-local coordinates.
    pub fn get_monitors(socket_path: &Path) -> Result<Option<Vec<MonitorInfo>>, IpcError> {
        let response = Self::request(socket_path, "get_monitors", None)?;
        match classify_rpc_response(&response) {
            RpcOutcome::Result(result) => {
                let arr = result
                    .get("monitors")
                    .cloned()
                    .unwrap_or(serde_json::Value::Array(Vec::new()));
                let monitors: Vec<MonitorInfo> =
                    serde_json::from_value(arr).map_err(IpcError::Json)?;
                Ok(Some(monitors))
            }
            RpcOutcome::Unavailable => Ok(None),
            RpcOutcome::Error { code, message } => Err(IpcError::Rpc { code, message }),
            RpcOutcome::Malformed => Err(IpcError::MalformedResponse(response.to_string())),
        }
    }
}

/// JSON-RPC server-error codes used by wayclickd. Mirrors the constants in
/// `wayclick-core::ipc` (kept here to avoid the dependency direction reversal).
const JSONRPC_INTERNAL_ERROR: i32 = -32000;
const JSONRPC_UNSUPPORTED: i32 = -32001;

enum RpcOutcome<'a> {
    Result(&'a Value),
    /// Server returned an error that callers should treat as "feature
    /// unavailable" (`-32001` Unsupported or `-32000` Internal error /
    /// transiently unavailable).
    Unavailable,
    Error {
        code: i32,
        message: String,
    },
    Malformed,
}

fn classify_rpc_response(response: &Value) -> RpcOutcome<'_> {
    if let Some(result) = response.get("result") {
        return RpcOutcome::Result(result);
    }
    if let Some(err) = response.get("error") {
        let code = err.get("code").and_then(|c| c.as_i64()).unwrap_or(0) as i32;
        let message = err
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("")
            .to_string();
        return match code {
            JSONRPC_UNSUPPORTED | JSONRPC_INTERNAL_ERROR => RpcOutcome::Unavailable,
            _ => RpcOutcome::Error { code, message },
        };
    }
    RpcOutcome::Malformed
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
