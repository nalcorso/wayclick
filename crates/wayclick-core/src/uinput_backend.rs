// UinputBackend — emits real input events via /dev/uinput
//
// This module is gated behind the `integration` feature for tests since it
// requires /dev/uinput to be writable.

use crate::config::{MouseButton, ScrollDirection};
use crate::input_backend::{BackendError, InputBackend};
use crate::logger::Logger;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::unix::io::AsRawFd;
use std::sync::{Arc, Mutex};

// Linux input event constants
const EV_SYN: u16 = 0x00;
const EV_KEY: u16 = 0x01;
const EV_REL: u16 = 0x02;
const SYN_REPORT: u16 = 0x00;
const REL_X: u16 = 0x00;
const REL_Y: u16 = 0x01;
const REL_WHEEL: u16 = 0x08;
const REL_HWHEEL: u16 = 0x06;

const BTN_LEFT: u16 = 0x110;
const BTN_RIGHT: u16 = 0x111;
const BTN_MIDDLE: u16 = 0x112;
const BTN_SIDE: u16 = 0x113;
const BTN_EXTRA: u16 = 0x114;

// uinput ioctl constants
const UINPUT_IOCTL_BASE: u8 = b'U';

// Struct sizes for uinput_setup
const UINPUT_MAX_NAME_SIZE: usize = 80;
const BUS_USB: u16 = 0x03;

/// Raw input_event struct matching the kernel layout
#[repr(C)]
#[derive(Copy, Clone, Default)]
struct InputEvent {
    time_sec: u64,
    time_usec: u64,
    type_: u16,
    code: u16,
    value: i32,
}

/// uinput_setup struct
#[repr(C)]
#[derive(Copy, Clone)]
struct UinputSetup {
    id: InputId,
    name: [u8; UINPUT_MAX_NAME_SIZE],
    ff_effects_max: u32,
}

#[repr(C)]
#[derive(Copy, Clone, Default)]
struct InputId {
    bustype: u16,
    vendor: u16,
    product: u16,
    version: u16,
}

// ioctl request codes (computed from _IOW macros)
// UI_SET_EVBIT  = _IOW('U', 100, int)
// UI_SET_KEYBIT = _IOW('U', 101, int)
// UI_SET_RELBIT = _IOW('U', 102, int)
// UI_DEV_SETUP  = _IOW('U', 3, struct uinput_setup)
// UI_DEV_CREATE = _IO('U', 1)
// UI_DEV_DESTROY = _IO('U', 2)

nix::ioctl_write_int!(ui_set_evbit, UINPUT_IOCTL_BASE, 100);
nix::ioctl_write_int!(ui_set_keybit, UINPUT_IOCTL_BASE, 101);
nix::ioctl_write_int!(ui_set_relbit, UINPUT_IOCTL_BASE, 102);
nix::ioctl_none!(ui_dev_create, UINPUT_IOCTL_BASE, 1);
nix::ioctl_none!(ui_dev_destroy, UINPUT_IOCTL_BASE, 2);

// UI_DEV_SETUP = _IOW('U', 3, sizeof(uinput_setup))
// sizeof(uinput_setup) = 92 on 64-bit
nix::ioctl_write_ptr!(ui_dev_setup, UINPUT_IOCTL_BASE, 3, UinputSetup);

pub struct UinputBackend {
    logger: Arc<Logger>,
    file: Mutex<Option<File>>,
}

impl UinputBackend {
    pub fn new(logger: Arc<Logger>) -> Self {
        Self {
            logger,
            file: Mutex::new(None),
        }
    }

    fn write_event(&self, type_: u16, code: u16, value: i32) -> Result<(), BackendError> {
        let event = InputEvent {
            time_sec: 0,
            time_usec: 0,
            type_,
            code,
            value,
        };
        let bytes: &[u8] = unsafe {
            std::slice::from_raw_parts(
                &event as *const InputEvent as *const u8,
                std::mem::size_of::<InputEvent>(),
            )
        };
        let mut guard = self.file.lock().unwrap();
        let file = guard.as_mut().ok_or(BackendError::NotInitialized)?;
        file.write_all(bytes)?;
        Ok(())
    }

    fn syn(&self) -> Result<(), BackendError> {
        self.write_event(EV_SYN, SYN_REPORT, 0)
    }
}

impl InputBackend for UinputBackend {
    fn init(&mut self) -> Result<(), BackendError> {
        let file = OpenOptions::new()
            .write(true)
            .open("/dev/uinput")
            .map_err(|e| {
                BackendError::Other(format!(
                    "Cannot open /dev/uinput: {}. Check permissions (see PERMISSIONS.md)",
                    e
                ))
            })?;

        let fd = file.as_raw_fd();

        unsafe {
            // Enable event types
            ui_set_evbit(fd, EV_KEY as _)
                .map_err(|e| BackendError::Other(format!("UI_SET_EVBIT EV_KEY: {}", e)))?;
            ui_set_evbit(fd, EV_REL as _)
                .map_err(|e| BackendError::Other(format!("UI_SET_EVBIT EV_REL: {}", e)))?;
            ui_set_evbit(fd, EV_SYN as _)
                .map_err(|e| BackendError::Other(format!("UI_SET_EVBIT EV_SYN: {}", e)))?;

            // Enable buttons
            for btn in [BTN_LEFT, BTN_RIGHT, BTN_MIDDLE, BTN_SIDE, BTN_EXTRA] {
                ui_set_keybit(fd, btn as _)
                    .map_err(|e| BackendError::Other(format!("UI_SET_KEYBIT {}: {}", btn, e)))?;
            }

            // Enable common keyboard keys (KEY_ESC=1 through KEY_MAX)
            for key in 1..=248u16 {
                let _ = ui_set_keybit(fd, key as _);
            }

            // Enable relative axes
            for rel in [REL_X, REL_Y, REL_WHEEL, REL_HWHEEL] {
                ui_set_relbit(fd, rel as _)
                    .map_err(|e| BackendError::Other(format!("UI_SET_RELBIT {}: {}", rel, e)))?;
            }

            // Set up device info
            let mut setup = UinputSetup {
                id: InputId {
                    bustype: BUS_USB,
                    vendor: 0x1d6b,  // Linux Foundation
                    product: 0x0001,
                    version: 1,
                },
                name: [0u8; UINPUT_MAX_NAME_SIZE],
                ff_effects_max: 0,
            };
            let name = b"wayclick-virtual-pointer";
            setup.name[..name.len()].copy_from_slice(name);

            ui_dev_setup(fd, &setup)
                .map_err(|e| BackendError::Other(format!("UI_DEV_SETUP: {}", e)))?;
            ui_dev_create(fd)
                .map_err(|e| BackendError::Other(format!("UI_DEV_CREATE: {}", e)))?;
        }

        // Small delay to let the device register
        std::thread::sleep(std::time::Duration::from_millis(100));

        *self.file.lock().unwrap() = Some(file);
        self.logger.info("UinputBackend initialized: wayclick-virtual-pointer");
        Ok(())
    }

    fn click(&self, button: MouseButton) -> Result<(), BackendError> {
        let code = button.event_code();
        // Press
        self.write_event(EV_KEY, code, 1)?;
        self.syn()?;
        // Release
        self.write_event(EV_KEY, code, 0)?;
        self.syn()?;
        Ok(())
    }

    fn key_press(&self, key_code: u32) -> Result<(), BackendError> {
        self.write_event(EV_KEY, key_code as u16, 1)?;
        self.syn()?;
        Ok(())
    }

    fn key_release(&self, key_code: u32) -> Result<(), BackendError> {
        self.write_event(EV_KEY, key_code as u16, 0)?;
        self.syn()?;
        Ok(())
    }

    fn scroll(&self, direction: ScrollDirection, amount: i32) -> Result<(), BackendError> {
        let (axis, val) = match direction {
            ScrollDirection::Up => (REL_WHEEL, amount),
            ScrollDirection::Down => (REL_WHEEL, -amount),
            ScrollDirection::Right => (REL_HWHEEL, amount),
            ScrollDirection::Left => (REL_HWHEEL, -amount),
        };
        self.write_event(EV_REL, axis, val)?;
        self.syn()?;
        Ok(())
    }

    fn move_relative(&self, dx: i32, dy: i32) -> Result<(), BackendError> {
        if dx != 0 {
            self.write_event(EV_REL, REL_X, dx)?;
        }
        if dy != 0 {
            self.write_event(EV_REL, REL_Y, dy)?;
        }
        self.syn()?;
        Ok(())
    }

    fn name(&self) -> &str {
        "uinput"
    }
}

impl Drop for UinputBackend {
    fn drop(&mut self) {
        let guard = self.file.lock().unwrap();
        if let Some(ref file) = *guard {
            unsafe {
                let _ = ui_dev_destroy(file.as_raw_fd());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logger::LogLevel;

    #[test]
    #[cfg_attr(not(feature = "integration"), ignore)]
    fn test_uinput_init() {
        let logger = Arc::new(Logger::new(100, LogLevel::Info, false));
        let mut backend = UinputBackend::new(logger);
        backend.init().unwrap();
    }

    #[test]
    #[cfg_attr(not(feature = "integration"), ignore)]
    fn test_uinput_click_left() {
        let logger = Arc::new(Logger::new(100, LogLevel::Info, false));
        let mut backend = UinputBackend::new(logger);
        backend.init().unwrap();
        backend.click(MouseButton::Left).unwrap();
    }

    #[test]
    #[cfg_attr(not(feature = "integration"), ignore)]
    fn test_uinput_click_right() {
        let logger = Arc::new(Logger::new(100, LogLevel::Info, false));
        let mut backend = UinputBackend::new(logger);
        backend.init().unwrap();
        backend.click(MouseButton::Right).unwrap();
    }

    #[test]
    #[cfg_attr(not(feature = "integration"), ignore)]
    fn test_uinput_click_middle() {
        let logger = Arc::new(Logger::new(100, LogLevel::Info, false));
        let mut backend = UinputBackend::new(logger);
        backend.init().unwrap();
        backend.click(MouseButton::Middle).unwrap();
    }

    #[test]
    #[cfg_attr(not(feature = "integration"), ignore)]
    fn test_uinput_scroll_up() {
        let logger = Arc::new(Logger::new(100, LogLevel::Info, false));
        let mut backend = UinputBackend::new(logger);
        backend.init().unwrap();
        backend.scroll(ScrollDirection::Up, 3).unwrap();
    }

    #[test]
    #[cfg_attr(not(feature = "integration"), ignore)]
    fn test_uinput_scroll_down() {
        let logger = Arc::new(Logger::new(100, LogLevel::Info, false));
        let mut backend = UinputBackend::new(logger);
        backend.init().unwrap();
        backend.scroll(ScrollDirection::Down, 3).unwrap();
    }

    #[test]
    #[cfg_attr(not(feature = "integration"), ignore)]
    fn test_uinput_key_press() {
        let logger = Arc::new(Logger::new(100, LogLevel::Info, false));
        let mut backend = UinputBackend::new(logger);
        backend.init().unwrap();
        backend.key_press(57).unwrap(); // KEY_SPACE
        backend.key_release(57).unwrap();
    }

    #[test]
    fn test_init_fails_gracefully_no_device() {
        // On systems without /dev/uinput writable, init should fail gracefully
        let logger = Arc::new(Logger::new(100, LogLevel::Info, false));
        logger.set_quiet(true);
        let mut backend = UinputBackend::new(logger);
        // This may or may not fail depending on the system
        let _ = backend.init();
    }
}
