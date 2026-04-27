// Integration tests for wayclick

#[cfg(test)]
mod integration {
    use std::io::Write;
    use std::os::unix::net::UnixStream;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration;

    use wayclick_core::config::Config;
    use wayclick_core::engine::Engine;
    use wayclick_core::event_bus::EventBus;
    use wayclick_core::input_backend::{LoggingBackend, MockBackend};
    use wayclick_core::ipc::{decode_frame, encode_frame, ipc_request, write_frame, IpcServer};
    use wayclick_core::logger::{LogLevel, Logger};
    use wayclick_core::lua_api::load_config;
    use serde_json::{json, Value};

    fn setup_test_daemon(
        config: Config,
    ) -> (
        Arc<Mutex<Engine>>,
        Arc<Logger>,
        PathBuf,
        Arc<AtomicBool>,
        thread::JoinHandle<()>,
    ) {
        let logger = Arc::new(Logger::new(100, LogLevel::Trace, false));
        logger.set_quiet(true);

        let event_bus = Arc::new(EventBus::new());
        let backend: Arc<dyn wayclick_core::input_backend::InputBackend> =
            Arc::new(MockBackend::new());
        let engine = Arc::new(Mutex::new(Engine::new(
            config,
            backend,
            logger.clone(),
            event_bus.clone(),
            "test".into(),
        )));

        let dir = tempfile::tempdir().unwrap();
        let socket_path = dir.path().join("test.sock");

        let server = IpcServer::new(socket_path.clone(), engine.clone(), logger.clone(), event_bus).unwrap();
        let shutdown = server.shutdown_flag();

        let handle = thread::spawn(move || {
            server.run();
            // Keep tempdir alive
            let _dir = dir;
        });

        // Wait for server to start
        thread::sleep(Duration::from_millis(100));

        (engine, logger, socket_path, shutdown, handle)
    }

    fn teardown(shutdown: Arc<AtomicBool>, handle: thread::JoinHandle<()>) {
        shutdown.store(true, Ordering::Relaxed);
        let _ = handle.join();
    }

    #[test]
    fn test_daemon_startup_and_ping() {
        let config = Config::default();
        let (_, _, socket_path, shutdown, handle) = setup_test_daemon(config);

        let response = ipc_request(&socket_path, "ping", None).unwrap();
        assert_eq!(response["result"], "pong");

        teardown(shutdown, handle);
    }

    #[test]
    fn test_status_disabled_on_start() {
        let config = Config::default();
        let (_, _, socket_path, shutdown, handle) = setup_test_daemon(config);

        let response = ipc_request(&socket_path, "status", None).unwrap();
        assert_eq!(response["result"]["enabled"], false);

        teardown(shutdown, handle);
    }

    #[test]
    fn test_toggle_enable_disable() {
        let config = Config::default();
        let (_, _, socket_path, shutdown, handle) = setup_test_daemon(config);

        // Toggle on
        let response = ipc_request(&socket_path, "toggle", None).unwrap();
        assert_eq!(response["result"]["enabled"], true);

        // Verify status
        let response = ipc_request(&socket_path, "status", None).unwrap();
        assert_eq!(response["result"]["enabled"], true);

        // Toggle off
        let response = ipc_request(&socket_path, "toggle", None).unwrap();
        assert_eq!(response["result"]["enabled"], false);

        teardown(shutdown, handle);
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

        let (_, _, socket_path, shutdown, handle) = setup_test_daemon(config);

        let response = ipc_request(&socket_path, "list_triggers", None).unwrap();
        let triggers = response["result"].as_array().unwrap();
        assert_eq!(triggers.len(), 1);
        assert_eq!(triggers[0]["id"], "test_trigger");

        teardown(shutdown, handle);
    }

    #[test]
    fn test_enable_disable_commands() {
        let config = Config::default();
        let (_, _, socket_path, shutdown, handle) = setup_test_daemon(config);

        let response = ipc_request(&socket_path, "enable", None).unwrap();
        assert_eq!(response["result"]["enabled"], true);

        let response = ipc_request(&socket_path, "disable", None).unwrap();
        assert_eq!(response["result"]["enabled"], false);

        teardown(shutdown, handle);
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

        let (engine, _, socket_path, shutdown, handle) = setup_test_daemon(config);

        // Enable
        ipc_request(&socket_path, "enable", None).unwrap();

        // Fire trigger
        let response = ipc_request(
            &socket_path,
            "trigger",
            Some(serde_json::json!({"id": "test_fire"})),
        )
        .unwrap();
        assert!(response.get("result").is_some());

        // Wait for clicks
        thread::sleep(Duration::from_millis(50));

        // Stop trigger
        ipc_request(
            &socket_path,
            "trigger",
            Some(serde_json::json!({"id": "test_fire"})),
        )
        .unwrap();

        teardown(shutdown, handle);
    }

    #[test]
    fn test_logs_tail() {
        let config = Config::default();
        let (_, logger, socket_path, shutdown, handle) = setup_test_daemon(config);

        logger.info("integration test log");

        let response = ipc_request(
            &socket_path,
            "logs_tail",
            Some(serde_json::json!({"n": 10})),
        )
        .unwrap();
        let logs = response["result"].as_array().unwrap();
        assert!(!logs.is_empty());

        teardown(shutdown, handle);
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

    fn ipc_call(sock: &mut UnixStream, id: u64, method: &str, params: Value) -> Value {
        let req = json!({"jsonrpc":"2.0","id":id,"method":method,"params":params});
        write_frame(sock, &req).unwrap();
        decode_frame(sock).unwrap()
    }

    #[test]
    fn test_register_and_list_dynamic_trigger() {
        let config = Config::default();
        let (engine, _, socket_path, shutdown, handle) = setup_test_daemon(config);
        engine.lock().unwrap().set_enabled(true);

        let mut sock = UnixStream::connect(&socket_path).unwrap();
        sock.set_read_timeout(Some(Duration::from_secs(5))).unwrap();

        let resp = ipc_call(
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

        let resp = ipc_call(&mut sock, 2, "list_dynamic_triggers", json!({}));
        let triggers = resp["result"].as_array().unwrap();
        assert_eq!(triggers.len(), 1);
        assert_eq!(triggers[0]["id"], "dyn_toggle");
        assert!(triggers[0]["dynamic"].as_bool().unwrap_or(false));

        teardown(shutdown, handle);
    }

    #[test]
    fn test_register_duplicate_trigger_fails() {
        let config = Config::default();
        let (_, _, socket_path, shutdown, handle) = setup_test_daemon(config);

        let mut sock = UnixStream::connect(&socket_path).unwrap();
        sock.set_read_timeout(Some(Duration::from_secs(5))).unwrap();

        ipc_call(
            &mut sock,
            1,
            "register_trigger",
            json!({
                "id": "dup",
                "mode": "toggle",
                "action": {"type": "auto_click", "button": "left", "interval_ms": 50}
            }),
        );

        let resp = ipc_call(
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

        teardown(shutdown, handle);
    }

    #[test]
    fn test_unregister_dynamic_trigger() {
        let config = Config::default();
        let (_, _, socket_path, shutdown, handle) = setup_test_daemon(config);

        let mut sock = UnixStream::connect(&socket_path).unwrap();
        sock.set_read_timeout(Some(Duration::from_secs(5))).unwrap();

        ipc_call(
            &mut sock,
            1,
            "register_trigger",
            json!({
                "id": "to_remove",
                "mode": "toggle",
                "action": {"type": "auto_click", "button": "left", "interval_ms": 50}
            }),
        );

        let resp = ipc_call(&mut sock, 2, "unregister_trigger", json!({"id": "to_remove"}));
        assert_eq!(resp["result"]["unregistered"], "to_remove");

        let resp = ipc_call(&mut sock, 3, "list_dynamic_triggers", json!({}));
        assert_eq!(resp["result"].as_array().unwrap().len(), 0);

        teardown(shutdown, handle);
    }

    #[test]
    fn test_dynamic_trigger_cleaned_up_on_disconnect() {
        let config = Config::default();
        let (engine, _, socket_path, shutdown, handle) = setup_test_daemon(config);

        {
            let mut sock = UnixStream::connect(&socket_path).unwrap();
            sock.set_read_timeout(Some(Duration::from_secs(5))).unwrap();

            ipc_call(
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

        // Wait for server to process the disconnect.
        thread::sleep(Duration::from_millis(200));

        let snapshots = engine.lock().unwrap().triggers_snapshot();
        let found = snapshots.iter().any(|t| t.id == "ephemeral");
        assert!(!found, "Dynamic trigger should be cleaned up on disconnect");

        teardown(shutdown, handle);
    }

    #[test]
    fn test_register_trigger_validation_error() {
        let config = Config::default();
        let (_, _, socket_path, shutdown, handle) = setup_test_daemon(config);

        let mut sock = UnixStream::connect(&socket_path).unwrap();
        sock.set_read_timeout(Some(Duration::from_secs(5))).unwrap();

        // interval_ms = 0 should fail validation (min is 1).
        let resp = ipc_call(
            &mut sock,
            1,
            "register_trigger",
            json!({
                "id": "bad",
                "mode": "toggle",
                "action": {"type": "auto_click", "button": "left", "interval_ms": 0}
            }),
        );
        assert!(resp.get("error").is_some(), "Zero interval should be rejected");

        teardown(shutdown, handle);
    }

    // --- event subscription tests ---

    #[test]
    fn test_subscribe_and_receive_enabled_changed_event() {
        let config = Config::default();
        let (_, _, socket_path, shutdown, handle) = setup_test_daemon(config);

        let mut sock = UnixStream::connect(&socket_path).unwrap();
        sock.set_read_timeout(Some(Duration::from_secs(5))).unwrap();

        // Subscribe to all events.
        let resp = ipc_call(&mut sock, 1, "subscribe", json!({}));
        assert_eq!(resp["result"]["subscribed"], true);

        // Enable via a second connection so we don't send on the subscribed socket.
        ipc_request(&socket_path, "enable", None).unwrap();

        // Drain frames until we see an enabled_changed event (or timeout).
        let mut got_event = false;
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while std::time::Instant::now() < deadline {
            match decode_frame(&mut sock) {
                Ok(msg) => {
                    if msg.get("method").and_then(|v| v.as_str()) == Some("event") {
                        if msg["params"]["type"] == "enabled_changed" {
                            assert_eq!(msg["params"]["enabled"], true);
                            got_event = true;
                            break;
                        }
                    }
                }
                Err(_) => break,
            }
        }
        assert!(got_event, "Should have received enabled_changed event");

        teardown(shutdown, handle);
    }

    #[test]
    fn test_unsubscribe_stops_events() {
        let config = Config::default();
        let (_, _, socket_path, shutdown, handle) = setup_test_daemon(config);

        let mut sock = UnixStream::connect(&socket_path).unwrap();
        sock.set_read_timeout(Some(Duration::from_millis(500))).unwrap();

        ipc_call(&mut sock, 1, "subscribe", json!({}));
        let resp = ipc_call(&mut sock, 2, "unsubscribe", json!({}));
        assert_eq!(resp["result"]["subscribed"], false);

        // Trigger an event.
        ipc_request(&socket_path, "enable", None).unwrap();
        thread::sleep(Duration::from_millis(100));

        // Next read should time out — no events should arrive.
        let received = decode_frame(&mut sock);
        assert!(received.is_err(), "Should not receive events after unsubscribe");

        teardown(shutdown, handle);
    }
}
