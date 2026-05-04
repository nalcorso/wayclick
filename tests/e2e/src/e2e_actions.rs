// SPDX-License-Identifier: MIT
//! E2E tests that validate the full IPC→engine→backend path for action execution.
//! These tests verify the *output* of actions (backend calls) rather than the IPC protocol.

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serde_json::json;
    use wayclick_core::config::{ActionConfig, CompositeMode, Config, TriggerBinding, TriggerMode};
    use wayclick_core::engine::with_engine_events;
    use wayclick_core::input_backend::BackendCall;

    use crate::helpers::{poll_until, TestDaemon};

    fn toggle_trigger(id: &str, action: ActionConfig) -> TriggerBinding {
        TriggerBinding {
            id: id.to_string(),
            name: id.to_string(),
            description: String::new(),
            mode: TriggerMode::Toggle,
            action,
            cooldown_ms: None,
        }
    }

    fn oneshot_trigger(id: &str, action: ActionConfig) -> TriggerBinding {
        TriggerBinding {
            id: id.to_string(),
            name: id.to_string(),
            description: String::new(),
            mode: TriggerMode::OneShot,
            action,
            cooldown_ms: None,
        }
    }

    fn hold_trigger(id: &str, action: ActionConfig) -> TriggerBinding {
        TriggerBinding {
            id: id.to_string(),
            name: id.to_string(),
            description: String::new(),
            mode: TriggerMode::Hold,
            action,
            cooldown_ms: None,
        }
    }

    /// Oneshot keystroke produces KeyPress then KeyRelease in the correct order.
    #[test]
    fn test_oneshot_keystroke_backend_calls() {
        let config = Config {
            triggers: vec![oneshot_trigger(
                "ks",
                ActionConfig::Keystroke {
                    key_name: "KEY_A".into(),
                    key_code: 30,
                    modifier_names: vec![],
                    modifier_codes: vec![],
                    hold_ms: 0,
                },
            )],
            ..Config::default()
        };

        let daemon = TestDaemon::new(config);
        with_engine_events(&daemon.engine, |eng| eng.set_enabled(true));

        daemon.ipc("trigger", Some(json!({"id": "ks"})));

        let calls = daemon.backend_calls.lock().unwrap().clone();
        assert_eq!(calls.len(), 2, "Expected exactly 2 backend calls");
        assert_eq!(calls[0], BackendCall::KeyPress(30));
        assert_eq!(calls[1], BackendCall::KeyRelease(30));

        daemon.teardown();
    }

    /// Toggle auto-click accumulates multiple clicks while active; count stops after toggle-off.
    #[test]
    fn test_toggle_auto_click_accumulates_clicks() {
        let config = Config {
            triggers: vec![toggle_trigger(
                "clicker",
                ActionConfig::AutoClick {
                    button: wayclick_core::config::MouseButton::Left,
                    interval_ms: 5,
                    duration_ms: None,
                    jitter_ms: 0,
                    hold_ms: 0,
                },
            )],
            ..Config::default()
        };

        let daemon = TestDaemon::new(config);
        let backend_calls = daemon.backend_calls.clone();
        with_engine_events(&daemon.engine, |eng| eng.set_enabled(true));

        // Toggle on
        daemon.ipc("trigger", Some(json!({"id": "clicker"})));

        // Wait for at least 2 clicks to accumulate
        let ok = poll_until(Duration::from_secs(2), || {
            backend_calls.lock().unwrap().len() >= 2
        });
        assert!(ok, "Should accumulate at least 2 clicks while toggle is on");

        // Toggle off — stop_worker() joins the worker thread, so count is stable after this returns
        daemon.ipc("trigger", Some(json!({"id": "clicker"})));

        let count_after_stop = backend_calls.lock().unwrap().len();
        assert!(count_after_stop >= 2);

        // Verify count does not increase further
        std::thread::sleep(Duration::from_millis(50));
        let count_later = backend_calls.lock().unwrap().len();
        assert_eq!(
            count_after_stop, count_later,
            "Count should not increase after toggle-off"
        );

        daemon.teardown();
    }

    /// Hold trigger starts clicking on press=true and stops on press=false.
    #[test]
    fn test_hold_trigger_stops_on_release() {
        let config = Config {
            triggers: vec![hold_trigger(
                "holder",
                ActionConfig::AutoClick {
                    button: wayclick_core::config::MouseButton::Left,
                    interval_ms: 5,
                    duration_ms: None,
                    jitter_ms: 0,
                    hold_ms: 0,
                },
            )],
            ..Config::default()
        };

        let daemon = TestDaemon::new(config);
        let backend_calls = daemon.backend_calls.clone();
        with_engine_events(&daemon.engine, |eng| eng.set_enabled(true));

        // Press
        daemon.ipc("trigger", Some(json!({"id": "holder", "press": true})));

        let ok = poll_until(Duration::from_secs(2), || {
            backend_calls.lock().unwrap().len() >= 2
        });
        assert!(ok, "Should accumulate clicks while held");

        // Release — stop_worker() joins, count stable after this
        daemon.ipc("trigger", Some(json!({"id": "holder", "press": false})));

        let count_after_release = backend_calls.lock().unwrap().len();
        std::thread::sleep(Duration::from_millis(50));
        let count_later = backend_calls.lock().unwrap().len();
        assert_eq!(
            count_after_release, count_later,
            "Count should not increase after release"
        );

        daemon.teardown();
    }

    /// Oneshot composite sequence fires sub-actions in the correct order.
    #[test]
    fn test_oneshot_sequence_correct_order() {
        let config = Config {
            triggers: vec![oneshot_trigger(
                "seq",
                ActionConfig::Composite {
                    mode: CompositeMode::Sequence,
                    actions: vec![
                        ActionConfig::Keystroke {
                            key_name: "KEY_A".into(),
                            key_code: 30,
                            modifier_names: vec![],
                            modifier_codes: vec![],
                            hold_ms: 0,
                        },
                        ActionConfig::Keystroke {
                            key_name: "KEY_ENTER".into(),
                            key_code: 28,
                            modifier_names: vec![],
                            modifier_codes: vec![],
                            hold_ms: 0,
                        },
                    ],
                },
            )],
            ..Config::default()
        };

        let daemon = TestDaemon::new(config);
        with_engine_events(&daemon.engine, |eng| eng.set_enabled(true));

        daemon.ipc("trigger", Some(json!({"id": "seq"})));

        let calls = daemon.backend_calls.lock().unwrap().clone();
        assert_eq!(
            calls.len(),
            4,
            "Expected 4 backend calls for 2-keystroke sequence"
        );
        assert_eq!(calls[0], BackendCall::KeyPress(30));
        assert_eq!(calls[1], BackendCall::KeyRelease(30));
        assert_eq!(calls[2], BackendCall::KeyPress(28));
        assert_eq!(calls[3], BackendCall::KeyRelease(28));

        daemon.teardown();
    }
}
