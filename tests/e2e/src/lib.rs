// SPDX-License-Identifier: MIT
// Integration tests for wayclick

#[cfg(test)]
mod e2e_actions;
#[cfg(test)]
mod e2e_dynamic;
#[cfg(test)]
mod e2e_events;
#[cfg(test)]
mod e2e_ipc;
#[cfg(test)]
pub mod helpers;

#[cfg(test)]
mod integration {
    use std::io::Write;
    use std::thread;
    use std::time::Duration;

    use serde_json::json;
    use wayclick_core::config::Config;
    use wayclick_core::engine::with_engine_events;
    use wayclick_ipc_client::frame::decode_frame;
    use wayclick_core::logger::{LogLevel, Logger};
    use wayclick_core::lua_api::load_config;

    use crate::helpers::{ipc_call_raw, poll_until, TestDaemon};

    #[test]
    fn test_daemon_startup_and_ping() {
        let daemon = TestDaemon::new(Config::default());
        let response = daemon.ipc("ping", None);
        assert_eq!(response["result"], "pong");
        daemon.teardown();
    }

    #[test]
    fn test_status_disabled_on_start() {
        let daemon = TestDaemon::new(Config::default());
        let response = daemon.ipc("status", None);
        assert_eq!(response["result"]["enabled"], false);
        daemon.teardown();
    }

    #[test]
    fn test_toggle_enable_disable() {
        let daemon = TestDaemon::new(Config::default());

        let response = daemon.ipc("toggle", None);
        assert_eq!(response["result"]["enabled"], true);

        let response = daemon.ipc("status", None);
        assert_eq!(response["result"]["enabled"], true);

        let response = daemon.ipc("toggle", None);
        assert_eq!(response["result"]["enabled"], false);

        daemon.teardown();
    }

    #[test]
    fn test_trigger_list_matches_config() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("init.lua");
        std::fs::File::create(&config_path)
            .unwrap()
            .write_all(
                br#"
                wayclick.register_trigger({
                    id = "test_trigger",
                    name = "Test",
                    action = wayclick.auto_click({ button = "left", interval_ms = 50 }),
                })
            "#,
            )
            .unwrap();

        let logger = Arc::new(Logger::new(100, LogLevel::Trace, false));
        logger.set_quiet(true);
        let config = load_config(&config_path, &logger).unwrap();

        let daemon = TestDaemon::new(config);
        let response = daemon.ipc("list_triggers", None);
        let triggers = response["result"].as_array().unwrap();
        assert_eq!(triggers.len(), 1);
        assert_eq!(triggers[0]["id"], "test_trigger");
        daemon.teardown();
    }

    #[test]
    fn test_enable_disable_commands() {
        let daemon = TestDaemon::new(Config::default());

        let response = daemon.ipc("enable", None);
        assert_eq!(response["result"]["enabled"], true);

        let response = daemon.ipc("disable", None);
        assert_eq!(response["result"]["enabled"], false);

        daemon.teardown();
    }

    #[test]
    fn test_trigger_fires() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("init.lua");
        std::fs::File::create(&config_path)
            .unwrap()
            .write_all(
                br#"
                wayclick.register_trigger({
                    id = "test_fire",
                    mode = "toggle",
                    action = wayclick.auto_click({ button = "left", interval_ms = 10 }),
                })
            "#,
            )
            .unwrap();

        let logger = Arc::new(Logger::new(100, LogLevel::Trace, false));
        logger.set_quiet(true);
        let config = load_config(&config_path, &logger).unwrap();

        let daemon = TestDaemon::new(config);
        daemon.ipc("enable", None);

        let response = daemon.ipc("trigger", Some(json!({"id": "test_fire"})));
        assert!(response.get("result").is_some());

        thread::sleep(Duration::from_millis(50));

        daemon.ipc("trigger", Some(json!({"id": "test_fire"})));
        daemon.teardown();
    }

    #[test]
    fn test_logs_tail() {
        let daemon = TestDaemon::new(Config::default());
        daemon.logger.info("integration test log");

        let response = daemon.ipc("logs_tail", Some(json!({"n": 10})));
        let logs = response["result"].as_array().unwrap();
        assert!(!logs.is_empty());

        daemon.teardown();
    }

    #[test]
    fn test_check_config_valid() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("init.lua");
        std::fs::File::create(&config_path)
            .unwrap()
            .write_all(
                br#"
                wayclick.set_options({ dry_run = true })
                wayclick.register_trigger({
                    id = "test",
                    action = wayclick.noop(),
                })
            "#,
            )
            .unwrap();

        let logger = Arc::new(Logger::new(100, LogLevel::Trace, false));
        logger.set_quiet(true);
        let result = load_config(&config_path, &logger);
        assert!(result.is_ok());
    }

    #[test]
    fn test_check_config_invalid() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("init.lua");
        std::fs::File::create(&config_path)
            .unwrap()
            .write_all(b"this is not valid lua @@@ !!!!")
            .unwrap();

        let logger = Arc::new(Logger::new(100, LogLevel::Trace, false));
        logger.set_quiet(true);
        let result = load_config(&config_path, &logger);
        assert!(result.is_err());
    }

    // --- dynamic trigger tests ---

    #[test]
    fn test_register_and_list_dynamic_trigger() {
        let daemon = TestDaemon::new(Config::default());
        with_engine_events(&daemon.engine, |eng| eng.set_enabled(true));

        let mut sock = daemon.connect();

        let resp = ipc_call_raw(
            &mut sock,
            1,
            "register_trigger",
            json!({
                "id": "dyn_toggle",
                "mode": "toggle",
                "action": {"type": "auto_click", "button": "left", "interval_ms": 50}
            }),
        );
        assert_eq!(resp["result"]["registered"], "dyn_toggle");

        let resp = ipc_call_raw(&mut sock, 2, "list_dynamic_triggers", json!({}));
        let triggers = resp["result"].as_array().unwrap();
        assert_eq!(triggers.len(), 1);
        assert_eq!(triggers[0]["id"], "dyn_toggle");
        assert!(triggers[0]["dynamic"].as_bool().unwrap_or(false));

        daemon.teardown();
    }

    #[test]
    fn test_register_duplicate_trigger_fails() {
        let daemon = TestDaemon::new(Config::default());
        let mut sock = daemon.connect();

        ipc_call_raw(
            &mut sock,
            1,
            "register_trigger",
            json!({
                "id": "dup",
                "mode": "toggle",
                "action": {"type": "auto_click", "button": "left", "interval_ms": 50}
            }),
        );

        let resp = ipc_call_raw(
            &mut sock,
            2,
            "register_trigger",
            json!({
                "id": "dup",
                "mode": "toggle",
                "action": {"type": "auto_click", "button": "left", "interval_ms": 50}
            }),
        );
        assert!(resp.get("error").is_some(), "Duplicate should error");

        daemon.teardown();
    }

    #[test]
    fn test_unregister_dynamic_trigger() {
        let daemon = TestDaemon::new(Config::default());
        let mut sock = daemon.connect();

        ipc_call_raw(
            &mut sock,
            1,
            "register_trigger",
            json!({
                "id": "to_remove",
                "mode": "toggle",
                "action": {"type": "auto_click", "button": "left", "interval_ms": 50}
            }),
        );

        let resp = ipc_call_raw(
            &mut sock,
            2,
            "unregister_trigger",
            json!({"id": "to_remove"}),
        );
        assert_eq!(resp["result"]["unregistered"], "to_remove");

        let resp = ipc_call_raw(&mut sock, 3, "list_dynamic_triggers", json!({}));
        assert_eq!(resp["result"].as_array().unwrap().len(), 0);

        daemon.teardown();
    }

    #[test]
    fn test_dynamic_trigger_cleaned_up_on_disconnect() {
        let daemon = TestDaemon::new(Config::default());
        let engine = daemon.engine.clone();

        {
            let mut sock = daemon.connect();
            ipc_call_raw(
                &mut sock,
                1,
                "register_trigger",
                json!({
                    "id": "ephemeral",
                    "mode": "toggle",
                    "action": {"type": "auto_click", "button": "left", "interval_ms": 50}
                }),
            );
            // sock drops here, closing the connection
        }

        let ok = poll_until(Duration::from_secs(2), || {
            let snaps = engine.lock().unwrap().triggers_snapshot();
            !snaps.iter().any(|t| t.id == "ephemeral")
        });
        assert!(ok, "Dynamic trigger should be cleaned up on disconnect");

        daemon.teardown();
    }

    #[test]
    fn test_register_trigger_validation_error() {
        let daemon = TestDaemon::new(Config::default());
        let mut sock = daemon.connect();

        let resp = ipc_call_raw(
            &mut sock,
            1,
            "register_trigger",
            json!({
                "id": "bad",
                "mode": "toggle",
                "action": {"type": "auto_click", "button": "left", "interval_ms": 0}
            }),
        );
        assert!(
            resp.get("error").is_some(),
            "Zero interval should be rejected"
        );

        daemon.teardown();
    }

    // --- event subscription tests ---

    #[test]
    fn test_subscribe_and_receive_enabled_changed_event() {
        let daemon = TestDaemon::new(Config::default());
        let mut sock = daemon.connect();

        let resp = ipc_call_raw(&mut sock, 1, "subscribe", json!({}));
        assert_eq!(resp["result"]["subscribed"], true);

        daemon.ipc("enable", None);

        let event =
            crate::helpers::wait_for_event(&mut sock, "enabled_changed", Duration::from_secs(3));
        assert!(
            event.is_some(),
            "Should have received enabled_changed event"
        );
        assert_eq!(event.unwrap()["enabled"], true);

        daemon.teardown();
    }

    #[test]
    fn test_unsubscribe_stops_events() {
        let daemon = TestDaemon::new(Config::default());
        let mut sock = daemon.connect();

        ipc_call_raw(&mut sock, 1, "subscribe", json!({}));
        let resp = ipc_call_raw(&mut sock, 2, "unsubscribe", json!({}));
        assert_eq!(resp["result"]["subscribed"], false);

        daemon.ipc("enable", None);

        sock.set_read_timeout(Some(Duration::from_millis(300)))
            .unwrap();
        let received = decode_frame(&mut sock);
        assert!(
            received.is_err(),
            "Should not receive events after unsubscribe"
        );

        daemon.teardown();
    }

    use std::sync::Arc;
}
