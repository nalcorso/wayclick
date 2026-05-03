// SPDX-License-Identifier: MIT
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock};
use wayclick_ipc_client::socket::{default_socket_path, socket_path_for_user};

/// Serializes tests that mutate the process-wide `XDG_RUNTIME_DIR` env var.
/// Cargo runs tests in the same binary in parallel; without this guard,
/// concurrent set/remove on the same env var produces flakes.
fn env_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

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
    let _guard = env_lock();
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
    let _guard = env_lock();
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
