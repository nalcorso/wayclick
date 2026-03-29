// EvdevMonitor — Phase 3 implementation
// Coordinates device monitoring threads and hotplug events.

use crate::config::DeviceBinding;
use crate::engine::Engine;
use crate::evdev_source::DeviceInfo;
use crate::logger::Logger;
use std::sync::{Arc, Mutex};

#[allow(dead_code)]
pub struct EvdevMonitor {
    engine: Arc<Mutex<Engine>>,
    logger: Arc<Logger>,
    config_bindings: Vec<DeviceBinding>,
}

impl EvdevMonitor {
    pub fn new(engine: Arc<Mutex<Engine>>, logger: Arc<Logger>) -> Self {
        Self {
            engine,
            logger,
            config_bindings: Vec::new(),
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
        self.logger
            .info("EvdevMonitor: device monitoring not yet implemented (Phase 3)");
    }

    pub fn stop(&mut self) {
        self.logger.info("EvdevMonitor: stopped");
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
    use std::path::PathBuf;

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
}
