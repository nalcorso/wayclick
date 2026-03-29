// Integration tests for wayclick

#[cfg(test)]
mod integration {
    use std::io::Write;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration;

    use wayclick_core::config::Config;
    use wayclick_core::engine::Engine;
    use wayclick_core::input_backend::{LoggingBackend, MockBackend};
    use wayclick_core::ipc::{ipc_request, IpcServer};
    use wayclick_core::logger::{LogLevel, Logger};
    use wayclick_core::lua_api::load_config;

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

        let backend: Arc<dyn wayclick_core::input_backend::InputBackend> =
            Arc::new(MockBackend::new());
        let engine = Arc::new(Mutex::new(Engine::new(
            config,
            backend,
            logger.clone(),
            "test".into(),
        )));

        let dir = tempfile::tempdir().unwrap();
        let socket_path = dir.path().join("test.sock");

        let server =
            IpcServer::new(socket_path.clone(), engine.clone(), logger.clone()).unwrap();
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
}
