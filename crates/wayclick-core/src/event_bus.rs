// SPDX-License-Identifier: MIT
use crate::focus_tracker::WindowInfo;
use crate::MutexExt;
use serde::Serialize;
use std::str::FromStr;
use std::sync::mpsc;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// Returns the current time as milliseconds since the Unix epoch.
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Error type for parsing EventType from strings.
#[derive(Debug, Clone, Copy)]
pub struct ParseEventTypeError;

impl std::fmt::Display for ParseEventTypeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "unknown event type")
    }
}

impl std::error::Error for ParseEventTypeError {}

/// All event types that the event bus can emit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    TriggerActivated,
    TriggerDeactivated,
    LayerChanged,
    EnabledChanged,
    ConfigReloaded,
    /// Raw key/button press or release observed by the evdev monitor.
    /// Only value=1 (press) and value=0 (release) are published; repeats (value=2) are filtered.
    InputReceived,
    /// Mouse wheel or horizontal scroll observed by the evdev monitor.
    /// delta_y > 0 = scroll up; delta_x > 0 = scroll right.
    ScrollReceived,
    /// The focused window changed (new process/app gained focus, or window title updated).
    FocusChanged,
}

impl FromStr for EventType {
    type Err = ParseEventTypeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "trigger_activated" => Ok(Self::TriggerActivated),
            "trigger_deactivated" => Ok(Self::TriggerDeactivated),
            "layer_changed" => Ok(Self::LayerChanged),
            "enabled_changed" => Ok(Self::EnabledChanged),
            "config_reloaded" => Ok(Self::ConfigReloaded),
            "input_received" => Ok(Self::InputReceived),
            "scroll_received" => Ok(Self::ScrollReceived),
            "focus_changed" => Ok(Self::FocusChanged),
            _ => Err(ParseEventTypeError),
        }
    }
}

impl EventType {
    /// Parse a string to EventType, returning None if invalid.
    pub fn from_str_opt(s: &str) -> Option<Self> {
        s.parse().ok()
    }
}

/// An event emitted by the engine.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum Event {
    TriggerActivated {
        trigger_id: String,
        timestamp_ms: u64,
    },
    TriggerDeactivated {
        trigger_id: String,
        timestamp_ms: u64,
    },
    LayerChanged {
        from: String,
        to: String,
        timestamp_ms: u64,
    },
    EnabledChanged {
        enabled: bool,
        timestamp_ms: u64,
    },
    ConfigReloaded {
        timestamp_ms: u64,
    },
    /// Raw key or button press/release from the evdev monitor.
    /// `value` is 1 for press, 0 for release (repeats are never published).
    InputReceived {
        code: u16,
        value: i32,
        device_name: String,
        timestamp_ms: u64,
    },
    /// Mouse wheel or horizontal scroll from the evdev monitor.
    /// `delta_y` > 0 = scroll up, < 0 = scroll down (REL_WHEEL).
    /// `delta_x` > 0 = scroll right, < 0 = scroll left (REL_HWHEEL).
    ScrollReceived {
        delta_x: i32,
        delta_y: i32,
        device_name: String,
        timestamp_ms: u64,
    },
    /// The focused window changed.
    ///
    /// `window` is `None` when focus moves to the desktop (no window focused).
    /// `previous` is `None` when focus was previously unknown (e.g., at startup).
    /// Both `app_id` changes and title-only changes emit this event; the `previous`
    /// field lets subscribers compare to detect process-level changes.
    FocusChanged {
        window: Option<WindowInfo>,
        previous: Option<WindowInfo>,
        timestamp_ms: u64,
    },
}

impl Event {
    pub fn event_type(&self) -> EventType {
        match self {
            Event::TriggerActivated { .. } => EventType::TriggerActivated,
            Event::TriggerDeactivated { .. } => EventType::TriggerDeactivated,
            Event::LayerChanged { .. } => EventType::LayerChanged,
            Event::EnabledChanged { .. } => EventType::EnabledChanged,
            Event::ConfigReloaded { .. } => EventType::ConfigReloaded,
            Event::InputReceived { .. } => EventType::InputReceived,
            Event::ScrollReceived { .. } => EventType::ScrollReceived,
            Event::FocusChanged { .. } => EventType::FocusChanged,
        }
    }

    pub fn trigger_activated(trigger_id: String) -> Self {
        Self::TriggerActivated {
            trigger_id,
            timestamp_ms: now_ms(),
        }
    }

    pub fn trigger_deactivated(trigger_id: String) -> Self {
        Self::TriggerDeactivated {
            trigger_id,
            timestamp_ms: now_ms(),
        }
    }

    pub fn layer_changed(from: String, to: String) -> Self {
        Self::LayerChanged {
            from,
            to,
            timestamp_ms: now_ms(),
        }
    }

    pub fn enabled_changed(enabled: bool) -> Self {
        Self::EnabledChanged {
            enabled,
            timestamp_ms: now_ms(),
        }
    }

    pub fn config_reloaded() -> Self {
        Self::ConfigReloaded {
            timestamp_ms: now_ms(),
        }
    }

    pub fn input_received(code: u16, value: i32, device_name: String) -> Self {
        Self::InputReceived {
            code,
            value,
            device_name,
            timestamp_ms: now_ms(),
        }
    }

    pub fn scroll_received(delta_x: i32, delta_y: i32, device_name: String) -> Self {
        Self::ScrollReceived {
            delta_x,
            delta_y,
            device_name,
            timestamp_ms: now_ms(),
        }
    }

    pub fn focus_changed(window: Option<WindowInfo>, previous: Option<WindowInfo>) -> Self {
        Self::FocusChanged {
            window,
            previous,
            timestamp_ms: now_ms(),
        }
    }
}

/// Per-subscriber state: an optional event type filter and a bounded send channel.
struct SubscriberEntry {
    /// `None` means the subscriber receives all event types.
    filter: Option<Vec<EventType>>,
    tx: mpsc::SyncSender<Event>,
}

impl SubscriberEntry {
    fn accepts(&self, event: &Event) -> bool {
        match &self.filter {
            None => true,
            Some(types) => types.contains(&event.event_type()),
        }
    }
}

/// Thread-safe event bus. Multiple subscribers receive events via bounded channels.
/// Slow or disconnected subscribers are silently dropped when their channel is full or closed.
pub struct EventBus {
    subscribers: Mutex<Vec<SubscriberEntry>>,
}

/// Capacity of each subscriber's event channel.
/// If a subscriber cannot keep up it will be silently removed.
const SUBSCRIBER_CHANNEL_CAPACITY: usize = 64;

impl EventBus {
    pub fn new() -> Self {
        Self {
            subscribers: Mutex::new(Vec::new()),
        }
    }

    /// Subscribe to events. Pass `Some(types)` to receive only certain event types,
    /// or `None` to receive all. Returns a receiver the caller drains in its own thread.
    pub fn subscribe(&self, filter: Option<Vec<EventType>>) -> mpsc::Receiver<Event> {
        let (tx, rx) = mpsc::sync_channel(SUBSCRIBER_CHANNEL_CAPACITY);
        self.subscribers
            .lock()
            .unwrap()
            .push(SubscriberEntry { filter, tx });
        rx
    }

    /// Publish an event to all matching subscribers.
    /// Subscribers whose channel is full or closed are removed.
    pub fn publish(&self, event: &Event) {
        let mut subs = self.subscribers.lock_or_recover();
        subs.retain(|entry| {
            if !entry.accepts(event) {
                return true; // keep — not targeting this subscriber
            }
            entry.tx.try_send(event.clone()).is_ok()
        });
    }

    /// Remove all subscribers (called on shutdown).
    pub fn clear(&self) {
        self.subscribers.lock_or_recover().clear();
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_publish_received_by_subscriber() {
        let bus = EventBus::new();
        let rx = bus.subscribe(None);
        bus.publish(&Event::trigger_activated("t1".into()));
        let event = rx
            .recv_timeout(std::time::Duration::from_millis(100))
            .unwrap();
        assert!(
            matches!(event, Event::TriggerActivated { ref trigger_id, .. } if trigger_id == "t1")
        );
    }

    #[test]
    fn test_filter_only_matching_events_delivered() {
        let bus = EventBus::new();
        let rx = bus.subscribe(Some(vec![EventType::LayerChanged]));
        bus.publish(&Event::trigger_activated("t1".into()));
        bus.publish(&Event::layer_changed("base".into(), "combat".into()));
        // First event was filtered; only layer_changed arrives
        let event = rx
            .recv_timeout(std::time::Duration::from_millis(100))
            .unwrap();
        assert!(matches!(event, Event::LayerChanged { .. }));
        assert!(rx.try_recv().is_err(), "no more events expected");
    }

    #[test]
    fn test_multiple_subscribers() {
        let bus = EventBus::new();
        let rx1 = bus.subscribe(None);
        let rx2 = bus.subscribe(None);
        bus.publish(&Event::enabled_changed(false));
        assert!(rx1
            .recv_timeout(std::time::Duration::from_millis(100))
            .is_ok());
        assert!(rx2
            .recv_timeout(std::time::Duration::from_millis(100))
            .is_ok());
    }

    #[test]
    fn test_dead_subscriber_pruned() {
        let bus = EventBus::new();
        let rx = bus.subscribe(None);
        drop(rx); // simulate connection close
                  // Should not panic; dead subscriber pruned on next publish
        bus.publish(&Event::config_reloaded());
        assert_eq!(bus.subscribers.lock_or_recover().len(), 0);
    }

    #[test]
    fn test_slow_subscriber_pruned_when_full() {
        let bus = EventBus::new();
        let _rx = bus.subscribe(None); // never drained
                                       // Fill the channel beyond capacity
        for i in 0..=SUBSCRIBER_CHANNEL_CAPACITY {
            bus.publish(&Event::trigger_activated(format!("t{}", i)));
        }
        // After overflow, subscriber is removed
        assert_eq!(bus.subscribers.lock_or_recover().len(), 0);
    }

    #[test]
    fn test_event_type_from_str() {
        assert_eq!(
            EventType::from_str_opt("trigger_activated"),
            Some(EventType::TriggerActivated)
        );
        assert_eq!(
            EventType::from_str_opt("config_reloaded"),
            Some(EventType::ConfigReloaded)
        );
        assert_eq!(
            EventType::from_str_opt("scroll_received"),
            Some(EventType::ScrollReceived)
        );
        assert_eq!(EventType::from_str_opt("unknown"), None);
    }

    #[test]
    fn test_scroll_received_serialization() {
        let event = Event::scroll_received(1, -3, "test-mouse".to_string());
        let val = serde_json::to_value(&event).unwrap();
        assert_eq!(val["type"], "scroll_received");
        assert_eq!(val["delta_x"], 1);
        assert_eq!(val["delta_y"], -3);
        assert_eq!(val["device_name"], "test-mouse");
    }

    #[test]
    fn test_scroll_received_published_and_received() {
        let bus = EventBus::new();
        let rx = bus.subscribe(Some(vec![EventType::ScrollReceived]));
        bus.publish(&Event::scroll_received(0, 1, "mouse".to_string()));
        let event = rx
            .recv_timeout(std::time::Duration::from_millis(100))
            .unwrap();
        assert!(matches!(
            event,
            Event::ScrollReceived {
                delta_x: 0,
                delta_y: 1,
                ..
            }
        ));
    }
}
