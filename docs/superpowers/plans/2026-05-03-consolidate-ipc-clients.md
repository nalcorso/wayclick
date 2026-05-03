# Consolidate IPC Clients into `wayclick-ipc-client` Crate

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extract the duplicated IPC client logic from `wayclick-core` and `extras/wayclick-playground` into a single dependency-light reusable crate `wayclick-ipc-client`, then migrate the TUI and playground to consume it.

**Architecture:** New library crate at `crates/wayclick-ipc-client` with five modules — `frame` (length-prefixed JSON-RPC framing), `socket` (XDG/proc-based path resolution), `types` (typed `ServiceStatus` / `TriggerInfo` / `FocusedWindow`), `sync_client` (blocking request/response), `async_client` (background-thread streaming with channel API). `wayclick-core` keeps the daemon-side server logic but depends on the new crate for the frame primitives — single source of truth. TUI swaps `ipc::ipc_request` → `SyncClient::request`. Playground deletes its local `ipc_client.rs` and uses `AsyncClient` directly.

**Tech Stack:** Rust 2021, MSRV 1.85.0, dependencies limited to `serde`, `serde_json`, `thiserror`. Background thread uses `std::thread` + `std::sync::mpsc` (no async runtime). All sockets are Unix domain sockets via `std::os::unix::net::UnixStream`.

---

## Decisions Locked In (read before starting)

These were settled during planning. Don't relitigate:

1. **`IpcCommand` is generic only.** `enum IpcCommand { Send(Value), Shutdown }`. The playground's `FireTrigger`/`EnableTrigger`/`DisableTrigger`/`RefreshTriggers` variants do **not** move into the library — they become `client.send("trigger", Some(json!({...})))` call sites in `app_state.rs`. This is a real but mechanical migration.
2. **`IpcMessage` is typed and wayclick-specific.** `Connected`, `Disconnected`, `TriggerActivated`, `TriggerDeactivated`, `RawInput`, `LayerChanged`, `EnabledChanged`, `ConfigReloaded`, `TriggerListUpdated`, `FocusChanged`, `ScrollReceived`. Same variants the playground uses today.
3. **`wayclick-core` will depend on `wayclick-ipc-client`.** This is the standard layered direction — only the *new* crate must avoid depending on core (so external consumers don't pull in the daemon). Core's server-side IPC code calls into `wayclick_ipc_client::frame::*` for encode/decode.
4. **No backward-compat re-exports from `wayclick-core::ipc` for client functions.** TUI is the only in-tree consumer and it's migrating in this same plan. Nothing external depends on the old API. Just delete `ipc_request` / `ipc_connect` after TUI migrates.
5. **Socket path resolution** in the new crate uses the playground's `/proc/self/status` UID-fallback approach (not `nix::unistd::getuid()`) to keep the dependency footprint at three crates. Result is identical on a properly configured Linux system; both `XDG_RUNTIME_DIR` and `/tmp/wayclick-{uid}.sock` paths match what the daemon writes.
6. **No documentation expansion.** Module-level rustdoc on each `lib.rs` module is sufficient; `docs/IPC.md` already exists and gets one short addition pointing at the new crate. Spec's "standalone protocol doc" is YAGNI.
7. **AsyncClient handshake matches today's playground.** It performs: `status` → `list_triggers` → `subscribe` → `get_focus` and emits `IpcMessage::Connected { status, triggers, initial_focus }` on success. Same keepalive ping (20s), same reconnect-on-drop behavior, same in-flight `list_triggers` ID tracking on `config_reloaded`.
8. **Spec mistake about playground depending on `wayclick-core`** — it doesn't; verified in `extras/wayclick-playground/Cargo.toml`. Just add the new crate's dep, don't try to remove a non-existent one.

---

## File Structure

### New files

```
crates/wayclick-ipc-client/
├── Cargo.toml                  — minimal deps (serde, serde_json, thiserror)
├── src/
│   ├── lib.rs                  — module declarations + public re-exports + crate-level rustdoc
│   ├── frame.rs                — encode_frame, decode_frame, write_frame, read_frame, IpcError, MAX_FRAME_SIZE
│   ├── socket.rs               — default_socket_path, socket_path_for_user, read_proc_uid
│   ├── types.rs                — ServiceStatus, TriggerInfo, FocusedWindow (Serialize + Deserialize)
│   ├── sync_client.rs          — SyncClient struct + request, with_timeout
│   └── async_client.rs         — AsyncClient struct + IpcMessage, IpcCommand, connect, send, send_json, recv, try_recv, shutdown
└── tests/
    ├── frame_tests.rs          — encode/decode roundtrip, oversized rejection, partial frames
    ├── socket_tests.rs         — XDG_RUNTIME_DIR set, fallback, UID parsing
    └── sync_client_tests.rs    — mock-server roundtrip + timeout behavior
```

### Modified files

- `Cargo.toml` (workspace root) — add `crates/wayclick-ipc-client` to members; add `wayclick-ipc-client` to workspace deps
- `crates/wayclick-core/Cargo.toml` — add `wayclick-ipc-client` dep
- `crates/wayclick-core/src/ipc.rs` — delete `IpcError`, `MAX_FRAME_SIZE`, `encode_frame`, `decode_frame`, `write_frame`, `ipc_connect`, `ipc_request`, and the `test_frame_*` tests. Replace internal callers with `wayclick_ipc_client::frame::{encode_frame, decode_frame, write_frame, IpcError}`.
- `crates/wayclick-core/benches/ipc_framing.rs` — change `use wayclick_core::ipc::{decode_frame, encode_frame};` to `use wayclick_ipc_client::frame::{decode_frame, encode_frame};`. Add `wayclick-ipc-client` as a dev-dependency on core.
- `crates/wayclick-tui/Cargo.toml` — drop `wayclick-core`, add `wayclick-ipc-client`
- `crates/wayclick-tui/src/main.rs` — replace 6 `ipc::ipc_request(...)` call sites; replace `wayclick_core::config::default_socket_path()` with `wayclick_ipc_client::socket::default_socket_path()`
- `extras/wayclick-playground/Cargo.toml` — add `wayclick-ipc-client` dep
- `extras/wayclick-playground/src/main.rs` — `mod ipc_client;` → remove; `use ipc_client::spawn_ipc_thread;` → `use wayclick_ipc_client::AsyncClient;`; spawn call → `AsyncClient::connect(...)`
- `extras/wayclick-playground/src/app_state.rs` — switch to `use wayclick_ipc_client::{AsyncClient, FocusedWindow, IpcMessage, ServiceStatus, TriggerInfo};` and rewrite `IpcCommand::*` send sites to `client.send(method, Some(params))`
- `extras/wayclick-playground/src/ui/mod.rs` — change `use crate::ipc_client::FocusedWindow;` to `use wayclick_ipc_client::FocusedWindow;`

### Deleted files

- `extras/wayclick-playground/src/ipc_client.rs` (556 lines, replaced by library)

---

## Phase 1 — Crate Skeleton + Frame Module

### Task 1: Create the crate and register it in the workspace

**Files:**
- Create: `crates/wayclick-ipc-client/Cargo.toml`
- Create: `crates/wayclick-ipc-client/src/lib.rs`
- Modify: `Cargo.toml` (workspace root)

- [ ] **Step 1: Create the crate directory and Cargo.toml**

Write `crates/wayclick-ipc-client/Cargo.toml`:

```toml
[package]
name = "wayclick-ipc-client"
description = "Reusable IPC client for the wayclick daemon (frame protocol, sync + async clients)"
version.workspace = true
edition.workspace = true
license.workspace = true
repository.workspace = true
homepage.workspace = true
rust-version.workspace = true

[dependencies]
serde = { workspace = true }
serde_json = { workspace = true }
thiserror = { workspace = true }
```

- [ ] **Step 2: Create a placeholder `src/lib.rs`**

Write `crates/wayclick-ipc-client/src/lib.rs`:

```rust
// SPDX-License-Identifier: MIT
//! Reusable IPC client for the wayclick daemon.
//!
//! Provides:
//! - [`frame`] — length-prefixed JSON-RPC 2.0 frame encoder/decoder
//! - [`socket`] — daemon socket path resolution (XDG_RUNTIME_DIR + UID fallback)
//! - [`types`] — typed deserialization for status/trigger/focus responses
//! - [`sync_client`] — blocking request/response client (use this for CLIs and TUIs)
//! - [`async_client`] — background-thread streaming client (use this for event-driven apps)

pub mod frame;
```

- [ ] **Step 3: Register in workspace and add workspace dep**

In the root `Cargo.toml`, add `"crates/wayclick-ipc-client"` to `[workspace] members` (insert alphabetically near the other crates). Under `[workspace.dependencies]`, add:

```toml
wayclick-ipc-client = { path = "crates/wayclick-ipc-client" }
```

- [ ] **Step 4: Verify the empty crate builds**

Run: `cargo build -p wayclick-ipc-client`
Expected: `Compiling wayclick-ipc-client v0.1.0 ...` then `Finished` with zero warnings.

The build will fail because `frame` module doesn't exist yet. Move on to Task 2 to create it; commit after Task 2.

---

### Task 2: Port the frame module from `wayclick-core/src/ipc.rs`

**Files:**
- Create: `crates/wayclick-ipc-client/src/frame.rs`
- Test: `crates/wayclick-ipc-client/tests/frame_tests.rs`

- [ ] **Step 1: Write `frame.rs` byte-for-byte equivalent to the original**

Write `crates/wayclick-ipc-client/src/frame.rs`:

```rust
// SPDX-License-Identifier: MIT
//! Length-prefixed JSON-RPC 2.0 frame protocol.
//!
//! Wire format: 4-byte big-endian length prefix + UTF-8 JSON payload.
//! Maximum frame size is [`MAX_FRAME_SIZE`] bytes; larger frames are rejected
//! to bound memory use against malicious or buggy peers.

use serde_json::Value;
use std::io::{self, Read, Write};
use thiserror::Error;

/// Maximum size of a single IPC frame in bytes.
pub const MAX_FRAME_SIZE: u32 = 65536;

/// Errors produced by frame encoding / decoding and clients built on top.
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

/// Encode a JSON value into a length-prefixed frame.
///
/// Returns [`IpcError::FrameTooLarge`] if the encoded JSON exceeds
/// [`MAX_FRAME_SIZE`] bytes.
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

/// Decode one length-prefixed frame from a reader.
///
/// Reads exactly `4 + length` bytes. Returns [`IpcError::ConnectionClosed`]
/// when the reader returns EOF before the length prefix is complete.
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

/// Encode `payload` and write the resulting frame to `writer`, flushing on success.
pub fn write_frame(writer: &mut impl Write, payload: &Value) -> Result<(), IpcError> {
    let frame = encode_frame(payload)?;
    writer.write_all(&frame)?;
    writer.flush()?;
    Ok(())
}

/// Convenience alias for [`decode_frame`]. Provided for symmetry with [`write_frame`].
pub fn read_frame(reader: &mut impl Read) -> Result<Value, IpcError> {
    decode_frame(reader)
}
```

- [ ] **Step 2: Update `lib.rs` to export the frame primitives**

Edit `crates/wayclick-ipc-client/src/lib.rs` so `pub mod frame;` is followed by:

```rust
pub use frame::{IpcError, MAX_FRAME_SIZE};
```

- [ ] **Step 3: Write the failing integration test for frame encode/decode**

Write `crates/wayclick-ipc-client/tests/frame_tests.rs`:

```rust
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
    bytes.extend_from_slice(&((MAX_FRAME_SIZE + 1) as u32).to_be_bytes());
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
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p wayclick-ipc-client`
Expected: 6 passed, 0 failed.

- [ ] **Step 5: Commit**

```bash
git add crates/wayclick-ipc-client Cargo.toml
git commit -m "feat(ipc-client): add wayclick-ipc-client crate with frame module"
```

---

## Phase 2 — Socket, Types, Sync Client

### Task 3: Implement the `socket` module

**Files:**
- Create: `crates/wayclick-ipc-client/src/socket.rs`
- Test: `crates/wayclick-ipc-client/tests/socket_tests.rs`
- Modify: `crates/wayclick-ipc-client/src/lib.rs`

- [ ] **Step 1: Write `socket.rs`**

Write `crates/wayclick-ipc-client/src/socket.rs`:

```rust
// SPDX-License-Identifier: MIT
//! Daemon socket path resolution.
//!
//! The wayclick daemon writes its Unix socket at `$XDG_RUNTIME_DIR/wayclick.sock`
//! when that env var is set, or `/tmp/wayclick-{uid}.sock` as a fallback.
//! Both daemon and clients agree on this convention.

use std::path::PathBuf;

/// Resolve the default daemon socket path.
///
/// Honors `XDG_RUNTIME_DIR` first, then falls back to `/tmp/wayclick-{uid}.sock`
/// where `uid` is read from `/proc/self/status` (1000 if that read fails).
pub fn default_socket_path() -> PathBuf {
    if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR") {
        if !dir.is_empty() {
            return PathBuf::from(dir).join("wayclick.sock");
        }
    }
    let uid = read_proc_uid().unwrap_or(1000);
    socket_path_for_user(uid)
}

/// Build the fallback `/tmp` socket path for a specific UID.
pub fn socket_path_for_user(uid: u32) -> PathBuf {
    PathBuf::from(format!("/tmp/wayclick-{uid}.sock"))
}

/// Read the real UID from `/proc/self/status`.
/// Returns `None` if the file can't be read or the line is missing.
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
```

- [ ] **Step 2: Re-export from lib.rs**

Add to `crates/wayclick-ipc-client/src/lib.rs`:

```rust
pub mod socket;
```

- [ ] **Step 3: Write integration tests**

Write `crates/wayclick-ipc-client/tests/socket_tests.rs`:

```rust
// SPDX-License-Identifier: MIT
use std::path::PathBuf;
use wayclick_ipc_client::socket::{default_socket_path, socket_path_for_user};

#[test]
fn socket_path_for_user_format() {
    assert_eq!(
        socket_path_for_user(1000),
        PathBuf::from("/tmp/wayclick-1000.sock")
    );
    assert_eq!(
        socket_path_for_user(0),
        PathBuf::from("/tmp/wayclick-0.sock")
    );
}

#[test]
fn default_uses_xdg_runtime_dir_when_set() {
    // Save & override; this test mutates env so it must not run alongside others
    // that read XDG_RUNTIME_DIR. Cargo runs each test binary in one process but
    // tests within a binary run in parallel — keep env mutation localized.
    let prev = std::env::var("XDG_RUNTIME_DIR").ok();
    std::env::set_var("XDG_RUNTIME_DIR", "/run/user/test-12345");
    let path = default_socket_path();
    assert_eq!(path, PathBuf::from("/run/user/test-12345/wayclick.sock"));
    match prev {
        Some(v) => std::env::set_var("XDG_RUNTIME_DIR", v),
        None => std::env::remove_var("XDG_RUNTIME_DIR"),
    }
}

#[test]
fn default_falls_back_to_tmp_when_xdg_unset() {
    let prev = std::env::var("XDG_RUNTIME_DIR").ok();
    std::env::remove_var("XDG_RUNTIME_DIR");
    let path = default_socket_path();
    let s = path.to_string_lossy();
    assert!(
        s.starts_with("/tmp/wayclick-") && s.ends_with(".sock"),
        "got {s}"
    );
    if let Some(v) = prev {
        std::env::set_var("XDG_RUNTIME_DIR", v);
    }
}
```

> **Note on test isolation:** The two `default_*` tests mutate `XDG_RUNTIME_DIR` and may race if run in parallel. If flakes appear, mark them `#[ignore]` and add a `#[test]` `default_socket_path_smoke()` that simply asserts the result is non-empty and has the `.sock` suffix. Don't try to fix this with a global mutex — the test surface is small.

- [ ] **Step 4: Run tests**

Run: `cargo test -p wayclick-ipc-client --test socket_tests`
Expected: 3 passed.

- [ ] **Step 5: Commit**

```bash
git add crates/wayclick-ipc-client/src/socket.rs crates/wayclick-ipc-client/src/lib.rs crates/wayclick-ipc-client/tests/socket_tests.rs
git commit -m "feat(ipc-client): add socket path resolution module"
```

---

### Task 4: Implement the `types` module

**Files:**
- Create: `crates/wayclick-ipc-client/src/types.rs`
- Modify: `crates/wayclick-ipc-client/src/lib.rs`

- [ ] **Step 1: Write `types.rs`**

Write `crates/wayclick-ipc-client/src/types.rs`:

```rust
// SPDX-License-Identifier: MIT
//! Typed views over JSON-RPC responses returned by the wayclick daemon.
//!
//! These types are loose Serde wrappers — fields use `#[serde(default)]`
//! and missing fields default rather than failing deserialization, so that
//! older or newer daemon versions remain partially compatible with this
//! client. Tighten this later if a stricter contract is wanted.

use serde::{Deserialize, Serialize};

/// Service-wide status returned by the `status` / `status_json` methods.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ServiceStatus {
    /// Whether the daemon is currently active (running triggers).
    pub enabled: bool,
    /// Total number of triggers configured.
    pub trigger_count: usize,
    /// Number of currently-active (held / latched) triggers.
    pub active_triggers: usize,
    /// Active layer name (e.g. `"default"`).
    pub layer: String,
    /// Daemon uptime in seconds.
    pub uptime_secs: u64,
    /// Whether dry-run mode is enabled (input is read but not synthesized).
    pub dry_run: bool,
}

/// A single trigger as reported by `list_triggers`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TriggerInfo {
    /// Stable identifier used to address the trigger over IPC.
    pub id: String,
    /// Human-friendly name from config.
    pub name: String,
    /// Trigger mode (e.g. `"oneshot"`, `"toggle"`, `"hold"`).
    pub mode: String,
    /// Whether the trigger is currently active.
    pub active: bool,
    /// Lifetime activation count.
    pub activate_count: u64,
    /// Whether the user has enabled this trigger (separate from runtime activation).
    pub user_enabled: bool,
    /// Whether the trigger was created at runtime via IPC (vs. from config).
    pub dynamic: bool,
}

/// Currently-focused window, as reported by `get_focus` and `focus_changed` events.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FocusedWindow {
    pub app_id: String,
    pub title: String,
    pub process_name: Option<String>,
    /// Backend that produced this focus information (e.g. `"hyprland"`).
    pub backend: String,
    /// True when the window is an XWayland surface.
    pub xwayland: bool,
}
```

- [ ] **Step 2: Re-export from lib.rs**

Add to `crates/wayclick-ipc-client/src/lib.rs`:

```rust
pub mod types;
pub use types::{FocusedWindow, ServiceStatus, TriggerInfo};
```

- [ ] **Step 3: Verify build**

Run: `cargo build -p wayclick-ipc-client`
Expected: Compiles with zero warnings.

- [ ] **Step 4: Commit**

```bash
git add crates/wayclick-ipc-client/src/types.rs crates/wayclick-ipc-client/src/lib.rs
git commit -m "feat(ipc-client): add typed ServiceStatus, TriggerInfo, FocusedWindow"
```

---

### Task 5: Implement the `sync_client` module

**Files:**
- Create: `crates/wayclick-ipc-client/src/sync_client.rs`
- Test: `crates/wayclick-ipc-client/tests/sync_client_tests.rs`
- Modify: `crates/wayclick-ipc-client/src/lib.rs`

- [ ] **Step 1: Write `sync_client.rs`**

Write `crates/wayclick-ipc-client/src/sync_client.rs`:

```rust
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
```

- [ ] **Step 2: Re-export from lib.rs**

Add to `crates/wayclick-ipc-client/src/lib.rs`:

```rust
pub mod sync_client;
pub use sync_client::SyncClient;
```

- [ ] **Step 3: Write integration test using a mock server**

Write `crates/wayclick-ipc-client/tests/sync_client_tests.rs`:

```rust
// SPDX-License-Identifier: MIT
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::thread;
use wayclick_ipc_client::SyncClient;

/// Spawn a one-shot mock server on a temp socket. Returns (path, join handle).
/// Server reads one frame, optionally inspects it via `expect`, then writes `reply`.
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

        // Read one frame
        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf).unwrap();
        let len = u32::from_be_bytes(len_buf) as usize;
        let mut payload = vec![0u8; len];
        stream.read_exact(&mut payload).unwrap();
        let received: Value = serde_json::from_slice(&payload).unwrap();

        // Write reply
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
fn sync_request_returns_io_error_when_socket_missing() {
    let path = PathBuf::from("/tmp/wayclick-nonexistent-test.sock");
    let _ = std::fs::remove_file(&path);
    let result = SyncClient::request(&path, "ping", None);
    assert!(matches!(
        result,
        Err(wayclick_ipc_client::frame::IpcError::Io(_))
    ));
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p wayclick-ipc-client`
Expected: All passing including the 3 new sync_client tests (12+ total).

- [ ] **Step 5: Commit**

```bash
git add crates/wayclick-ipc-client/src/sync_client.rs crates/wayclick-ipc-client/src/lib.rs crates/wayclick-ipc-client/tests/sync_client_tests.rs
git commit -m "feat(ipc-client): add SyncClient with mock-server tests"
```

---

## Phase 3 — Async Client

### Task 6: Implement the `async_client` module

The `async_client` is the largest task. It's a faithful port of `extras/wayclick-playground/src/ipc_client.rs` lines 316–556 with one API change: instead of `spawn_ipc_thread()` returning `(Receiver, Sender)`, expose `AsyncClient::connect(path) -> Result<AsyncClient, IpcError>` with `send(method, params)` and `recv` / `try_recv` methods. The internal command channel carries a private enum `Command { Send(Value), Shutdown }`; the public `IpcCommand` is just an alias.

**Files:**
- Create: `crates/wayclick-ipc-client/src/async_client.rs`
- Modify: `crates/wayclick-ipc-client/src/lib.rs`

- [ ] **Step 1: Write `async_client.rs`**

Write `crates/wayclick-ipc-client/src/async_client.rs`:

```rust
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
```

- [ ] **Step 2: Re-export from lib.rs**

Add to `crates/wayclick-ipc-client/src/lib.rs`:

```rust
pub mod async_client;
pub use async_client::{AsyncClient, IpcCommand, IpcMessage};
```

- [ ] **Step 3: Verify build with zero warnings**

Run: `cargo build -p wayclick-ipc-client`
Expected: `Finished` with no warnings.

Run: `cargo clippy -p wayclick-ipc-client --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add crates/wayclick-ipc-client/src/async_client.rs crates/wayclick-ipc-client/src/lib.rs
git commit -m "feat(ipc-client): add AsyncClient with background-thread streaming"
```

> **Why no integration test for AsyncClient at this stage:** an end-to-end test would need a mock server that handles the full handshake (status + list_triggers + subscribe + get_focus) plus event injection. That's significant test infrastructure for code that's a faithful port of already-shipping logic. Defer to manual smoke testing in Phase 5 against the real daemon. If a regression bites, *then* build the harness — don't pre-build it.

---

## Phase 4 — Wire It All Up

### Task 7: Migrate `wayclick-core` to depend on the new crate

**Files:**
- Modify: `crates/wayclick-core/Cargo.toml`
- Modify: `crates/wayclick-core/src/ipc.rs`
- Modify: `crates/wayclick-core/benches/ipc_framing.rs`

- [ ] **Step 1: Add the dependency**

Edit `crates/wayclick-core/Cargo.toml`. In `[dependencies]` add:

```toml
wayclick-ipc-client = { workspace = true }
```

Also add to `[dev-dependencies]` if there isn't already a `[dev-dependencies]` section, since the bench file imports it; if there *is* a `[dev-dependencies]` section, add `wayclick-ipc-client = { workspace = true }` there too — Cargo allows the same dep in both, and benches use dev-deps.

> **Quick check:** `grep -A3 dev-dependencies crates/wayclick-core/Cargo.toml`. If empty or absent, add the section.

- [ ] **Step 2: Strip the duplicated frame code from `ipc.rs`**

Edit `crates/wayclick-core/src/ipc.rs`:

Replace the existing `IpcError`, `MAX_FRAME_SIZE`, `encode_frame`, `decode_frame`, `write_frame` declarations (lines 29–82 in the current file) and the `ipc_connect` + `ipc_request` functions (lines 706–732) — delete them all.

At the top of the file, replace:
```rust
use std::io::{self, Read, Write};
```
with:
```rust
use std::io::{Read, Write};
use wayclick_ipc_client::frame::{decode_frame, write_frame, IpcError};
```

(Keep all other imports, including `std::os::unix::net::{UnixListener, UnixStream}`.)

Verify each remaining call site of `decode_frame`, `write_frame`, `IpcError` still resolves correctly via the new import. The current uses of `encode_frame` (if any) likewise come from the new crate.

- [ ] **Step 3: Delete the moved frame tests**

In the `#[cfg(test)] mod tests { ... }` block of `crates/wayclick-core/src/ipc.rs`, delete `test_frame_encode_decode` and `test_frame_encode_decode_roundtrip` (lines 759–784 in the original). Keep all other tests — they exercise server-side request handling and stay relevant.

Also delete the test (around line 893) that calls `ipc_request(&path, "ping", None)` if it exists — `ipc_request` no longer exists. If the test was testing the round-trip via the server, rewrite it to use `wayclick_ipc_client::SyncClient::request(&path, "ping", None).unwrap()` instead.

> **Search to confirm:** `grep -n "ipc_request\|encode_frame\|decode_frame" crates/wayclick-core/src/ipc.rs` should show only the new-crate import and the remaining server-side `decode_frame`/`write_frame` call sites.

- [ ] **Step 4: Update the benchmark**

Edit `crates/wayclick-core/benches/ipc_framing.rs`. Change:
```rust
use wayclick_core::ipc::{decode_frame, encode_frame};
```
to:
```rust
use wayclick_ipc_client::frame::{decode_frame, encode_frame};
```

- [ ] **Step 5: Verify everything still compiles and passes**

Run: `cargo build -p wayclick-core`
Expected: `Finished` with no warnings.

Run: `cargo test -p wayclick-core`
Expected: All passing (one or two fewer than before because the moved tests are now in the new crate).

Run: `cargo bench -p wayclick-core --no-run`
Expected: Bench compiles.

- [ ] **Step 6: Commit**

```bash
git add crates/wayclick-core/Cargo.toml crates/wayclick-core/src/ipc.rs crates/wayclick-core/benches/ipc_framing.rs Cargo.lock
git commit -m "refactor(core): use wayclick-ipc-client for frame protocol; drop client helpers"
```

---

### Task 8: Migrate `wayclick-tui`

**Files:**
- Modify: `crates/wayclick-tui/Cargo.toml`
- Modify: `crates/wayclick-tui/src/main.rs`

- [ ] **Step 1: Update dependencies**

Edit `crates/wayclick-tui/Cargo.toml`. Replace:
```toml
wayclick-core = { workspace = true }
```
with:
```toml
wayclick-ipc-client = { workspace = true }
```

- [ ] **Step 2: Update `main.rs`**

Edit `crates/wayclick-tui/src/main.rs`:

Replace the line `use wayclick_core::ipc;` with:
```rust
use wayclick_ipc_client::SyncClient;
```

Replace each of the 6 `ipc::ipc_request(&self.socket_path, "method_name", params)` call sites with `SyncClient::request(&self.socket_path, "method_name", params)`. Concretely (current line numbers):

| Line | Before | After |
|------|--------|-------|
| 95   | `ipc::ipc_request(&self.socket_path, "status_json", None)` | `SyncClient::request(&self.socket_path, "status_json", None)` |
| 158  | `ipc::ipc_request(&self.socket_path, "list_triggers", None)` | `SyncClient::request(&self.socket_path, "list_triggers", None)` |
| 197  | `ipc::ipc_request(&self.socket_path, "logs_tail", Some(params))` | `SyncClient::request(&self.socket_path, "logs_tail", Some(params))` |
| 228  | `ipc::ipc_request(&self.socket_path, method, None)` | `SyncClient::request(&self.socket_path, method, None)` |
| 235  | `ipc::ipc_request(&self.socket_path, "reload_config", None)` | `SyncClient::request(&self.socket_path, "reload_config", None)` |
| 244  | `ipc::ipc_request(&self.socket_path, "trigger", Some(params))` | `SyncClient::request(&self.socket_path, "trigger", Some(params))` |

Replace `wayclick_core::config::default_socket_path()` (line 279) with:
```rust
wayclick_ipc_client::socket::default_socket_path()
```

- [ ] **Step 3: Verify the TUI builds cleanly**

Run: `cargo build -p wayclick-tui`
Expected: `Finished` with no warnings.

Run: `cargo clippy -p wayclick-tui --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add crates/wayclick-tui/Cargo.toml crates/wayclick-tui/src/main.rs Cargo.lock
git commit -m "refactor(tui): switch to wayclick-ipc-client SyncClient"
```

---

### Task 9: Migrate `wayclick-playground`

**Files:**
- Modify: `extras/wayclick-playground/Cargo.toml`
- Modify: `extras/wayclick-playground/src/main.rs`
- Modify: `extras/wayclick-playground/src/app_state.rs`
- Modify: `extras/wayclick-playground/src/ui/mod.rs`
- Delete: `extras/wayclick-playground/src/ipc_client.rs`

- [ ] **Step 1: Add the dependency**

Edit `extras/wayclick-playground/Cargo.toml`. In `[dependencies]` add:
```toml
wayclick-ipc-client = { workspace = true }
```

(Playground does **not** have `wayclick-core` as a dep — the spec was wrong about removing it. Just add the new crate.)

- [ ] **Step 2: Update `main.rs`**

Edit `extras/wayclick-playground/src/main.rs`:

Delete the line `mod ipc_client;`.
Replace `use ipc_client::spawn_ipc_thread;` with:
```rust
use wayclick_ipc_client::{socket::default_socket_path, AsyncClient};
```

Replace the line:
```rust
let (ipc_rx, ipc_cmd_tx) = spawn_ipc_thread();
```
with:
```rust
let ipc_client = AsyncClient::connect(default_socket_path())
    .expect("failed to spawn IPC client thread");
```

Update the `AppState::new(...)` call to take the client:
```rust
let mut app_state = AppState::new(ipc_client);
```

- [ ] **Step 3: Update `app_state.rs`**

Edit `extras/wayclick-playground/src/app_state.rs`:

Replace:
```rust
use crate::ipc_client::{FocusedWindow, IpcCommand, IpcMessage, ServiceStatus, TriggerInfo};
```
with:
```rust
use serde_json::json;
use wayclick_ipc_client::{AsyncClient, FocusedWindow, IpcMessage, ServiceStatus, TriggerInfo};
```

Replace the `ipc_rx` / `ipc_cmd_tx` fields with a single `ipc: AsyncClient` field:
```rust
pub struct AppState {
    ipc: AsyncClient,
    // ...other fields stay the same
}
```

Update `AppState::new`:
```rust
pub fn new(ipc: AsyncClient) -> Self {
    Self {
        ipc,
        // ...other fields stay the same
    }
}
```

Replace event-pump calls — `self.ipc_rx.try_recv()` becomes `self.ipc.try_recv()` and the loop should handle `Ok(Some(msg))` / `Ok(None)` / `Err(...)` instead of the `TryRecvError` shape:
```rust
while let Ok(Some(msg)) = self.ipc.try_recv() {
    match msg {
        IpcMessage::Connected { status, triggers, initial_focus } => { /* same body */ }
        IpcMessage::Disconnected => { /* same body */ }
        // ...
    }
}
```

Replace each `IpcCommand::*` send. The four wayclick-specific variants used in `app_state.rs` (lines 206, 218, 221, 228) become explicit `client.send(...)` calls:

| Old | New |
|-----|-----|
| `self.ipc_cmd_tx.send(IpcCommand::FireTrigger(id.to_string()))` | `let _ = self.ipc.send("trigger", Some(json!({"id": id, "press": true})));` |
| `self.ipc_cmd_tx.send(IpcCommand::EnableTrigger(id))` | `let _ = self.ipc.send("enable_trigger", Some(json!({"id": id})));` |
| `self.ipc_cmd_tx.send(IpcCommand::DisableTrigger(id))` | `let _ = self.ipc.send("disable_trigger", Some(json!({"id": id})));` |
| `self.ipc_cmd_tx.send(IpcCommand::RefreshTriggers)` | `let _ = self.ipc.send("list_triggers", None);` |

> **Note on `RefreshTriggers`:** the old `IpcCommand::RefreshTriggers` variant set up in-flight ID tracking so that the response was routed back as `IpcMessage::TriggerListUpdated`. With the new generic `send`, that tracking no longer happens — the response will be silently dropped (it's not a recognised event). The library still emits `TriggerListUpdated` automatically after every `ConfigReloaded`. If `app_state.rs` previously relied on the manual refresh path (search for `RefreshTriggers` calls and check what triggers them), confirm whether `ConfigReloaded` covers the same paths. If it doesn't, this task needs an extension: the trigger-list ID tracking should be added back into `async_client.rs`'s event loop — recognise responses to manually-issued `list_triggers` requests and emit `TriggerListUpdated` for them. Verify before moving on.

- [ ] **Step 4: Update `ui/mod.rs`**

Edit `extras/wayclick-playground/src/ui/mod.rs`. Replace:
```rust
use crate::ipc_client::FocusedWindow;
```
with:
```rust
use wayclick_ipc_client::FocusedWindow;
```

- [ ] **Step 5: Delete the old client file**

```bash
rm extras/wayclick-playground/src/ipc_client.rs
```

- [ ] **Step 6: Verify the playground builds**

Run: `cargo build -p wayclick-playground`
Expected: `Finished` with no warnings.

Run: `cargo clippy -p wayclick-playground --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add extras/wayclick-playground Cargo.lock
git commit -m "refactor(playground): use wayclick-ipc-client AsyncClient"
```

---

## Phase 5 — Tests, Docs, Verification

### Task 10: Final verification + light docs

**Files:**
- Modify: `docs/IPC.md`
- Modify: `CHANGELOG.md`

- [ ] **Step 1: Run the full workspace build and test suite**

Run: `cargo build --workspace`
Expected: All 7 crates build, zero warnings.

Run: `cargo test --workspace`
Expected: All tests pass. Total count should be (previous total − 2 frame tests moved out of core + ~10 new tests in `wayclick-ipc-client`).

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 2: Update `docs/IPC.md`**

Add a short section near the top of `docs/IPC.md`:

```markdown
## Reusable client library

Programs that want to consume the wayclick IPC protocol from Rust can depend on
the `wayclick-ipc-client` crate (in `crates/wayclick-ipc-client`). It provides:

- `frame` — length-prefixed JSON-RPC framing (encode/decode, `MAX_FRAME_SIZE`)
- `socket::default_socket_path()` — XDG/UID-based path resolution
- `SyncClient::request(path, method, params)` — blocking request/response
- `AsyncClient::connect(path)` — background-thread streaming with typed
  `IpcMessage` events (used by `wayclick-playground`)

The crate has no dependency on `wayclick-core` and pulls only `serde`,
`serde_json`, and `thiserror`.
```

- [ ] **Step 3: Update `CHANGELOG.md`**

Add an entry under the unreleased section (or create one):

```markdown
### Added

- New `wayclick-ipc-client` crate consolidates the IPC client logic previously
  duplicated between `wayclick-core` and `wayclick-playground`. Suitable for
  third-party tools that want to talk to the daemon without pulling in the
  whole core crate.

### Changed

- `wayclick-tui` now depends on `wayclick-ipc-client` instead of
  `wayclick-core`.
- `wayclick-playground` now uses `wayclick-ipc-client::AsyncClient` instead of
  its bundled `ipc_client` module.

### Removed

- `wayclick_core::ipc::ipc_request` and `ipc_connect` — these client-side
  helpers moved to `wayclick_ipc_client::SyncClient`. The frame primitives
  (`encode_frame` / `decode_frame` / `write_frame`) likewise moved to
  `wayclick_ipc_client::frame`. Server-side IPC code in `wayclick-core` is
  unchanged in behavior; it now imports the frame primitives from the new
  crate.
```

- [ ] **Step 4: Manual smoke test against the running daemon**

If a daemon is available locally:

```bash
systemctl --user start wayclick    # or however it's started in this env
cargo run -p wayclick-tui          # observe: status fetches, trigger list, logs all populate
cargo run -p wayclick-playground   # observe: connects, particle effects fire on input
```

Verify in the playground that:
- Initial state populates correctly (status + triggers + focus)
- `trigger_activated` / `trigger_deactivated` events drive particle effects
- `config_reloaded` triggers a `TriggerListUpdated` (touch the config and save)
- Disconnect/reconnect: stop the daemon, see `Disconnected`; restart, see `Connected` again

If no daemon is available, document that in the commit message and rely on the unit/integration test coverage.

- [ ] **Step 5: Commit docs**

```bash
git add docs/IPC.md CHANGELOG.md
git commit -m "docs: document wayclick-ipc-client crate and migration"
```

---

## Self-Review Checklist (run before declaring done)

- [ ] `cargo build --workspace` — zero warnings
- [ ] `cargo test --workspace` — all pass
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` — clean
- [ ] No file in the new crate's `src/` imports anything from `wayclick_core`
- [ ] `grep -rn "wayclick_core::ipc::ipc_request\|wayclick_core::ipc::encode_frame\|wayclick_core::ipc::decode_frame" crates extras` returns nothing
- [ ] `extras/wayclick-playground/src/ipc_client.rs` no longer exists
- [ ] `cargo run -p wayclick-tui` works against a running daemon (or smoke skipped with note)
- [ ] `cargo run -p wayclick-playground` works against a running daemon (or smoke skipped with note)
- [ ] `CHANGELOG.md` updated; `docs/IPC.md` mentions new crate

---

## Risk Log

- **Playground manual `RefreshTriggers` path** — flagged inline in Task 9 Step 3. Verify whether the only caller is something `ConfigReloaded` already covers; if not, add ID-tracked manual-list_triggers handling back into `async_client.rs`.
- **Test parallelism on `XDG_RUNTIME_DIR`** — flagged in Task 3 Step 3. If flakes appear, simplify the test rather than serialize.
- **Server-side `ipc.rs` re-imports** — Task 7 Step 2 deletes `ipc_connect`. Confirm the daemon (server-side code in the same file) doesn't accidentally call its own `ipc_connect` for outbound RPC anywhere — `grep -n "ipc_connect" crates/wayclick-core/src/` should return only the deleted definition before deleting.
