//! E2E tests for dynamic trigger lifecycle: registration, execution, ownership, and cleanup.

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serde_json::json;
    use wayclick_core::config::Config;

    use crate::helpers::{ipc_call_raw, poll_until, TestDaemon};

    /// A registered dynamic trigger appears in the global list_triggers with dynamic:true.
    #[test]
    fn test_dynamic_trigger_visible_in_list_triggers() {
        let daemon = TestDaemon::new(Config::default());
        let mut sock = daemon.connect();

        ipc_call_raw(
            &mut sock,
            1,
            "register_trigger",
            json!({
                "id": "vis_test",
                "mode": "toggle",
                "action": {"type": "auto_click", "button": "left", "interval_ms": 50}
            }),
        );

        // list_triggers (global) must include the dynamic trigger
        let response = daemon.ipc("list_triggers", None);
        let triggers = response["result"].as_array().unwrap();
        let found = triggers.iter().find(|t| t["id"] == "vis_test");
        assert!(found.is_some(), "Dynamic trigger must appear in list_triggers");
        assert!(
            found.unwrap()["dynamic"].as_bool().unwrap_or(false),
            "Trigger should be marked dynamic:true"
        );

        daemon.teardown();
    }

    /// A dynamic toggle trigger produces backend calls when fired.
    #[test]
    fn test_dynamic_trigger_fires_produces_backend_calls() {
        let daemon = TestDaemon::new(Config::default());
        let backend_calls = daemon.backend_calls.clone();

        let mut sock = daemon.connect();
        let reg = ipc_call_raw(
            &mut sock,
            1,
            "register_trigger",
            json!({
                "id": "dyn_clicker",
                "mode": "toggle",
                "action": {"type": "auto_click", "button": "left", "interval_ms": 5}
            }),
        );
        assert_eq!(reg["result"]["registered"], "dyn_clicker");

        daemon.ipc("enable", None);

        // Fire (toggle on)
        daemon.ipc("trigger", Some(json!({"id": "dyn_clicker"})));

        let ok = poll_until(Duration::from_secs(2), || {
            backend_calls.lock().unwrap().len() >= 2
        });
        assert!(ok, "Dynamic trigger should produce backend calls");

        // Fire (toggle off) — stop_worker() joins, so count is stable after this
        daemon.ipc("trigger", Some(json!({"id": "dyn_clicker"})));

        let final_count = backend_calls.lock().unwrap().len();
        assert!(final_count >= 2);

        daemon.teardown();
    }

    /// A connection cannot unregister a dynamic trigger owned by a different connection.
    #[test]
    fn test_cross_connection_unregister_rejected() {
        let daemon = TestDaemon::new(Config::default());

        // Connection A registers the trigger
        let mut sock_a = daemon.connect();
        ipc_call_raw(
            &mut sock_a,
            1,
            "register_trigger",
            json!({
                "id": "owned_trigger",
                "mode": "toggle",
                "action": {"type": "auto_click", "button": "left", "interval_ms": 50}
            }),
        );

        // Connection B tries to unregister it — must fail
        let mut sock_b = daemon.connect();
        let resp = ipc_call_raw(
            &mut sock_b,
            1,
            "unregister_trigger",
            json!({"id": "owned_trigger"}),
        );
        assert!(
            resp.get("error").is_some(),
            "Cross-connection unregister must be rejected"
        );

        daemon.teardown();
    }

    /// An active dynamic trigger is stopped when its owning connection closes.
    #[test]
    fn test_active_dynamic_trigger_stopped_on_disconnect() {
        let daemon = TestDaemon::new(Config::default());
        let engine = daemon.engine.clone();
        let backend_calls = daemon.backend_calls.clone();

        {
            let mut sock = daemon.connect();
            ipc_call_raw(
                &mut sock,
                1,
                "register_trigger",
                json!({
                    "id": "cleanup_test",
                    "mode": "toggle",
                    "action": {"type": "auto_click", "button": "left", "interval_ms": 5}
                }),
            );

            daemon.ipc("enable", None);

            // Toggle on — worker is active
            daemon.ipc("trigger", Some(json!({"id": "cleanup_test"})));

            // Confirm the worker is running
            let ok = poll_until(Duration::from_secs(2), || {
                backend_calls.lock().unwrap().len() >= 2
            });
            assert!(ok, "Worker should be running before disconnect");

            // sock drops here — connection closes; server calls cleanup_connection()
        }

        // Wait for the server to process the disconnect and remove the trigger
        let cleaned = poll_until(Duration::from_secs(2), || {
            let snaps = engine.lock().unwrap().triggers_snapshot();
            !snaps.iter().any(|t| t.id == "cleanup_test")
        });
        assert!(cleaned, "Trigger should be removed from engine after disconnect");

        // Verify the worker is stopped: count should be stable
        let count1 = backend_calls.lock().unwrap().len();
        std::thread::sleep(Duration::from_millis(50));
        let count2 = backend_calls.lock().unwrap().len();
        assert_eq!(count1, count2, "Backend call count should not increase after cleanup");

        daemon.teardown();
    }

    /// Dynamic triggers survive engine config reload (apply_config preserves dynamic triggers).
    #[test]
    fn test_dynamic_trigger_survives_config_reload() {
        let daemon = TestDaemon::new(Config::default());

        let mut sock = daemon.connect();
        ipc_call_raw(
            &mut sock,
            1,
            "register_trigger",
            json!({
                "id": "survivor",
                "mode": "toggle",
                "action": {"type": "auto_click", "button": "left", "interval_ms": 50}
            }),
        );

        // Reload config at the engine level
        daemon.engine.lock().unwrap().apply_config(Config::default());

        // Trigger should still be present
        let snaps = daemon.engine.lock().unwrap().triggers_snapshot();
        assert!(
            snaps.iter().any(|t| t.id == "survivor"),
            "Dynamic trigger must survive config reload"
        );

        daemon.teardown();
    }
}
