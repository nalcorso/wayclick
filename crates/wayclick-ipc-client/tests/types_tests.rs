// SPDX-License-Identifier: MIT
//! Round-trip tests against sample daemon JSON. These guard the types
//! against silent drift — if the daemon's response shape changes and these
//! tests don't update, deserialization will fail at runtime instead of in CI.

use serde_json::{from_value, json};
use wayclick_ipc_client::{FocusedWindow, ServiceStatus, TriggerInfo};

#[test]
fn service_status_deserializes_from_daemon_status_response() {
    // Mirrors crates/wayclick-core/src/ipc.rs handle_status response shape.
    let value = json!({
        "enabled": true,
        "trigger_count": 3,
        "active_triggers": ["combat.crit", "movement.dash"],
        "layer": "default",
        "uptime_secs": 123,
        "dry_run": false,
    });
    let status: ServiceStatus =
        from_value(value).expect("daemon status response should deserialize");
    assert!(status.enabled);
    assert_eq!(status.trigger_count, 3);
    assert_eq!(
        status.active_triggers,
        vec!["combat.crit".to_string(), "movement.dash".to_string()]
    );
    assert_eq!(status.layer, "default");
    assert_eq!(status.uptime_secs, 123);
    assert!(!status.dry_run);
}

#[test]
fn service_status_handles_missing_optional_fields() {
    // Older daemons might omit some fields; serde(default) keeps us tolerant.
    let value = json!({"enabled": false});
    let status: ServiceStatus = from_value(value).expect("partial status should deserialize");
    assert!(!status.enabled);
    assert_eq!(status.trigger_count, 0);
    assert!(status.active_triggers.is_empty());
    assert_eq!(status.layer, "");
}

#[test]
fn trigger_info_deserializes_from_daemon_list_triggers_entry() {
    let value = json!({
        "id": "combat.crit",
        "name": "Critical Strike",
        "mode": "oneshot",
        "active": false,
        "activate_count": 42,
        "user_enabled": true,
        "dynamic": false,
    });
    let trigger: TriggerInfo = from_value(value).expect("trigger entry should deserialize");
    assert_eq!(trigger.id, "combat.crit");
    assert_eq!(trigger.name, "Critical Strike");
    assert_eq!(trigger.mode, "oneshot");
    assert!(!trigger.active);
    assert_eq!(trigger.activate_count, 42);
    assert!(trigger.user_enabled);
    assert!(!trigger.dynamic);
}

#[test]
fn focused_window_deserializes_inner_shape() {
    // The daemon's get_focus returns {"window": <this>} — this test deserializes
    // just the inner shape, which is the contract callers will use after extracting.
    let value = json!({
        "app_id": "firefox",
        "title": "Mozilla Firefox",
        "process_name": "firefox",
        "backend": "hyprland",
        "xwayland": false,
    });
    let window: FocusedWindow = from_value(value).expect("focus window should deserialize");
    assert_eq!(window.app_id, "firefox");
    assert_eq!(window.title, "Mozilla Firefox");
    assert_eq!(window.process_name.as_deref(), Some("firefox"));
    assert_eq!(window.backend, "hyprland");
    assert!(!window.xwayland);
}

#[test]
fn focused_window_handles_missing_process_name() {
    let value = json!({
        "app_id": "unknown",
        "title": "",
        "backend": "x11",
        "xwayland": true,
    });
    let window: FocusedWindow =
        from_value(value).expect("window without process_name should deserialize");
    assert_eq!(window.app_id, "unknown");
    assert_eq!(window.process_name, None);
    assert!(window.xwayland);
}
