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
