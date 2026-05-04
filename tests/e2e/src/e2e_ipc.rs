// SPDX-License-Identifier: MIT
//! E2E tests for IPC protocol correctness: error handling, concurrent connections, and limits.

#[cfg(test)]
mod tests {
    use std::os::unix::net::UnixStream;
    use std::sync::{Arc, Barrier};
    use std::thread;
    use std::time::Duration;

    use serde_json::json;
    use wayclick_core::config::Config;
    use wayclick_ipc_client::SyncClient;

    use crate::helpers::{ipc_call_raw, TestDaemon};

    /// set_layer followed by get_layer reflects the new layer.
    #[test]
    fn test_set_layer_and_get_layer_round_trip() {
        let daemon = TestDaemon::new(Config::default());

        daemon.ipc("set_layer", Some(json!({"layer": "alt"})));
        let response = daemon.ipc("get_layer", None);
        assert_eq!(response["result"]["layer"], "alt");

        daemon.teardown();
    }

    /// Status reflects enabled state and trigger count accurately.
    #[test]
    fn test_status_fields_accurate() {
        let daemon = TestDaemon::new(Config::default());
        daemon.ipc("enable", None);

        // Register a dynamic trigger on a socket that stays open so the trigger is not cleaned up
        let mut sock = daemon.connect();
        let reg = ipc_call_raw(
            &mut sock,
            1,
            "register_trigger",
            json!({
                "id": "status_dyn",
                "mode": "toggle",
                "action": {"type": "auto_click", "button": "left", "interval_ms": 50}
            }),
        );
        assert_eq!(reg["result"]["registered"], "status_dyn");

        let status = daemon.ipc("status", None);
        assert_eq!(status["result"]["enabled"], true);
        assert_eq!(status["result"]["trigger_count"], 1);
        assert_eq!(status["result"]["backend"], "mock");

        drop(sock);
        daemon.teardown();
    }

    /// Triggering a named trigger while the engine is disabled returns an error.
    #[test]
    fn test_trigger_returns_error_when_disabled() {
        let daemon = TestDaemon::new(Config::default());
        // Engine starts disabled; trigger any id — disabled error fires before unknown-trigger check

        let response = daemon.ipc("trigger", Some(json!({"id": "any_trigger"})));
        assert!(
            response.get("error").is_some(),
            "Should return an error when engine is disabled"
        );

        daemon.teardown();
    }

    /// Trigger call with a missing `id` parameter returns JSON-RPC invalid-params error (-32602).
    #[test]
    fn test_trigger_missing_id_returns_error() {
        let daemon = TestDaemon::new(Config::default());
        daemon.ipc("enable", None);

        let mut sock = daemon.connect();
        let response = ipc_call_raw(&mut sock, 1, "trigger", json!({}));

        assert!(response.get("error").is_some(), "Should return an error");
        assert_eq!(
            response["error"]["code"], -32602,
            "Should be invalid-params error"
        );

        daemon.teardown();
    }

    /// Many concurrent connections all receive correct responses.
    #[test]
    fn test_concurrent_connections_all_succeed() {
        let daemon = TestDaemon::new(Config::default());
        let socket_path = daemon.socket_path.clone();

        const N: usize = 8;
        let barrier = Arc::new(Barrier::new(N));

        let handles: Vec<_> = (0..N)
            .map(|_| {
                let path = socket_path.clone();
                let b = barrier.clone();
                thread::spawn(move || {
                    b.wait(); // all threads start at the same time
                    let resp = SyncClient::request(&path, "ping", None).unwrap();
                    assert_eq!(resp["result"], "pong");
                })
            })
            .collect();

        for h in handles {
            h.join().expect("concurrent ping thread panicked");
        }

        daemon.teardown();
    }

    /// The 33rd connection attempt is rejected when 32 connections are active.
    #[test]
    fn test_connection_limit_enforced() {
        const MAX: usize = 32;
        let daemon = TestDaemon::new(Config::default());
        let socket_path = daemon.socket_path.clone();

        // Hold MAX connections open, each confirmed working via ping
        let mut socks: Vec<UnixStream> = Vec::with_capacity(MAX);
        for i in 0..MAX {
            let mut sock = UnixStream::connect(&socket_path).unwrap();
            sock.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            // Confirm the connection is accepted and serviced
            let _ = ipc_call_raw(&mut sock, i as u64, "ping", json!(null));
            socks.push(sock);
        }

        // The (MAX+1)-th connection should be rejected
        let result = SyncClient::request(&socket_path, "ping", None);
        assert!(
            result.is_err(),
            "Connection beyond MAX_IPC_CONNECTIONS should be rejected"
        );

        // Release the held connections
        drop(socks);
        daemon.teardown();
    }
}
