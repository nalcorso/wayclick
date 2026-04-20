// EvdevMonitor — coordinates device monitoring threads, hotplug, and trigger dispatch.

use crate::config::DeviceBinding;
use crate::engine::Engine;
use crate::evdev_source::{self, DeviceInfo, EvdevSource, InputSource, EV_KEY};
use crate::logger::Logger;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

pub struct EvdevMonitor {
    engine: Arc<Mutex<Engine>>,
    logger: Arc<Logger>,
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
            config_bindings: Vec::new(),
            running: Arc::new(AtomicBool::new(false)),
            device_threads: Vec::new(),
            scan_thread: None,
            tracked_devices: Arc::new(Mutex::new(HashMap::new())),
        }
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

        self.scan_thread = Some(thread::spawn(move || {
            while running.load(Ordering::SeqCst) {
                thread::sleep(Duration::from_secs(2));
                if !running.load(Ordering::SeqCst) {
                    break;
                }
                // Scan for new devices
                let devices = evdev_source::enumerate_devices();
                for dev in &devices {
                    let already_tracked = tracked.lock().unwrap().contains_key(&dev.path);
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
                            tracked.lock().unwrap().insert(dev.path.clone(), ());
                            spawn_device_thread(
                                dev.path.clone(),
                                binding.exclusive,
                                binding.clone(),
                                engine.clone(),
                                logger.clone(),
                                running.clone(),
                                tracked.clone(),
                            );
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

                    let handle = spawn_device_thread(
                        dev.path.clone(),
                        binding.exclusive,
                        binding.clone(),
                        self.engine.clone(),
                        self.logger.clone(),
                        self.running.clone(),
                        self.tracked_devices.clone(),
                    );
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

        self.tracked_devices.lock().unwrap().clear();
        self.logger.info("EvdevMonitor: stopped");
    }
}

fn spawn_device_thread(
    path: PathBuf,
    exclusive: bool,
    binding: DeviceBinding,
    engine: Arc<Mutex<Engine>>,
    logger: Arc<Logger>,
    running: Arc<AtomicBool>,
    tracked: Arc<Mutex<HashMap<PathBuf, ()>>>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let mut source = match EvdevSource::open(&path, exclusive) {
            Ok(s) => s,
            Err(e) => {
                logger.warn(format!("Failed to open {:?}: {}", path, e));
                tracked.lock().unwrap().remove(&path);
                return;
            }
        };

        logger.debug(format!(
            "Monitoring device '{}' at {:?}",
            source.device_info().name,
            path
        ));

        while running.load(Ordering::SeqCst) {
            match source.poll_events(Duration::from_millis(100)) {
                Ok(events) => {
                    process_device_events(&events, &binding, &engine, &logger);
                }
                Err(crate::evdev_source::SourceError::Disconnected) => {
                    logger.warn(format!("Device {:?} disconnected", path));
                    tracked.lock().unwrap().remove(&path);
                    break;
                }
                Err(e) => {
                    logger.warn(format!("Read error on {:?}: {}", path, e));
                    tracked.lock().unwrap().remove(&path);
                    break;
                }
            }
        }

        source.close();
    })
}

/// Process a batch of raw evdev events — extracted for testability.
/// Handles EV_KEY press/release dispatch and filters repeat events.
fn process_device_events(
    events: &[evdev_source::InputEvent],
    binding: &DeviceBinding,
    engine: &Arc<Mutex<Engine>>,
    logger: &Arc<Logger>,
) {
    for event in events {
        if event.event_type == EV_KEY {
            match event.value {
                1 => dispatch_button_event(event.code, true, binding, engine, logger),
                0 => dispatch_button_event(event.code, false, binding, engine, logger),
                _ => {} // value==2 (repeat) — ignore
            }
        }
    }
}

fn dispatch_button_event(
    code: u16,
    press: bool,
    binding: &DeviceBinding,
    engine: &Arc<Mutex<Engine>>,
    logger: &Arc<Logger>,
) {
    for bb in &binding.button_bindings {
        // Simple single-button match (chord/hold handled in evdev-monitor-rewrite)
        if bb.codes.len() == 1 && bb.codes[0] == code {
            // Layer filtering
            if let Some(ref layer) = bb.layer {
                let eng = engine.lock().unwrap();
                if eng.current_layer() != layer {
                    continue;
                }
            }
            let code_name = bb.code_names.first().map(|s| s.as_str()).unwrap_or("?");
            logger.debug(format!(
                "Button {} {}, firing trigger '{}'",
                code_name,
                if press { "pressed" } else { "released" },
                bb.trigger_id
            ));
            let mut eng = engine.lock().unwrap();
            let _ = eng.trigger_event(&bb.trigger_id, press);
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
            button_bindings: vec![],
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

    fn make_hold_engine() -> (Arc<Mutex<Engine>>, Arc<Mutex<Vec<crate::input_backend::BackendCall>>>, Arc<Logger>) {
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
            "test".into(),
        )));
        engine.lock().unwrap().set_enabled(true);
        (engine, calls, logger)
    }

    fn hold_binding() -> DeviceBinding {
        DeviceBinding {
            device_match: DeviceMatch::ByName {
                contains: "test".into(),
            },
            button_bindings: vec![ButtonBinding {
                codes: vec![0x114], // BTN_EXTRA
                code_names: vec!["BTN_EXTRA".into()],
                trigger_id: "hold_test".into(),
                hold_trigger_id: None,
                hold_threshold_ms: None,
                layer: None,
            }],
            exclusive: false,
        }
    }

    #[test]
    fn test_hold_mode_starts_on_press_stops_on_release() {
        let (engine, calls, logger) = make_hold_engine();
        let binding = hold_binding();

        // Press: hold trigger should start
        let press = vec![InputEvent {
            event_type: EV_KEY,
            code: 0x114,
            value: 1,
        }];
        process_device_events(&press, &binding, &engine, &logger);

        // Wait for some clicks to accumulate
        thread::sleep(Duration::from_millis(50));
        let clicks_during_hold = calls.lock().unwrap().len();
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
        process_device_events(&release, &binding, &engine, &logger);

        // Allow worker to wind down
        thread::sleep(Duration::from_millis(30));
        let clicks_at_release = calls.lock().unwrap().len();

        // Wait and ensure no more clicks
        thread::sleep(Duration::from_millis(50));
        let clicks_after = calls.lock().unwrap().len();

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
            "test".into(),
        )));
        engine.lock().unwrap().set_enabled(true);

        let binding = DeviceBinding {
            device_match: DeviceMatch::ByName {
                contains: "test".into(),
            },
            button_bindings: vec![ButtonBinding {
                codes: vec![0x114],
                code_names: vec!["BTN_EXTRA".into()],
                trigger_id: "toggle_test".into(),
                hold_trigger_id: None,
                hold_threshold_ms: None,
                layer: None,
            }],
            exclusive: false,
        };

        // Press toggles ON
        let press = vec![InputEvent {
            event_type: EV_KEY,
            code: 0x114,
            value: 1,
        }];
        process_device_events(&press, &binding, &engine, &logger);
        thread::sleep(Duration::from_millis(30));
        let clicks_after_press = calls.lock().unwrap().len();
        assert!(clicks_after_press > 0, "Toggle should be ON after press");

        // Repeat (value=2) should NOT re-toggle (which would turn it OFF)
        let repeat = vec![InputEvent {
            event_type: EV_KEY,
            code: 0x114,
            value: 2,
        }];
        process_device_events(&repeat, &binding, &engine, &logger);
        thread::sleep(Duration::from_millis(30));
        let clicks_after_repeat = calls.lock().unwrap().len();
        assert!(
            clicks_after_repeat > clicks_after_press,
            "Toggle should still be ON after repeat event, got {} vs {} before repeat",
            clicks_after_repeat,
            clicks_after_press
        );
    }
}
