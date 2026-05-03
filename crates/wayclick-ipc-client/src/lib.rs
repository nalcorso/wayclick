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

pub use frame::{IpcError, MAX_FRAME_SIZE};

pub mod socket;

pub mod types;
pub use types::{FocusedWindow, ServiceStatus, TriggerInfo};
