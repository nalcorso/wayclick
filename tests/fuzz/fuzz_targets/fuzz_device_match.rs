// SPDX-License-Identifier: MIT
#![no_main]
use libfuzzer_sys::fuzz_target;
use std::path::PathBuf;
use wayclick_core::config::{DeviceBinding, DeviceMatch};
use wayclick_core::evdev_monitor::match_device;
use wayclick_core::evdev_source::DeviceInfo;

fuzz_target!(|data: &[u8]| {
    if data.len() < 10 {
        return;
    }

    // Build a DeviceInfo from fuzz data
    let name_len = (data[0] as usize).min(data.len() - 5);
    let name = String::from_utf8_lossy(&data[1..1 + name_len]).to_string();
    let rest = &data[1 + name_len..];

    if rest.len() < 4 {
        return;
    }

    let vendor_id = u16::from_le_bytes([rest[0], rest[1]]);
    let product_id = u16::from_le_bytes([rest[2], rest[3]]);

    let info = DeviceInfo {
        path: PathBuf::from("/dev/input/event0"),
        name: name.clone(),
        vendor_id,
        product_id,
        phys: String::new(),
    };

    // Test various match types — must not panic
    let binding_name = DeviceBinding {
        device_match: DeviceMatch::ByName {
            contains: name.clone(),
        },
        bindings: vec![],
        exclusive: false,
    };
    let _ = match_device(&info, &binding_name);

    let binding_vid = DeviceBinding {
        device_match: DeviceMatch::ByVidPid {
            vendor: vendor_id,
            product: product_id,
        },
        bindings: vec![],
        exclusive: false,
    };
    let _ = match_device(&info, &binding_vid);

    let binding_any = DeviceBinding {
        device_match: DeviceMatch::Any {
            matchers: vec![
                DeviceMatch::ByName {
                    contains: name.clone(),
                },
                DeviceMatch::ByVidPid {
                    vendor: vendor_id,
                    product: product_id,
                },
            ],
        },
        bindings: vec![],
        exclusive: false,
    };
    let _ = match_device(&info, &binding_any);
});
