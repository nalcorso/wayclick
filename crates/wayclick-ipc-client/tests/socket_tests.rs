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
