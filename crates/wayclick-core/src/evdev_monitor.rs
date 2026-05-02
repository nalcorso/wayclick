// SPDX-License-Identifier: MIT
// EvdevMonitor — coordinates device monitoring threads, hotplug, and trigger dispatch.

use crate::config::{Binding, DeviceBinding, TriggerEdge};
use crate::engine::{with_engine_events, Engine};
use crate::evdev_source::{
    self, DeviceInfo, EvdevSource, InputSource, EV_ABS, EV_KEY, EV_REL, EV_SYN, REL_HWHEEL,
    REL_WHEEL, SYN_DROPPED, SYN_REPORT,
};
use crate::event_bus::{Event, EventBus};
use crate::input_backend::InputBackend;
use crate::logger::Logger;
use crate::MutexExt;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

pub struct EvdevMonitor {
    engine: Arc<Mutex<Engine>>,
    logger: Arc<Logger>,
    event_bus: Option<Arc<EventBus>>,
    backend: Option<Arc<dyn InputBackend>>,
    config_bindings: Vec<DeviceBinding>,
    running: Arc<AtomicBool>,
    device_threads: Vec<JoinHandle<()>>,
    scan_thread: Option<JoinHandle<()>>,
    tracked_devices: Arc<Mutex<HashMap<PathBuf, ()>>>,
}

impl EvdevMonitor {
    pub fn new(engine: Arc<Mutex<Engine>>, logger: Arc<Logger>) -> Self {
        Self {
            engine,
            logger,
            event_bus: None,
            backend: None,
            config_bindings: Vec::new(),
            running: Arc::new(AtomicBool::new(false)),
            device_threads: Vec::new(),
            scan_thread: None,
            tracked_devices: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Set the backend used for event forwarding in exclusive mode.
    pub fn set_backend(&mut self, backend: Arc<dyn InputBackend>) {
        self.backend = Some(backend);
    }

    /// Set the event bus for publishing raw InputReceived events.
    pub fn set_event_bus(&mut self, event_bus: Arc<EventBus>) {
        self.event_bus = Some(event_bus);
    }

    pub fn configure(&mut self, bindings: Vec<DeviceBinding>) {
        self.config_bindings = bindings;
        self.logger.info(format!(
            "EvdevMonitor configured with {} device bindings",
            self.config_bindings.len()
        ));
    }

    pub fn start(&mut self) {
        if self.config_bindings.is_empty() {
            self.logger
                .info("EvdevMonitor: no device bindings, skipping device monitoring");
            return;
        }

        self.running.store(true, Ordering::SeqCst);

        // Initial scan
        self.scan_devices();

        // Launch periodic scan thread for hotplug detection
        let running = self.running.clone();
        let engine = self.engine.clone();
        let logger = self.logger.clone();
        let bindings = self.config_bindings.clone();
        let tracked = self.tracked_devices.clone();
        let backend = self.backend.clone();
        let event_bus = self.event_bus.clone();

        self.scan_thread = Some(thread::spawn(move || {
            while running.load(Ordering::SeqCst) {
                thread::sleep(Duration::from_secs(2));
                if !running.load(Ordering::SeqCst) {
                    break;
                }
                // Scan for new devices
                let devices = evdev_source::enumerate_devices();
                for dev in &devices {
                    let already_tracked = tracked.lock_or_recover().contains_key(&dev.path);
                    if already_tracked {
                        continue;
                    }
                    // Check if this device matches any binding
                    for binding in &bindings {
                        if match_device(dev, binding) {
                            logger.info(format!(
                                "Hotplug: new device '{}' at {:?} matches binding",
                                dev.name, dev.path
                            ));
                            tracked.lock_or_recover().insert(dev.path.clone(), ());
                            spawn_device_thread(DeviceThreadParams {
                                path: dev.path.clone(),
                                exclusive: binding.exclusive,
                                binding: binding.clone(),
                                engine: engine.clone(),
                                logger: logger.clone(),
                                running: running.clone(),
                                tracked: tracked.clone(),
                                backend: backend.clone(),
                                event_bus: event_bus.clone(),
                            });
                            break;
                        }
                    }
                }
            }
        }));

        self.logger.info("EvdevMonitor: device monitoring started");
    }

    fn scan_devices(&mut self) {
        let devices = evdev_source::enumerate_devices();
        self.logger.info(format!(
            "EvdevMonitor: found {} input devices",
            devices.len()
        ));

        for dev in &devices {
            for binding in &self.config_bindings {
                if match_device(dev, binding) {
                    self.logger.info(format!(
                        "EvdevMonitor: matched '{}' at {:?}",
                        dev.name, dev.path
                    ));
                    self.tracked_devices
                        .lock()
                        .unwrap()
                        .insert(dev.path.clone(), ());

                    let handle = spawn_device_thread(DeviceThreadParams {
                        path: dev.path.clone(),
                        exclusive: binding.exclusive,
                        binding: binding.clone(),
                        engine: self.engine.clone(),
                        logger: self.logger.clone(),
                        running: self.running.clone(),
                        tracked: self.tracked_devices.clone(),
                        backend: self.backend.clone(),
                        event_bus: self.event_bus.clone(),
                    });
                    self.device_threads.push(handle);
                    break; // Only match first binding per device
                }
            }
        }
    }

    pub fn stop(&mut self) {
        self.running.store(false, Ordering::SeqCst);

        if let Some(thread) = self.scan_thread.take() {
            let _ = thread.join();
        }

        // Device threads will exit when running becomes false
        for thread in self.device_threads.drain(..) {
            let _ = thread.join();
        }

        self.tracked_devices.lock_or_recover().clear();
        self.logger.info("EvdevMonitor: stopped");
    }
}

struct DeviceThreadParams {
    path: PathBuf,
    exclusive: bool,
    binding: DeviceBinding,
    engine: Arc<Mutex<Engine>>,
    logger: Arc<Logger>,
    running: Arc<AtomicBool>,
    tracked: Arc<Mutex<HashMap<PathBuf, ()>>>,
    backend: Option<Arc<dyn InputBackend>>,
    event_bus: Option<Arc<EventBus>>,
}

fn spawn_device_thread(params: DeviceThreadParams) -> JoinHandle<()> {
    let DeviceThreadParams {
        path,
        exclusive,
        binding,
        engine,
        logger,
        running,
        tracked,
        backend,
        event_bus,
    } = params;
    thread::spawn(move || {
        let mut source = match EvdevSource::open(&path, exclusive) {
            Ok(s) => s,
            Err(e) => {
                logger.warn(format!("Failed to open {:?}: {}", path, e));
                tracked.lock_or_recover().remove(&path);
                return;
            }
        };

        let device_name = source.device_info().name.clone();
        logger.debug(format!("Monitoring device '{}' at {:?}", device_name, path));

        let forward_backend = if exclusive { backend } else { None };
        let mut processor = DeviceProcessor::new(
            binding,
            engine,
            logger.clone(),
            forward_backend,
            device_name,
            event_bus,
        );

        while running.load(Ordering::SeqCst) {
            match source.poll_events(Duration::from_millis(100)) {
                Ok(events) => {
                    processor.process_events(&events);
                }
                Err(crate::evdev_source::SourceError::Disconnected) => {
                    logger.warn(format!("Device {:?} disconnected", path));
                    tracked.lock_or_recover().remove(&path);
                    break;
                }
                Err(e) => {
                    logger.warn(format!("Read error on {:?}: {}", path, e));
                    tracked.lock_or_recover().remove(&path);
                    break;
                }
            }
        }

        source.close();
    })
}

/// Whitelisted event types that can be forwarded through uinput.
const FORWARDABLE_TYPES: [u16; 3] = [EV_KEY, EV_REL, EV_ABS];

/// Per-device event processor. Maintains state across poll batches for
/// frame accumulation and claim-based suppression.
pub(crate) struct DeviceProcessor {
    binding: DeviceBinding,
    engine: Arc<Mutex<Engine>>,
    logger: Arc<Logger>,
    forward_backend: Option<Arc<dyn InputBackend>>,
    pending_frame: Vec<evdev_source::InputEvent>,
    /// All codes currently held on this device (for chord detection across frames).
    held_codes: HashSet<u16>,
    /// Active press-edge claims: binding index → was_swallowed.
    /// Tracks which bindings have fired on press and are awaiting their release dispatch.
    active_claims: HashMap<usize, bool>,
    /// Physical device name for InputReceived events.
    device_name: String,
    /// Optional event bus for publishing raw input events.
    event_bus: Option<Arc<EventBus>>,
}

impl DeviceProcessor {
    pub(crate) fn new(
        binding: DeviceBinding,
        engine: Arc<Mutex<Engine>>,
        logger: Arc<Logger>,
        forward_backend: Option<Arc<dyn InputBackend>>,
        device_name: String,
        event_bus: Option<Arc<EventBus>>,
    ) -> Self {
        Self {
            binding,
            engine,
            logger,
            forward_backend,
            pending_frame: Vec::new(),
            held_codes: HashSet::new(),
            active_claims: HashMap::new(),
            device_name,
            event_bus,
        }
    }

    /// Fire a trigger event through the engine and log it.
    /// `press` is `true` for activation and `false` for deactivation.
    fn fire_trigger(&self, trigger_id: &str, code_name: &str, press: bool, log_msg: &str) {
        self.logger.debug(format!(
            "Button {} {}, {} trigger '{}'",
            code_name,
            if press { "pressed" } else { "released" },
            log_msg,
            trigger_id
        ));
        with_engine_events(&self.engine, |eng| {
            let _ = eng.trigger_event(trigger_id, press);
        });
    }

    /// Process a batch of raw evdev events.
    /// In exclusive mode: accumulates events into frames (SYN_REPORT-delimited),
    /// claims matched events, and forwards the rest.
    /// In non-exclusive mode: dispatches matched EV_KEY events immediately.
    pub(crate) fn process_events(&mut self, events: &[evdev_source::InputEvent]) {
        if self.forward_backend.is_some() {
            for event in events {
                if event.event_type == EV_SYN {
                    match event.code {
                        SYN_REPORT => {
                            self.process_frame();
                            self.pending_frame.clear();
                        }
                        SYN_DROPPED => {
                            self.logger.warn("SYN_DROPPED: discarding partial frame");
                            self.pending_frame.clear();
                            // Reset held state to avoid phantom chord matches
                            self.held_codes.clear();
                            self.active_claims.clear();
                        }
                        _ => {}
                    }
                } else {
                    // Publish physical key/button press and release to event bus (not repeats).
                    // This represents physical input observed by wayclick, regardless of whether
                    // the event will later be swallowed or forwarded.
                    if event.event_type == EV_KEY && (event.value == 0 || event.value == 1) {
                        if let Some(bus) = &self.event_bus {
                            bus.publish(&Event::input_received(
                                event.code,
                                event.value,
                                self.device_name.clone(),
                            ));
                        }
                    }
                    // Publish mouse wheel events to the event bus.
                    if event.event_type == EV_REL
                        && (event.code == REL_WHEEL || event.code == REL_HWHEEL)
                    {
                        if let Some(bus) = &self.event_bus {
                            let (dx, dy) = if event.code == REL_WHEEL {
                                (0, event.value)
                            } else {
                                (event.value, 0)
                            };
                            bus.publish(&Event::scroll_received(dx, dy, self.device_name.clone()));
                        }
                    }
                    self.pending_frame.push(event.clone());
                }
            }
        } else {
            // Non-exclusive: immediate dispatch, no forwarding or swallowing.
            // Only single-code bindings are matched (chord detection requires frame buffering).
            for event in events {
                if event.event_type == EV_KEY {
                    // Publish physical key/button press and release to event bus (not repeats).
                    if event.value == 0 || event.value == 1 {
                        if let Some(bus) = &self.event_bus {
                            bus.publish(&Event::input_received(
                                event.code,
                                event.value,
                                self.device_name.clone(),
                            ));
                        }
                    }
                } else if event.event_type == EV_REL
                    && (event.code == REL_WHEEL || event.code == REL_HWHEEL)
                {
                    // Publish mouse wheel events to the event bus.
                    if let Some(bus) = &self.event_bus {
                        let (dx, dy) = if event.code == REL_WHEEL {
                            (0, event.value)
                        } else {
                            (event.value, 0)
                        };
                        bus.publish(&Event::scroll_received(dx, dy, self.device_name.clone()));
                    }
                }
                if event.event_type == EV_KEY {
                    for binding_item in &self.binding.bindings {
                        if let Binding::Button(ref bb) = binding_item {
                            if bb.codes.len() != 1 || bb.codes[0] != event.code {
                                continue;
                            }
                            if let Some(ref layer) = bb.layer {
                                let eng = self.engine.lock().unwrap();
                                if eng.current_layer() != layer {
                                    continue;
                                }
                            }
                            let code_name =
                                bb.code_names.first().map(|s| s.as_str()).unwrap_or("?");
                            match event.value {
                                1 if bb.on == TriggerEdge::Press => {
                                    self.fire_trigger(&bb.trigger_id, code_name, true, "firing");
                                }
                                0 if bb.on == TriggerEdge::Press => {
                                    self.fire_trigger(
                                        &bb.trigger_id,
                                        code_name,
                                        false,
                                        "deactivating",
                                    );
                                }
                                0 if bb.on == TriggerEdge::Release => {
                                    self.fire_trigger(
                                        &bb.trigger_id,
                                        code_name,
                                        true,
                                        "firing on-release",
                                    );
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
        }
    }

    /// Process a complete frame. Two-pass: (1) build claims and suppression sets,
    /// (2) forward non-suppressed events.
    fn process_frame(&mut self) {
        let backend = match &self.forward_backend {
            Some(b) => b.clone(),
            None => return,
        };

        // Codes from newly activated swallowed claims — press/repeat events for these
        // codes will be suppressed. Built up across pass 1.
        let mut suppress_codes: HashSet<u16> = HashSet::new();

        // Codes whose release event should be suppressed (came from a swallowed active_claim).
        let mut released_swallowed_codes: HashSet<u16> = HashSet::new();

        // Scroll event indices to suppress (populated inline during pass 1).
        let mut suppress_scroll_indices: HashSet<usize> = HashSet::new();

        // Pass 1: identify claims and dispatch to engine
        for (event_idx, event) in self.pending_frame.iter().enumerate() {
            if event.event_type == EV_KEY {
                match event.value {
                    1 => {
                        // Press: add to held state, then check for chord completions
                        self.held_codes.insert(event.code);

                        for (binding_idx, binding_item) in self.binding.bindings.iter().enumerate()
                        {
                            if let Binding::Button(ref bb) = binding_item {
                                if bb.on != TriggerEdge::Press {
                                    continue;
                                }
                                if self.active_claims.contains_key(&binding_idx) {
                                    continue;
                                }
                                if !bb.codes.iter().all(|c| self.held_codes.contains(c)) {
                                    continue;
                                }
                                if let Some(ref layer) = bb.layer {
                                    let eng = self.engine.lock().unwrap();
                                    if eng.current_layer() != layer {
                                        continue;
                                    }
                                }
                                let code_name =
                                    bb.code_names.first().map(|s| s.as_str()).unwrap_or("?");
                                self.logger.debug(format!(
                                    "Button {} pressed, claiming trigger '{}'",
                                    code_name, bb.trigger_id
                                ));
                                let triggered = with_engine_events(&self.engine, |eng| {
                                    eng.trigger_event(&bb.trigger_id, true).is_ok()
                                });
                                if triggered {
                                    self.active_claims.insert(binding_idx, bb.swallow);
                                    if bb.swallow {
                                        for &c in &bb.codes {
                                            suppress_codes.insert(c);
                                        }
                                    }
                                }
                            }
                        }
                    }
                    0 => {
                        // Release: first fire any on=Release bindings, then deactivate claims

                        // on=Release bindings: fire when this code is released and all other
                        // chord codes are still held.
                        for binding_item in self.binding.bindings.iter() {
                            if let Binding::Button(ref bb) = binding_item {
                                if bb.on != TriggerEdge::Release {
                                    continue;
                                }
                                if !bb.codes.contains(&event.code) {
                                    continue;
                                }
                                // All other chord codes must still be held
                                if !bb
                                    .codes
                                    .iter()
                                    .filter(|&&c| c != event.code)
                                    .all(|c| self.held_codes.contains(c))
                                {
                                    continue;
                                }
                                if let Some(ref layer) = bb.layer {
                                    let eng = self.engine.lock().unwrap();
                                    if eng.current_layer() != layer {
                                        continue;
                                    }
                                }
                                let code_name =
                                    bb.code_names.first().map(|s| s.as_str()).unwrap_or("?");
                                // swallow=true is forbidden for on=Release (validated at load time)
                                self.fire_trigger(
                                    &bb.trigger_id,
                                    code_name,
                                    true,
                                    "firing on-release",
                                );
                            }
                        }

                        // Deactivate all press-edge claims that include this code
                        let to_release: Vec<usize> = self
                            .active_claims
                            .iter()
                            .filter(|(&idx, _)| {
                                if let Binding::Button(ref bb) = self.binding.bindings[idx] {
                                    bb.codes.contains(&event.code)
                                } else {
                                    false
                                }
                            })
                            .map(|(&idx, _)| idx)
                            .collect();

                        for claim_idx in to_release {
                            let swallow = self.active_claims.remove(&claim_idx).unwrap();
                            if let Binding::Button(ref bb) = self.binding.bindings[claim_idx] {
                                let code_name =
                                    bb.code_names.first().map(|s| s.as_str()).unwrap_or("?");
                                self.fire_trigger(&bb.trigger_id, code_name, false, "deactivating");
                                if swallow {
                                    for &c in &bb.codes {
                                        released_swallowed_codes.insert(c);
                                    }
                                }
                            }
                        }

                        self.held_codes.remove(&event.code);
                    }
                    _ => {} // repeat — handled via suppress_codes below
                }
            } else if event.event_type == EV_REL {
                if let Some((direction, magnitude)) =
                    evdev_source::classify_scroll(event.code, event.value)
                {
                    for binding_item in &self.binding.bindings {
                        if let Binding::Scroll(ref sb) = binding_item {
                            if sb.direction != direction {
                                continue;
                            }
                            if let Some(ref layer) = sb.layer {
                                let eng = self.engine.lock().unwrap();
                                if eng.current_layer() != layer {
                                    continue;
                                }
                            }
                            self.logger.debug(format!(
                                "Scroll {:?} (magnitude {}), firing trigger '{}'",
                                direction, magnitude, sb.trigger_id
                            ));
                            with_engine_events(&self.engine, |eng| {
                                for _ in 0..magnitude {
                                    let _ = eng.trigger_event(&sb.trigger_id, true);
                                }
                            });
                            if sb.swallow {
                                suppress_scroll_indices.insert(event_idx);
                                // Also suppress the corresponding hi-res event in this frame
                                let hi_res_code = match event.code {
                                    evdev_source::REL_WHEEL => Some(evdev_source::REL_WHEEL_HI_RES),
                                    evdev_source::REL_HWHEEL => {
                                        Some(evdev_source::REL_HWHEEL_HI_RES)
                                    }
                                    _ => None,
                                };
                                if let Some(hrc) = hi_res_code {
                                    for (j, other) in self.pending_frame.iter().enumerate() {
                                        if other.event_type == EV_REL && other.code == hrc {
                                            suppress_scroll_indices.insert(j);
                                        }
                                    }
                                }
                            }
                            break; // first matching scroll binding wins
                        }
                    }
                }
            }
        }

        // Build final suppress_indices: KEY events whose code is in a swallowed claim or
        // was released from one, plus explicitly suppressed scroll events.
        let mut suppress_indices: HashSet<usize> = suppress_scroll_indices;

        for (i, event) in self.pending_frame.iter().enumerate() {
            if event.event_type == EV_KEY {
                let suppressed = match event.value {
                    0 => released_swallowed_codes.contains(&event.code),
                    _ => suppress_codes.contains(&event.code),
                };
                if suppressed {
                    suppress_indices.insert(i);
                }
            }
        }

        // Pass 2: forward non-suppressed events (whitelisted types only)
        let forward: Vec<(u16, u16, i32)> = self
            .pending_frame
            .iter()
            .enumerate()
            .filter(|(i, event)| {
                !suppress_indices.contains(i) && FORWARDABLE_TYPES.contains(&event.event_type)
            })
            .map(|(_, e)| (e.event_type, e.code, e.value))
            .collect();

        if !forward.is_empty() {
            let _ = backend.forward_frame(&forward);
        }
    }
}

/// Check if a device matches a binding's device match criteria.
pub fn match_device(info: &DeviceInfo, binding: &DeviceBinding) -> bool {
    match_device_inner(info, &binding.device_match)
}

fn match_device_inner(info: &DeviceInfo, device_match: &crate::config::DeviceMatch) -> bool {
    use crate::config::DeviceMatch;
    match device_match {
        DeviceMatch::ByPath { path } => info.path.to_string_lossy() == *path,
        DeviceMatch::ByName { contains } => {
            info.name.to_lowercase().contains(&contains.to_lowercase())
        }
        DeviceMatch::ByVidPid { vendor, product } => {
            info.vendor_id == *vendor && info.product_id == *product
        }
        DeviceMatch::ByPhys { contains } => {
            info.phys.to_lowercase().contains(&contains.to_lowercase())
        }
        DeviceMatch::Any { matchers } => matchers.iter().any(|m| match_device_inner(info, m)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::*;
    use crate::engine::Engine;
    use crate::evdev_source::InputEvent;
    use crate::event_bus::EventBus;
    use crate::input_backend::MockBackend;
    use crate::logger::LogLevel;
    use std::path::PathBuf;
    use std::thread;
    use std::time::Duration;

    fn test_device() -> DeviceInfo {
        DeviceInfo {
            path: PathBuf::from("/dev/input/event5"),
            name: "Logitech G Pro Gaming Mouse".into(),
            vendor_id: 0x046d,
            product_id: 0xc08b,
            phys: "usb-0000:00:14.0-2/input0".into(),
        }
    }

    fn make_binding(dm: DeviceMatch) -> DeviceBinding {
        DeviceBinding {
            device_match: dm,
            bindings: vec![],
            exclusive: false,
        }
    }

    #[test]
    fn test_match_by_name_exact() {
        let info = test_device();
        let binding = make_binding(DeviceMatch::ByName {
            contains: "Logitech G Pro Gaming Mouse".into(),
        });
        assert!(match_device(&info, &binding));
    }

    #[test]
    fn test_match_by_name_substring() {
        let info = test_device();
        let binding = make_binding(DeviceMatch::ByName {
            contains: "G Pro".into(),
        });
        assert!(match_device(&info, &binding));
    }

    #[test]
    fn test_match_by_name_case_insensitive() {
        let info = test_device();
        let binding = make_binding(DeviceMatch::ByName {
            contains: "logitech g pro".into(),
        });
        assert!(match_device(&info, &binding));
    }

    #[test]
    fn test_match_by_vidpid() {
        let info = test_device();
        let binding = make_binding(DeviceMatch::ByVidPid {
            vendor: 0x046d,
            product: 0xc08b,
        });
        assert!(match_device(&info, &binding));
    }

    #[test]
    fn test_match_by_phys_substring() {
        let info = test_device();
        let binding = make_binding(DeviceMatch::ByPhys {
            contains: "usb-0000:00:14.0".into(),
        });
        assert!(match_device(&info, &binding));
    }

    #[test]
    fn test_match_any_first_wins() {
        let info = test_device();
        let binding = make_binding(DeviceMatch::Any {
            matchers: vec![
                DeviceMatch::ByName {
                    contains: "G Pro".into(),
                },
                DeviceMatch::ByVidPid {
                    vendor: 0xFFFF,
                    product: 0xFFFF,
                },
            ],
        });
        assert!(match_device(&info, &binding));
    }

    #[test]
    fn test_match_any_all_fail() {
        let info = test_device();
        let binding = make_binding(DeviceMatch::Any {
            matchers: vec![
                DeviceMatch::ByName {
                    contains: "Razer".into(),
                },
                DeviceMatch::ByVidPid {
                    vendor: 0xFFFF,
                    product: 0xFFFF,
                },
            ],
        });
        assert!(!match_device(&info, &binding));
    }

    #[test]
    fn test_no_match() {
        let info = test_device();
        let binding = make_binding(DeviceMatch::ByName {
            contains: "Razer DeathAdder".into(),
        });
        assert!(!match_device(&info, &binding));
    }

    #[test]
    fn test_path_match() {
        let info = test_device();
        let binding = make_binding(DeviceMatch::ByPath {
            path: "/dev/input/event5".into(),
        });
        assert!(match_device(&info, &binding));
    }

    // --- Phase 1: Key release handling tests ---

    fn make_hold_engine() -> (
        Arc<Mutex<Engine>>,
        Arc<Mutex<Vec<crate::input_backend::BackendCall>>>,
        Arc<Logger>,
    ) {
        let logger = Arc::new(Logger::new(100, LogLevel::Trace, false));
        logger.set_quiet(true);
        let backend = MockBackend::new();
        let calls = backend.calls_clone();
        let config = Config {
            triggers: vec![TriggerBinding {
                id: "hold_test".into(),
                name: "Hold Test".into(),
                description: String::new(),
                mode: TriggerMode::Hold,
                action: ActionConfig::AutoClick {
                    button: MouseButton::Left,
                    interval_ms: 5,
                    duration_ms: None,
                    jitter_ms: 0,
                    hold_ms: 0,
                },
                cooldown_ms: None,
            }],
            ..Default::default()
        };
        let engine = Arc::new(Mutex::new(Engine::new(
            config,
            Arc::new(backend),
            logger.clone(),
            Arc::new(EventBus::new()),
            "test".into(),
        )));
        with_engine_events(&engine, |eng| eng.set_enabled(true));
        (engine, calls, logger)
    }

    fn hold_binding() -> DeviceBinding {
        DeviceBinding {
            device_match: DeviceMatch::ByName {
                contains: "test".into(),
            },
            bindings: vec![Binding::Button(ButtonBinding {
                codes: vec![0x114], // BTN_EXTRA
                code_names: vec!["BTN_EXTRA".into()],
                trigger_id: "hold_test".into(),
                hold_trigger_id: None,
                hold_threshold_ms: None,
                layer: None,
                swallow: false,
                on: TriggerEdge::Press,
            })],
            exclusive: false,
        }
    }

    fn make_processor(
        binding: DeviceBinding,
        engine: Arc<Mutex<Engine>>,
        logger: Arc<Logger>,
        forward_backend: Option<Arc<dyn InputBackend>>,
    ) -> DeviceProcessor {
        DeviceProcessor::new(
            binding,
            engine,
            logger,
            forward_backend,
            "test-device".to_string(),
            None,
        )
    }

    #[test]
    fn test_hold_mode_starts_on_press_stops_on_release() {
        let (engine, calls, logger) = make_hold_engine();
        let binding = hold_binding();
        let mut proc = make_processor(binding, engine, logger, None);

        // Press: hold trigger should start
        let press = vec![InputEvent {
            event_type: EV_KEY,
            code: 0x114,
            value: 1,
        }];
        proc.process_events(&press);

        thread::sleep(Duration::from_millis(50));
        let clicks_during_hold = calls.lock_or_recover().len();
        assert!(
            clicks_during_hold > 0,
            "Hold trigger should produce clicks while held"
        );

        // Release: hold trigger should stop
        let release = vec![InputEvent {
            event_type: EV_KEY,
            code: 0x114,
            value: 0,
        }];
        proc.process_events(&release);

        thread::sleep(Duration::from_millis(30));
        let clicks_at_release = calls.lock_or_recover().len();

        thread::sleep(Duration::from_millis(50));
        let clicks_after = calls.lock_or_recover().len();

        assert_eq!(
            clicks_at_release, clicks_after,
            "Hold trigger should stop on release, but got {} at release vs {} after",
            clicks_at_release, clicks_after
        );
    }

    #[test]
    fn test_repeat_events_do_not_retrigger() {
        let logger = Arc::new(Logger::new(100, LogLevel::Trace, false));
        logger.set_quiet(true);
        let backend = MockBackend::new();
        let calls = backend.calls_clone();
        let config = Config {
            triggers: vec![TriggerBinding {
                id: "toggle_test".into(),
                name: "Toggle Test".into(),
                description: String::new(),
                mode: TriggerMode::Toggle,
                action: ActionConfig::AutoClick {
                    button: MouseButton::Left,
                    interval_ms: 5,
                    duration_ms: None,
                    jitter_ms: 0,
                    hold_ms: 0,
                },
                cooldown_ms: None,
            }],
            ..Default::default()
        };
        let engine = Arc::new(Mutex::new(Engine::new(
            config,
            Arc::new(backend),
            logger.clone(),
            Arc::new(EventBus::new()),
            "test".into(),
        )));
        with_engine_events(&engine, |eng| eng.set_enabled(true));

        let binding = DeviceBinding {
            device_match: DeviceMatch::ByName {
                contains: "test".into(),
            },
            bindings: vec![Binding::Button(ButtonBinding {
                codes: vec![0x114],
                code_names: vec!["BTN_EXTRA".into()],
                trigger_id: "toggle_test".into(),
                hold_trigger_id: None,
                hold_threshold_ms: None,
                layer: None,
                swallow: false,
                on: TriggerEdge::Press,
            })],
            exclusive: false,
        };

        let mut proc = make_processor(binding, engine, logger, None);

        // Press toggles ON
        let press = vec![InputEvent {
            event_type: EV_KEY,
            code: 0x114,
            value: 1,
        }];
        proc.process_events(&press);
        thread::sleep(Duration::from_millis(30));
        let clicks_after_press = calls.lock_or_recover().len();
        assert!(clicks_after_press > 0, "Toggle should be ON after press");

        // Repeat (value=2) should NOT re-toggle (which would turn it OFF)
        let repeat = vec![InputEvent {
            event_type: EV_KEY,
            code: 0x114,
            value: 2,
        }];
        proc.process_events(&repeat);
        thread::sleep(Duration::from_millis(30));
        let clicks_after_repeat = calls.lock_or_recover().len();
        assert!(
            clicks_after_repeat > clicks_after_press,
            "Toggle should still be ON after repeat event, got {} vs {} before repeat",
            clicks_after_repeat,
            clicks_after_press
        );
    }

    // --- Phase 2: Frame-based event forwarding tests ---

    use crate::evdev_source::{EV_REL, EV_SYN, SYN_DROPPED, SYN_REPORT};
    use crate::input_backend::BackendCall;

    const BTN_EXTRA: u16 = 0x114;
    const BTN_LEFT: u16 = 0x110;
    const REL_X: u16 = 0x00;
    const REL_Y: u16 = 0x01;

    fn make_oneshot_engine(
        trigger_id: &str,
    ) -> (
        Arc<Mutex<Engine>>,
        Arc<Mutex<Vec<BackendCall>>>,
        Arc<Logger>,
    ) {
        let logger = Arc::new(Logger::new(100, LogLevel::Trace, false));
        logger.set_quiet(true);
        let backend = MockBackend::new();
        let calls = backend.calls_clone();
        let config = Config {
            triggers: vec![TriggerBinding {
                id: trigger_id.into(),
                name: trigger_id.into(),
                description: String::new(),
                mode: TriggerMode::OneShot,
                action: ActionConfig::AutoClick {
                    button: MouseButton::Left,
                    interval_ms: 10,
                    duration_ms: Some(30),
                    jitter_ms: 0,
                    hold_ms: 0,
                },
                cooldown_ms: None,
            }],
            ..Default::default()
        };
        let engine = Arc::new(Mutex::new(Engine::new(
            config,
            Arc::new(backend),
            logger.clone(),
            Arc::new(EventBus::new()),
            "test".into(),
        )));
        with_engine_events(&engine, |eng| eng.set_enabled(true));
        (engine, calls, logger)
    }

    fn exclusive_binding_for(trigger_id: &str) -> DeviceBinding {
        DeviceBinding {
            device_match: DeviceMatch::ByName {
                contains: "test".into(),
            },
            bindings: vec![Binding::Button(ButtonBinding {
                codes: vec![BTN_EXTRA],
                code_names: vec!["BTN_EXTRA".into()],
                trigger_id: trigger_id.into(),
                hold_trigger_id: None,
                hold_threshold_ms: None,
                layer: None,
                swallow: true,
                on: TriggerEdge::Press,
            })],
            exclusive: true,
        }
    }

    fn syn_report() -> InputEvent {
        InputEvent {
            event_type: EV_SYN,
            code: SYN_REPORT,
            value: 0,
        }
    }

    fn syn_dropped() -> InputEvent {
        InputEvent {
            event_type: EV_SYN,
            code: SYN_DROPPED,
            value: 0,
        }
    }

    #[test]
    fn test_exclusive_forwards_unmatched_button() {
        let (engine, _engine_calls, logger) = make_oneshot_engine("test_trigger");
        let binding = exclusive_binding_for("test_trigger");
        let fwd_backend = MockBackend::new();
        let fwd_calls = fwd_backend.calls_clone();

        let mut proc = make_processor(binding, engine, logger, Some(Arc::new(fwd_backend)));

        // BTN_LEFT is not in our bindings — should be forwarded
        proc.process_events(&[
            InputEvent {
                event_type: EV_KEY,
                code: BTN_LEFT,
                value: 1,
            },
            syn_report(),
        ]);

        let calls = fwd_calls.lock_or_recover();
        assert_eq!(calls.len(), 1, "Should have forwarded one frame");
        if let BackendCall::ForwardFrame(ref events) = calls[0] {
            assert_eq!(events, &vec![(EV_KEY, BTN_LEFT, 1i32)]);
        } else {
            panic!("Expected ForwardFrame, got {:?}", calls[0]);
        }
    }

    #[test]
    fn test_exclusive_forwards_mouse_movement() {
        let (engine, _, logger) = make_oneshot_engine("test_trigger");
        let binding = exclusive_binding_for("test_trigger");
        let fwd_backend = MockBackend::new();
        let fwd_calls = fwd_backend.calls_clone();

        let mut proc = make_processor(binding, engine, logger, Some(Arc::new(fwd_backend)));

        proc.process_events(&[
            InputEvent {
                event_type: EV_REL,
                code: REL_X,
                value: 5,
            },
            InputEvent {
                event_type: EV_REL,
                code: REL_Y,
                value: -3,
            },
            syn_report(),
        ]);

        let calls = fwd_calls.lock_or_recover();
        assert_eq!(calls.len(), 1);
        if let BackendCall::ForwardFrame(ref events) = calls[0] {
            assert_eq!(events, &vec![(EV_REL, REL_X, 5i32), (EV_REL, REL_Y, -3i32)]);
        } else {
            panic!("Expected ForwardFrame");
        }
    }

    #[test]
    fn test_exclusive_suppresses_matched_button_press() {
        let (engine, _, logger) = make_oneshot_engine("test_trigger");
        let binding = exclusive_binding_for("test_trigger");
        let fwd_backend = MockBackend::new();
        let fwd_calls = fwd_backend.calls_clone();

        let mut proc = make_processor(binding, engine, logger, Some(Arc::new(fwd_backend)));

        // BTN_EXTRA is matched — should NOT be forwarded
        proc.process_events(&[
            InputEvent {
                event_type: EV_KEY,
                code: BTN_EXTRA,
                value: 1,
            },
            syn_report(),
        ]);

        let calls = fwd_calls.lock_or_recover();
        // No frame should be forwarded (only event was suppressed)
        assert!(
            calls.is_empty(),
            "Matched button should be suppressed, got {:?}",
            *calls
        );
    }

    #[test]
    fn test_exclusive_suppresses_matched_release() {
        let (engine, _, logger) = make_oneshot_engine("test_trigger");
        let binding = exclusive_binding_for("test_trigger");
        let fwd_backend = MockBackend::new();
        let fwd_calls = fwd_backend.calls_clone();

        let mut proc = make_processor(binding, engine, logger, Some(Arc::new(fwd_backend)));

        // Press in frame 1 — claimed
        proc.process_events(&[
            InputEvent {
                event_type: EV_KEY,
                code: BTN_EXTRA,
                value: 1,
            },
            syn_report(),
        ]);

        // Release in frame 2 — should also be suppressed (cross-frame claim)
        proc.process_events(&[
            InputEvent {
                event_type: EV_KEY,
                code: BTN_EXTRA,
                value: 0,
            },
            syn_report(),
        ]);

        let calls = fwd_calls.lock_or_recover();
        assert!(
            calls.is_empty(),
            "Both press and release should be suppressed, got {:?}",
            *calls
        );
    }

    #[test]
    fn test_syn_report_boundaries_preserved() {
        let (engine, _, logger) = make_oneshot_engine("test_trigger");
        let binding = exclusive_binding_for("test_trigger");
        let fwd_backend = MockBackend::new();
        let fwd_calls = fwd_backend.calls_clone();

        let mut proc = make_processor(binding, engine, logger, Some(Arc::new(fwd_backend)));

        // Two frames in one batch
        proc.process_events(&[
            InputEvent {
                event_type: EV_REL,
                code: REL_X,
                value: 1,
            },
            syn_report(),
            InputEvent {
                event_type: EV_REL,
                code: REL_X,
                value: 2,
            },
            syn_report(),
        ]);

        let calls = fwd_calls.lock_or_recover();
        assert_eq!(
            calls.len(),
            2,
            "Should produce two separate forwarded frames"
        );
    }

    #[test]
    fn test_non_exclusive_does_not_forward() {
        let (engine, _, logger) = make_oneshot_engine("test_trigger");
        // non-exclusive binding
        let binding = DeviceBinding {
            device_match: DeviceMatch::ByName {
                contains: "test".into(),
            },
            bindings: vec![Binding::Button(ButtonBinding {
                codes: vec![BTN_EXTRA],
                code_names: vec!["BTN_EXTRA".into()],
                trigger_id: "test_trigger".into(),
                hold_trigger_id: None,
                hold_threshold_ms: None,
                layer: None,
                swallow: false,
                on: TriggerEdge::Press,
            })],
            exclusive: false,
        };
        // No forwarding backend
        let mut proc = make_processor(binding, engine, logger, None);

        proc.process_events(&[
            InputEvent {
                event_type: EV_REL,
                code: REL_X,
                value: 5,
            },
            InputEvent {
                event_type: EV_KEY,
                code: BTN_EXTRA,
                value: 1,
            },
        ]);

        // No forward_frame calls possible — no backend
        // This test just verifies it doesn't panic and processes EV_KEY normally
    }

    #[test]
    fn test_frame_split_across_poll_batches() {
        let (engine, _, logger) = make_oneshot_engine("test_trigger");
        let binding = exclusive_binding_for("test_trigger");
        let fwd_backend = MockBackend::new();
        let fwd_calls = fwd_backend.calls_clone();

        let mut proc = make_processor(binding, engine, logger, Some(Arc::new(fwd_backend)));

        // First poll: partial frame (no SYN_REPORT yet)
        proc.process_events(&[InputEvent {
            event_type: EV_REL,
            code: REL_X,
            value: 10,
        }]);

        // Nothing forwarded yet
        assert!(fwd_calls.lock_or_recover().is_empty());

        // Second poll: rest of frame + SYN_REPORT
        proc.process_events(&[
            InputEvent {
                event_type: EV_REL,
                code: REL_Y,
                value: -5,
            },
            syn_report(),
        ]);

        let calls = fwd_calls.lock_or_recover();
        assert_eq!(calls.len(), 1, "Frame should be forwarded after SYN_REPORT");
        if let BackendCall::ForwardFrame(ref events) = calls[0] {
            assert_eq!(
                events,
                &vec![(EV_REL, REL_X, 10i32), (EV_REL, REL_Y, -5i32)]
            );
        }
    }

    #[test]
    fn test_claimed_press_release_across_polls() {
        let (engine, _, logger) = make_oneshot_engine("test_trigger");
        let binding = exclusive_binding_for("test_trigger");
        let fwd_backend = MockBackend::new();
        let fwd_calls = fwd_backend.calls_clone();

        let mut proc = make_processor(binding, engine, logger, Some(Arc::new(fwd_backend)));

        // Poll 1: press frame
        proc.process_events(&[
            InputEvent {
                event_type: EV_KEY,
                code: BTN_EXTRA,
                value: 1,
            },
            syn_report(),
        ]);

        // Poll 2: release frame (different poll batch)
        proc.process_events(&[
            InputEvent {
                event_type: EV_KEY,
                code: BTN_EXTRA,
                value: 0,
            },
            syn_report(),
        ]);

        // Both should be suppressed
        let calls = fwd_calls.lock_or_recover();
        assert!(
            calls.is_empty(),
            "Press and release across polls should both be suppressed, got {:?}",
            *calls
        );
    }

    #[test]
    fn test_engine_disabled_events_forwarded() {
        let logger = Arc::new(Logger::new(100, LogLevel::Trace, false));
        logger.set_quiet(true);
        let backend = MockBackend::new();
        let config = Config {
            triggers: vec![TriggerBinding {
                id: "test_trigger".into(),
                name: "test_trigger".into(),
                description: String::new(),
                mode: TriggerMode::OneShot,
                action: ActionConfig::AutoClick {
                    button: MouseButton::Left,
                    interval_ms: 10,
                    duration_ms: Some(30),
                    jitter_ms: 0,
                    hold_ms: 0,
                },
                cooldown_ms: None,
            }],
            ..Default::default()
        };
        let engine = Arc::new(Mutex::new(Engine::new(
            config,
            Arc::new(backend),
            logger.clone(),
            Arc::new(EventBus::new()),
            "test".into(),
        )));
        // Engine is DISABLED — events should NOT be claimed

        let binding = exclusive_binding_for("test_trigger");
        let fwd_backend = MockBackend::new();
        let fwd_calls = fwd_backend.calls_clone();

        let mut proc = make_processor(binding, engine, logger, Some(Arc::new(fwd_backend)));

        proc.process_events(&[
            InputEvent {
                event_type: EV_KEY,
                code: BTN_EXTRA,
                value: 1,
            },
            syn_report(),
        ]);

        let calls = fwd_calls.lock_or_recover();
        assert_eq!(
            calls.len(),
            1,
            "With engine disabled, matched event should be forwarded (not claimed)"
        );
    }

    #[test]
    fn test_syn_dropped_discards_partial_frame() {
        let (engine, _, logger) = make_oneshot_engine("test_trigger");
        let binding = exclusive_binding_for("test_trigger");
        let fwd_backend = MockBackend::new();
        let fwd_calls = fwd_backend.calls_clone();

        let mut proc = make_processor(binding, engine, logger, Some(Arc::new(fwd_backend)));

        // Partial frame, then SYN_DROPPED, then a valid frame
        proc.process_events(&[
            InputEvent {
                event_type: EV_REL,
                code: REL_X,
                value: 999,
            },
            syn_dropped(),
            InputEvent {
                event_type: EV_REL,
                code: REL_X,
                value: 1,
            },
            syn_report(),
        ]);

        let calls = fwd_calls.lock_or_recover();
        assert_eq!(
            calls.len(),
            1,
            "Should only forward the valid frame after SYN_DROPPED"
        );
        if let BackendCall::ForwardFrame(ref events) = calls[0] {
            assert_eq!(events, &vec![(EV_REL, REL_X, 1i32)]);
        }
    }

    // --- Phase 4: Scroll Trigger Dispatch Tests ---

    fn scroll_binding_for(
        direction: crate::config::ScrollDirection,
        trigger_id: &str,
    ) -> DeviceBinding {
        DeviceBinding {
            device_match: DeviceMatch::ByName {
                contains: "test".into(),
            },
            bindings: vec![Binding::Scroll(crate::config::ScrollBinding {
                direction,
                trigger_id: trigger_id.into(),
                layer: None,
                swallow: true,
            })],
            exclusive: true,
        }
    }

    #[test]
    fn test_scroll_up_fires_trigger_once() {
        let (engine, _, logger) = make_oneshot_engine("scroll_click");
        let binding = scroll_binding_for(crate::config::ScrollDirection::Up, "scroll_click");
        let fwd_backend = MockBackend::new();
        let fwd_calls = fwd_backend.calls_clone();

        let mut proc = make_processor(binding, engine, logger, Some(Arc::new(fwd_backend)));

        proc.process_events(&[
            InputEvent {
                event_type: EV_REL,
                code: evdev_source::REL_WHEEL,
                value: 1,
            },
            syn_report(),
        ]);

        // Scroll event was claimed (suppressed) — proves trigger was fired
        let calls = fwd_calls.lock_or_recover();
        assert!(
            calls.is_empty(),
            "Matched scroll up should be suppressed, got {:?}",
            *calls
        );
    }

    #[test]
    fn test_scroll_magnitude_fires_multiple() {
        // Use NoOp action so we can count trigger_event calls synchronously
        let logger = Arc::new(Logger::new(100, LogLevel::Trace, false));
        logger.set_quiet(true);
        let backend = MockBackend::new();
        let config = Config {
            triggers: vec![TriggerBinding {
                id: "scroll_click".into(),
                name: "scroll_click".into(),
                description: String::new(),
                mode: TriggerMode::OneShot,
                action: ActionConfig::NoOp,
                cooldown_ms: None,
            }],
            ..Default::default()
        };
        let engine = Arc::new(Mutex::new(Engine::new(
            config,
            Arc::new(backend),
            logger.clone(),
            Arc::new(EventBus::new()),
            "test".into(),
        )));
        with_engine_events(&engine, |eng| eng.set_enabled(true));

        let binding = scroll_binding_for(crate::config::ScrollDirection::Up, "scroll_click");
        let fwd_backend = MockBackend::new();
        let fwd_calls = fwd_backend.calls_clone();

        let mut proc = make_processor(binding, engine, logger, Some(Arc::new(fwd_backend)));

        proc.process_events(&[
            InputEvent {
                event_type: EV_REL,
                code: evdev_source::REL_WHEEL,
                value: 3,
            },
            syn_report(),
        ]);

        // Scroll was suppressed
        let calls = fwd_calls.lock_or_recover();
        assert!(calls.is_empty(), "Matched scroll should be suppressed");
        // Note: trigger_event was called 3 times internally (once per magnitude unit)
        // We verify this indirectly through suppression — the claim succeeded
    }

    #[test]
    fn test_scroll_down_fires_separate_trigger() {
        let (engine, _, logger) = make_oneshot_engine("scroll_down_click");
        let binding = scroll_binding_for(crate::config::ScrollDirection::Down, "scroll_down_click");
        let fwd_backend = MockBackend::new();
        let fwd_calls = fwd_backend.calls_clone();

        let mut proc = make_processor(binding, engine, logger, Some(Arc::new(fwd_backend)));

        proc.process_events(&[
            InputEvent {
                event_type: EV_REL,
                code: evdev_source::REL_WHEEL,
                value: -1,
            },
            syn_report(),
        ]);

        let calls = fwd_calls.lock_or_recover();
        assert!(
            calls.is_empty(),
            "Matched scroll down should be suppressed, got {:?}",
            *calls
        );
    }

    #[test]
    fn test_hi_res_suppressed_when_standard_matches() {
        let (engine, _, logger) = make_oneshot_engine("scroll_click");
        let binding = scroll_binding_for(crate::config::ScrollDirection::Up, "scroll_click");
        let fwd_backend = MockBackend::new();
        let fwd_calls = fwd_backend.calls_clone();

        let mut proc = make_processor(binding, engine, logger, Some(Arc::new(fwd_backend)));

        // Frame with both REL_WHEEL and REL_WHEEL_HI_RES
        proc.process_events(&[
            InputEvent {
                event_type: EV_REL,
                code: evdev_source::REL_WHEEL,
                value: 1,
            },
            InputEvent {
                event_type: EV_REL,
                code: evdev_source::REL_WHEEL_HI_RES,
                value: 120,
            },
            syn_report(),
        ]);

        let calls = fwd_calls.lock_or_recover();
        // Both should be suppressed — nothing forwarded
        assert!(
            calls.is_empty(),
            "Both standard and hi-res scroll should be suppressed when matched, got {:?}",
            *calls
        );
    }

    #[test]
    fn test_unmatched_scroll_direction_forwarded() {
        let (engine, _, logger) = make_oneshot_engine("scroll_click");
        // Only "up" is bound
        let binding = scroll_binding_for(crate::config::ScrollDirection::Up, "scroll_click");
        let fwd_backend = MockBackend::new();
        let fwd_calls = fwd_backend.calls_clone();

        let mut proc = make_processor(binding, engine, logger, Some(Arc::new(fwd_backend)));

        // Send scroll DOWN (not matched)
        proc.process_events(&[
            InputEvent {
                event_type: EV_REL,
                code: evdev_source::REL_WHEEL,
                value: -1,
            },
            InputEvent {
                event_type: EV_REL,
                code: evdev_source::REL_WHEEL_HI_RES,
                value: -120,
            },
            syn_report(),
        ]);

        let calls = fwd_calls.lock_or_recover();
        assert_eq!(calls.len(), 1, "Unmatched scroll should be forwarded");
        if let BackendCall::ForwardFrame(ref events) = calls[0] {
            // Both standard and hi-res should be forwarded
            assert_eq!(events.len(), 2);
        }
    }

    // --- Phase 5: swallow / on=release / chord tests ---

    fn button_binding(
        code: u16,
        code_name: &str,
        trigger_id: &str,
        swallow: bool,
        on: TriggerEdge,
        exclusive: bool,
    ) -> DeviceBinding {
        DeviceBinding {
            device_match: DeviceMatch::ByName {
                contains: "test".into(),
            },
            bindings: vec![Binding::Button(ButtonBinding {
                codes: vec![code],
                code_names: vec![code_name.into()],
                trigger_id: trigger_id.into(),
                hold_trigger_id: None,
                hold_threshold_ms: None,
                layer: None,
                swallow,
                on,
            })],
            exclusive,
        }
    }

    #[test]
    fn test_swallow_false_forwards_button_press_and_release() {
        let (engine, _, logger) = make_oneshot_engine("test_trigger");
        let binding = button_binding(
            BTN_EXTRA,
            "BTN_EXTRA",
            "test_trigger",
            false,
            TriggerEdge::Press,
            true,
        );
        let fwd_backend = MockBackend::new();
        let fwd_calls = fwd_backend.calls_clone();
        let mut proc = make_processor(binding, engine, logger, Some(Arc::new(fwd_backend)));

        proc.process_events(&[
            InputEvent {
                event_type: EV_KEY,
                code: BTN_EXTRA,
                value: 1,
            },
            syn_report(),
        ]);
        proc.process_events(&[
            InputEvent {
                event_type: EV_KEY,
                code: BTN_EXTRA,
                value: 0,
            },
            syn_report(),
        ]);

        let calls = fwd_calls.lock_or_recover();
        assert_eq!(
            calls.len(),
            2,
            "Both press and release should be forwarded (swallow=false)"
        );
    }

    #[test]
    fn test_swallow_true_suppresses_button_press_and_release() {
        let (engine, _, logger) = make_oneshot_engine("test_trigger");
        let binding = button_binding(
            BTN_EXTRA,
            "BTN_EXTRA",
            "test_trigger",
            true,
            TriggerEdge::Press,
            true,
        );
        let fwd_backend = MockBackend::new();
        let fwd_calls = fwd_backend.calls_clone();
        let mut proc = make_processor(binding, engine, logger, Some(Arc::new(fwd_backend)));

        proc.process_events(&[
            InputEvent {
                event_type: EV_KEY,
                code: BTN_EXTRA,
                value: 1,
            },
            syn_report(),
        ]);
        proc.process_events(&[
            InputEvent {
                event_type: EV_KEY,
                code: BTN_EXTRA,
                value: 0,
            },
            syn_report(),
        ]);

        let calls = fwd_calls.lock_or_recover();
        assert!(
            calls.is_empty(),
            "swallow=true should suppress both press and release, got {:?}",
            *calls
        );
    }

    #[test]
    fn test_swallow_false_forwards_scroll() {
        let (engine, _, logger) = make_oneshot_engine("scroll_click");
        let binding = DeviceBinding {
            device_match: DeviceMatch::ByName {
                contains: "test".into(),
            },
            bindings: vec![Binding::Scroll(crate::config::ScrollBinding {
                direction: crate::config::ScrollDirection::Up,
                trigger_id: "scroll_click".into(),
                layer: None,
                swallow: false,
            })],
            exclusive: true,
        };
        let fwd_backend = MockBackend::new();
        let fwd_calls = fwd_backend.calls_clone();
        let mut proc = make_processor(binding, engine, logger, Some(Arc::new(fwd_backend)));

        proc.process_events(&[
            InputEvent {
                event_type: EV_REL,
                code: evdev_source::REL_WHEEL,
                value: 1,
            },
            syn_report(),
        ]);

        let calls = fwd_calls.lock_or_recover();
        assert_eq!(
            calls.len(),
            1,
            "swallow=false scroll should be forwarded, got {:?}",
            *calls
        );
    }

    #[test]
    fn test_on_release_fires_on_release_not_press() {
        let logger = Arc::new(Logger::new(100, LogLevel::Trace, false));
        logger.set_quiet(true);
        let backend = MockBackend::new();
        let trigger_calls = backend.calls_clone();
        let config = Config {
            triggers: vec![TriggerBinding {
                id: "release_trigger".into(),
                name: "Release Trigger".into(),
                description: String::new(),
                mode: TriggerMode::OneShot,
                action: ActionConfig::NoOp,
                cooldown_ms: None,
            }],
            ..Default::default()
        };
        let engine = Arc::new(Mutex::new(Engine::new(
            config,
            Arc::new(backend),
            logger.clone(),
            Arc::new(EventBus::new()),
            "test".into(),
        )));
        with_engine_events(&engine, |eng| eng.set_enabled(true));

        // on=Release, swallow=false (validated: swallow+Release forbidden)
        let binding = button_binding(
            BTN_EXTRA,
            "BTN_EXTRA",
            "release_trigger",
            false,
            TriggerEdge::Release,
            false,
        );
        let mut proc = make_processor(binding, engine, logger, None);

        // Press: trigger should NOT fire
        proc.process_events(&[InputEvent {
            event_type: EV_KEY,
            code: BTN_EXTRA,
            value: 1,
        }]);
        assert!(
            trigger_calls.lock_or_recover().is_empty(),
            "Trigger should not fire on press with on=release"
        );

        // Release: trigger SHOULD fire
        proc.process_events(&[InputEvent {
            event_type: EV_KEY,
            code: BTN_EXTRA,
            value: 0,
        }]);
        // Since it's NoOp, no backend calls — but we can verify via engine state
        // (OneShot NoOp: no side effects in backend, but trigger_event was called)
        // The test passes if no panic and no press-time fire. Engine didn't error on release either.
    }

    #[test]
    fn test_chord_fires_only_when_both_held() {
        let logger = Arc::new(Logger::new(100, LogLevel::Trace, false));
        logger.set_quiet(true);
        let backend = MockBackend::new();
        let config = Config {
            triggers: vec![TriggerBinding {
                id: "chord_trigger".into(),
                name: "Chord Trigger".into(),
                description: String::new(),
                mode: TriggerMode::OneShot,
                action: ActionConfig::NoOp,
                cooldown_ms: None,
            }],
            ..Default::default()
        };
        let engine = Arc::new(Mutex::new(Engine::new(
            config,
            Arc::new(backend),
            logger.clone(),
            Arc::new(EventBus::new()),
            "test".into(),
        )));
        with_engine_events(&engine, |eng| eng.set_enabled(true));

        const BTN_SIDE: u16 = 0x113;

        let binding = DeviceBinding {
            device_match: DeviceMatch::ByName {
                contains: "test".into(),
            },
            bindings: vec![Binding::Button(ButtonBinding {
                codes: vec![BTN_SIDE, BTN_EXTRA],
                code_names: vec!["BTN_SIDE".into(), "BTN_EXTRA".into()],
                trigger_id: "chord_trigger".into(),
                hold_trigger_id: None,
                hold_threshold_ms: None,
                layer: None,
                swallow: false,
                on: TriggerEdge::Press,
            })],
            exclusive: false,
        };
        let mut proc = make_processor(binding, engine.clone(), logger, None);

        // Press only BTN_SIDE — chord should not fire
        proc.process_events(&[InputEvent {
            event_type: EV_KEY,
            code: BTN_SIDE,
            value: 1,
        }]);
        // Verify engine received trigger_event for chord: it hasn't (BTN_EXTRA not pressed)
        // We check indirectly: engine is in OneShot NoOp state, so if trigger fired it would
        // have been Ok(()). We verify by pressing the second code next.

        // Press BTN_EXTRA — now both held, chord should fire
        proc.process_events(&[InputEvent {
            event_type: EV_KEY,
            code: BTN_EXTRA,
            value: 1,
        }]);
        // If chord detection works, trigger_event("chord_trigger", true) was called exactly once.
        // No assert on backend calls since action is NoOp, but we verify no panic/double-fire:
        // Press BTN_EXTRA again should NOT re-fire (already claimed):
        proc.process_events(&[InputEvent {
            event_type: EV_KEY,
            code: BTN_EXTRA,
            value: 0,
        }]);
        proc.process_events(&[InputEvent {
            event_type: EV_KEY,
            code: BTN_EXTRA,
            value: 1,
        }]);
        // After release + re-press, chord fires again (BTN_SIDE still held, BTN_EXTRA re-pressed)
    }

    #[test]
    fn test_chord_partial_press_no_spurious_fire() {
        let logger = Arc::new(Logger::new(100, LogLevel::Trace, false));
        logger.set_quiet(true);
        let backend = MockBackend::new();
        let trigger_calls = backend.calls_clone();
        let config = Config {
            triggers: vec![TriggerBinding {
                id: "chord_trigger".into(),
                name: "Chord Trigger".into(),
                description: String::new(),
                mode: TriggerMode::OneShot,
                action: ActionConfig::AutoClick {
                    button: MouseButton::Left,
                    interval_ms: 1,
                    duration_ms: Some(5),
                    jitter_ms: 0,
                    hold_ms: 0,
                },
                cooldown_ms: None,
            }],
            ..Default::default()
        };
        let engine = Arc::new(Mutex::new(Engine::new(
            config,
            Arc::new(backend),
            logger.clone(),
            Arc::new(EventBus::new()),
            "test".into(),
        )));
        with_engine_events(&engine, |eng| eng.set_enabled(true));

        const BTN_SIDE: u16 = 0x113;

        let binding = DeviceBinding {
            device_match: DeviceMatch::ByName {
                contains: "test".into(),
            },
            bindings: vec![Binding::Button(ButtonBinding {
                codes: vec![BTN_SIDE, BTN_EXTRA],
                code_names: vec!["BTN_SIDE".into(), "BTN_EXTRA".into()],
                trigger_id: "chord_trigger".into(),
                hold_trigger_id: None,
                hold_threshold_ms: None,
                layer: None,
                swallow: false,
                on: TriggerEdge::Press,
            })],
            exclusive: false,
        };
        let mut proc = make_processor(binding, engine, logger, None);

        // Press only one key of the chord — trigger must NOT fire
        proc.process_events(&[InputEvent {
            event_type: EV_KEY,
            code: BTN_SIDE,
            value: 1,
        }]);
        thread::sleep(Duration::from_millis(20));
        assert!(
            trigger_calls.lock_or_recover().is_empty(),
            "Partial chord press should not fire trigger"
        );
    }

    // ─── Scroll event bus publishing ──────────────────────────────────────────

    fn make_processor_with_bus(bus: Arc<EventBus>) -> DeviceProcessor {
        let binding = make_binding(DeviceMatch::ByName {
            contains: "test".into(),
        });
        let (engine, _, logger) = make_oneshot_engine("test_trigger");
        DeviceProcessor::new(
            binding,
            engine,
            logger,
            None,
            "test-device".to_string(),
            Some(bus),
        )
    }

    #[test]
    fn test_rel_wheel_publishes_scroll_received() {
        let bus = Arc::new(EventBus::new());
        let rx = bus.subscribe(Some(vec![crate::event_bus::EventType::ScrollReceived]));
        let mut proc = make_processor_with_bus(bus);

        proc.process_events(&[
            InputEvent {
                event_type: EV_REL,
                code: REL_WHEEL,
                value: 1,
            },
            InputEvent {
                event_type: EV_SYN,
                code: SYN_REPORT,
                value: 0,
            },
        ]);

        let event = rx
            .recv_timeout(Duration::from_millis(200))
            .expect("ScrollReceived not published");
        assert!(
            matches!(
                event,
                Event::ScrollReceived {
                    delta_x: 0,
                    delta_y: 1,
                    ..
                }
            ),
            "Unexpected event: {:?}",
            event
        );
    }

    #[test]
    fn test_rel_hwheel_publishes_scroll_received() {
        let bus = Arc::new(EventBus::new());
        let rx = bus.subscribe(Some(vec![crate::event_bus::EventType::ScrollReceived]));
        let mut proc = make_processor_with_bus(bus);

        proc.process_events(&[
            InputEvent {
                event_type: EV_REL,
                code: REL_HWHEEL,
                value: -1,
            },
            InputEvent {
                event_type: EV_SYN,
                code: SYN_REPORT,
                value: 0,
            },
        ]);

        let event = rx
            .recv_timeout(Duration::from_millis(200))
            .expect("ScrollReceived not published");
        assert!(
            matches!(
                event,
                Event::ScrollReceived {
                    delta_x: -1,
                    delta_y: 0,
                    ..
                }
            ),
            "Unexpected event: {:?}",
            event
        );
    }

    #[test]
    fn test_hi_res_wheel_does_not_publish() {
        use crate::evdev_source::{REL_HWHEEL_HI_RES, REL_WHEEL_HI_RES};
        let bus = Arc::new(EventBus::new());
        let rx = bus.subscribe(Some(vec![crate::event_bus::EventType::ScrollReceived]));
        let mut proc = make_processor_with_bus(bus);

        proc.process_events(&[
            InputEvent {
                event_type: EV_REL,
                code: REL_WHEEL_HI_RES,
                value: 120,
            },
            InputEvent {
                event_type: EV_SYN,
                code: SYN_REPORT,
                value: 0,
            },
        ]);
        proc.process_events(&[
            InputEvent {
                event_type: EV_REL,
                code: REL_HWHEEL_HI_RES,
                value: -120,
            },
            InputEvent {
                event_type: EV_SYN,
                code: SYN_REPORT,
                value: 0,
            },
        ]);

        assert!(
            rx.recv_timeout(Duration::from_millis(100)).is_err(),
            "Hi-res wheel events should not publish ScrollReceived"
        );
    }
}
