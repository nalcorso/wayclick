use serde::Serialize;
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

/// All event types that the event bus can emit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    TriggerActivated,
    TriggerDeactivated,
    LayerChanged,
    EnabledChanged,
    ConfigReloaded,
}

impl EventType {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "trigger_activated" => Some(Self::TriggerActivated),
            "trigger_deactivated" => Some(Self::TriggerDeactivated),
            "layer_changed" => Some(Self::LayerChanged),
            "enabled_changed" => Some(Self::EnabledChanged),
            "config_reloaded" => Some(Self::ConfigReloaded),
            _ => None,
        }
    }
}

/// An event emitted by the engine.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
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
}

impl Event {
    pub fn event_type(&self) -> EventType {
        match self {
            Event::TriggerActivated { .. } => EventType::TriggerActivated,
            Event::TriggerDeactivated { .. } => EventType::TriggerDeactivated,
            Event::LayerChanged { .. } => EventType::LayerChanged,
            Event::EnabledChanged { .. } => EventType::EnabledChanged,
            Event::ConfigReloaded { .. } => EventType::ConfigReloaded,
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
        let mut subs = self.subscribers.lock().unwrap();
        subs.retain(|entry| {
            if !entry.accepts(event) {
                return true; // keep — not targeting this subscriber
            }
            entry.tx.try_send(event.clone()).is_ok()
        });
    }

    /// Remove all subscribers (called on shutdown).
    pub fn clear(&self) {
        self.subscribers.lock().unwrap().clear();
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
        let event = rx.recv_timeout(std::time::Duration::from_millis(100)).unwrap();
        assert!(matches!(event, Event::TriggerActivated { ref trigger_id, .. } if trigger_id == "t1"));
    }

    #[test]
    fn test_filter_only_matching_events_delivered() {
        let bus = EventBus::new();
        let rx = bus.subscribe(Some(vec![EventType::LayerChanged]));
        bus.publish(&Event::trigger_activated("t1".into()));
        bus.publish(&Event::layer_changed("base".into(), "combat".into()));
        // First event was filtered; only layer_changed arrives
        let event = rx.recv_timeout(std::time::Duration::from_millis(100)).unwrap();
        assert!(matches!(event, Event::LayerChanged { .. }));
        assert!(rx.try_recv().is_err(), "no more events expected");
    }

    #[test]
    fn test_multiple_subscribers() {
        let bus = EventBus::new();
        let rx1 = bus.subscribe(None);
        let rx2 = bus.subscribe(None);
        bus.publish(&Event::enabled_changed(false));
        assert!(rx1.recv_timeout(std::time::Duration::from_millis(100)).is_ok());
        assert!(rx2.recv_timeout(std::time::Duration::from_millis(100)).is_ok());
    }

    #[test]
    fn test_dead_subscriber_pruned() {
        let bus = EventBus::new();
        let rx = bus.subscribe(None);
        drop(rx); // simulate connection close
        // Should not panic; dead subscriber pruned on next publish
        bus.publish(&Event::config_reloaded());
        assert_eq!(bus.subscribers.lock().unwrap().len(), 0);
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
        assert_eq!(bus.subscribers.lock().unwrap().len(), 0);
    }

    #[test]
    fn test_event_type_from_str() {
        assert_eq!(EventType::from_str("trigger_activated"), Some(EventType::TriggerActivated));
        assert_eq!(EventType::from_str("config_reloaded"), Some(EventType::ConfigReloaded));
        assert_eq!(EventType::from_str("unknown"), None);
    }
}
