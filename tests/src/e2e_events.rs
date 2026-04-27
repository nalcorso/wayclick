//! E2E tests for the event subscription system.
//! Tests cover: event types, filtering, multiplexed commands+events, and multiple subscribers.

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serde_json::json;
    use wayclick_core::config::{
        ActionConfig, Config, TriggerBinding, TriggerMode,
    };
    use wayclick_core::engine::with_engine_events;
    use wayclick_core::ipc::decode_frame;

    use crate::helpers::{ipc_call_raw, wait_for_event, TestDaemon};

    fn toggle_trigger(id: &str) -> TriggerBinding {
        TriggerBinding {
            id: id.to_string(),
            name: id.to_string(),
            description: String::new(),
            mode: TriggerMode::Toggle,
            action: ActionConfig::AutoClick {
                button: wayclick_core::config::MouseButton::Left,
                interval_ms: 50,
                duration_ms: None,
                jitter_ms: 0,
                hold_ms: 0,
            },
            cooldown_ms: None,
        }
    }

    /// Toggle on fires trigger_activated; toggle off fires trigger_deactivated.
    #[test]
    fn test_trigger_activated_and_deactivated_events() {
        let config = Config {
            triggers: vec![toggle_trigger("evt_toggle")],
            ..Config::default()
        };
        let daemon = TestDaemon::new(config);
        with_engine_events(&daemon.engine, |eng| eng.set_enabled(true));

        let mut sock = daemon.connect();
        let resp = ipc_call_raw(&mut sock, 1, "subscribe", json!({}));
        assert_eq!(resp["result"]["subscribed"], true);

        // Toggle on
        daemon.ipc("trigger", Some(json!({"id": "evt_toggle"})));
        let event = wait_for_event(&mut sock, "trigger_activated", Duration::from_secs(3));
        assert!(event.is_some(), "Expected trigger_activated event");
        assert_eq!(event.unwrap()["trigger_id"], "evt_toggle");

        // Toggle off
        daemon.ipc("trigger", Some(json!({"id": "evt_toggle"})));
        let event = wait_for_event(&mut sock, "trigger_deactivated", Duration::from_secs(3));
        assert!(event.is_some(), "Expected trigger_deactivated event");
        assert_eq!(event.unwrap()["trigger_id"], "evt_toggle");

        daemon.teardown();
    }

    /// layer_changed event includes correct from/to fields.
    #[test]
    fn test_layer_changed_event_fields() {
        let daemon = TestDaemon::new(Config::default());

        let mut sock = daemon.connect();
        ipc_call_raw(&mut sock, 1, "subscribe", json!({}));

        daemon.ipc("set_layer", Some(json!({"layer": "combat"})));

        let event = wait_for_event(&mut sock, "layer_changed", Duration::from_secs(3));
        assert!(event.is_some(), "Expected layer_changed event");
        let event = event.unwrap();
        assert_eq!(event["from"], "base");
        assert_eq!(event["to"], "combat");

        daemon.teardown();
    }

    /// config_reloaded event is emitted when engine.apply_config() is called directly.
    #[test]
    fn test_config_reloaded_event_via_engine() {
        let daemon = TestDaemon::new(Config::default());

        let mut sock = daemon.connect();
        ipc_call_raw(&mut sock, 1, "subscribe", json!({}));

        // Trigger the event at the engine level (IPC reload_config is a stub that does not reload)
        with_engine_events(&daemon.engine, |eng| eng.apply_config(Config::default()));

        let event = wait_for_event(&mut sock, "config_reloaded", Duration::from_secs(3));
        assert!(event.is_some(), "Expected config_reloaded event");

        daemon.teardown();
    }

    /// A filter subscription ignores non-matching events; matching events still arrive (positive control).
    #[test]
    fn test_filtered_subscription_with_positive_control() {
        let daemon = TestDaemon::new(Config::default());

        let mut sock = daemon.connect();
        // Subscribe to only layer_changed
        let resp = ipc_call_raw(
            &mut sock,
            1,
            "subscribe",
            json!({"events": ["layer_changed"]}),
        );
        assert_eq!(resp["result"]["subscribed"], true);

        // Fire enabled_changed — should be filtered out
        daemon.ipc("enable", None);

        // Short wait: no enabled_changed event should arrive on our filtered socket
        sock.set_read_timeout(Some(Duration::from_millis(300))).unwrap();
        let spurious = decode_frame(&mut sock);
        assert!(
            spurious.is_err(),
            "enabled_changed should be filtered; got: {:?}",
            spurious
        );

        // Fire layer_changed — SHOULD arrive (positive control)
        sock.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        daemon.ipc("set_layer", Some(json!({"layer": "pvp"})));
        let event = wait_for_event(&mut sock, "layer_changed", Duration::from_secs(3));
        assert!(event.is_some(), "layer_changed must pass through filter");
        assert_eq!(event.unwrap()["to"], "pvp");

        daemon.teardown();
    }

    /// Commands and events can be multiplexed on a single socket.
    /// After subscribing and sending `enable` on the same socket, both the
    /// command response (non-null id) and the enabled_changed event (null id) must arrive.
    #[test]
    fn test_commands_and_events_multiplexed() {
        let daemon = TestDaemon::new(Config::default());
        let mut sock = daemon.connect();

        // Subscribe first; receive the subscribe response
        let subscribe_resp = ipc_call_raw(&mut sock, 1, "subscribe", json!({}));
        assert_eq!(subscribe_resp["result"]["subscribed"], true);

        // Enable on same socket — triggers enabled_changed event + response on same socket
        let req = json!({"jsonrpc":"2.0","id":2,"method":"enable","params":{}});
        wayclick_core::ipc::write_frame(&mut sock, &req).unwrap();

        // Drain until we have both the enable response (id=2) and the enabled_changed event
        let mut got_response = false;
        let mut got_event = false;
        let deadline = std::time::Instant::now() + Duration::from_secs(3);

        while (!got_response || !got_event) && std::time::Instant::now() < deadline {
            let remaining = deadline
                .checked_duration_since(std::time::Instant::now())
                .unwrap_or(Duration::from_millis(1));
            sock.set_read_timeout(Some(remaining.max(Duration::from_millis(1)))).unwrap();

            match decode_frame(&mut sock) {
                Ok(msg) => {
                    if msg.get("method").and_then(|v| v.as_str()) == Some("event") {
                        if msg["params"]["type"] == "enabled_changed" {
                            got_event = true;
                        }
                    } else if msg["id"] == json!(2) {
                        assert_eq!(msg["result"]["enabled"], true);
                        got_response = true;
                    }
                }
                Err(_) => break,
            }
        }

        assert!(got_response, "Should receive enable command response on subscribed socket");
        assert!(got_event, "Should receive enabled_changed event on same socket");

        daemon.teardown();
    }

    /// Two independent subscribers both receive the same event.
    #[test]
    fn test_two_subscribers_both_receive_event() {
        let daemon = TestDaemon::new(Config::default());

        let mut sock1 = daemon.connect();
        let mut sock2 = daemon.connect();

        ipc_call_raw(&mut sock1, 1, "subscribe", json!({}));
        ipc_call_raw(&mut sock2, 1, "subscribe", json!({}));

        daemon.ipc("enable", None);

        let event1 = wait_for_event(&mut sock1, "enabled_changed", Duration::from_secs(3));
        let event2 = wait_for_event(&mut sock2, "enabled_changed", Duration::from_secs(3));

        assert!(event1.is_some(), "First subscriber should receive event");
        assert!(event2.is_some(), "Second subscriber should receive event");

        daemon.teardown();
    }

    /// Re-subscribing replaces the previous filter; the old filter is not retained.
    #[test]
    fn test_resubscribe_replaces_filter() {
        let daemon = TestDaemon::new(Config::default());
        let mut sock = daemon.connect();

        // First subscription: all events
        ipc_call_raw(&mut sock, 1, "subscribe", json!({}));

        // Re-subscribe to only layer_changed — replaces the previous filter
        let resp = ipc_call_raw(
            &mut sock,
            2,
            "subscribe",
            json!({"events": ["layer_changed"]}),
        );
        assert_eq!(resp["result"]["subscribed"], true);

        // enabled_changed should NOT arrive now (it was in the previous all-events filter)
        daemon.ipc("enable", None);
        sock.set_read_timeout(Some(Duration::from_millis(300))).unwrap();
        let spurious = decode_frame(&mut sock);
        assert!(
            spurious.is_err(),
            "enabled_changed should be filtered after resubscribe; got: {:?}",
            spurious
        );

        // layer_changed SHOULD arrive (positive control)
        sock.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        daemon.ipc("set_layer", Some(json!({"layer": "town"})));
        let event = wait_for_event(&mut sock, "layer_changed", Duration::from_secs(3));
        assert!(event.is_some(), "layer_changed must arrive after resubscribe");

        daemon.teardown();
    }
}
