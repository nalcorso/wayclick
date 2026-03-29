use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::time::Duration;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SourceError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Device disconnected")]
    Disconnected,
    #[error("Device not found: {0}")]
    NotFound(String),
}

#[derive(Debug, Clone)]
pub struct DeviceInfo {
    pub path: PathBuf,
    pub name: String,
    pub vendor_id: u16,
    pub product_id: u16,
    pub phys: String,
}

#[derive(Debug, Clone)]
pub struct InputEvent {
    pub event_type: u16,
    pub code: u16,
    pub value: i32,
}

// Linux event types
pub const EV_KEY: u16 = 0x01;

pub trait InputSource: Send {
    fn device_info(&self) -> DeviceInfo;
    fn poll_events(&mut self, timeout: Duration) -> Result<Vec<InputEvent>, SourceError>;
    fn close(self);
}

/// Mock input source for testing.
pub struct MockSource {
    info: DeviceInfo,
    events: Vec<InputEvent>,
}

impl MockSource {
    pub fn new(info: DeviceInfo, events: Vec<InputEvent>) -> Self {
        Self { info, events }
    }
}

impl InputSource for MockSource {
    fn device_info(&self) -> DeviceInfo {
        self.info.clone()
    }

    fn poll_events(&mut self, _timeout: Duration) -> Result<Vec<InputEvent>, SourceError> {
        let events = std::mem::take(&mut self.events);
        Ok(events)
    }

    fn close(self) {}
}

/// Size of a raw input_event struct on this platform
const INPUT_EVENT_SIZE: usize = 24; // 8 + 8 + 2 + 2 + 4

/// Real evdev input source that reads from /dev/input/event*.
pub struct EvdevSource {
    info: DeviceInfo,
    file: File,
    grabbed: bool,
}

// ioctl for exclusive grab
nix::ioctl_write_int!(eviocgrab, b'E', 0x90);

// ioctls for device info
nix::ioctl_read_buf!(eviocgname, b'E', 0x06, u8);
nix::ioctl_read!(eviocgid, b'E', 0x02, [u16; 4]);
nix::ioctl_read_buf!(eviocgphys, b'E', 0x07, u8);

impl EvdevSource {
    /// Open a device by path.
    pub fn open(path: &Path, exclusive: bool) -> Result<Self, SourceError> {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(path)?;

        let info = read_device_info(path, &file)?;

        let mut grabbed = false;
        if exclusive {
            let fd = file.as_raw_fd();
            match unsafe { eviocgrab(fd, 1) } {
                Ok(_) => grabbed = true,
                Err(e) => {
                    eprintln!(
                        "Warning: failed to grab device {:?}: {}. Continuing without exclusive mode.",
                        path, e
                    );
                }
            }
        }

        Ok(Self {
            info,
            file,
            grabbed,
        })
    }
}

impl InputSource for EvdevSource {
    fn device_info(&self) -> DeviceInfo {
        self.info.clone()
    }

    fn poll_events(&mut self, timeout: Duration) -> Result<Vec<InputEvent>, SourceError> {
        let fd = self.file.as_raw_fd();

        // Use poll(2) to wait for events
        let mut pollfd = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let timeout_ms = timeout.as_millis() as i32;

        let ret = unsafe { libc::poll(&mut pollfd, 1, timeout_ms) };

        if ret < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::Interrupted {
                return Ok(vec![]);
            }
            return Err(SourceError::Io(err));
        }

        if ret == 0 {
            return Ok(vec![]); // Timeout
        }

        // Check for errors/hangup
        if pollfd.revents & (libc::POLLHUP | libc::POLLERR) != 0 {
            return Err(SourceError::Disconnected);
        }

        // Read available events
        let mut events = Vec::new();
        let mut buf = [0u8; INPUT_EVENT_SIZE * 64]; // Read up to 64 events at once

        match self.file.read(&mut buf) {
            Ok(n) => {
                let count = n / INPUT_EVENT_SIZE;
                for i in 0..count {
                    let offset = i * INPUT_EVENT_SIZE;
                    let event_bytes = &buf[offset..offset + INPUT_EVENT_SIZE];
                    // Parse: skip time (16 bytes), then type(2), code(2), value(4)
                    let type_ = u16::from_ne_bytes([event_bytes[16], event_bytes[17]]);
                    let code = u16::from_ne_bytes([event_bytes[18], event_bytes[19]]);
                    let value = i32::from_ne_bytes([
                        event_bytes[20],
                        event_bytes[21],
                        event_bytes[22],
                        event_bytes[23],
                    ]);
                    events.push(InputEvent {
                        event_type: type_,
                        code,
                        value,
                    });
                }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                // No data available
            }
            Err(ref e) if e.raw_os_error() == Some(libc::ENODEV) => {
                return Err(SourceError::Disconnected);
            }
            Err(e) => return Err(SourceError::Io(e)),
        }

        Ok(events)
    }

    fn close(self) {
        if self.grabbed {
            let fd = self.file.as_raw_fd();
            unsafe {
                let _ = eviocgrab(fd, 0);
            }
        }
        // File is dropped automatically
    }
}

/// Read device info from an open evdev file descriptor.
fn read_device_info(path: &Path, file: &File) -> Result<DeviceInfo, SourceError> {
    let fd = file.as_raw_fd();

    // Read device name
    let mut name_buf = [0u8; 256];
    let name = match unsafe { eviocgname(fd, &mut name_buf) } {
        Ok(_) => {
            let nul = name_buf.iter().position(|&b| b == 0).unwrap_or(name_buf.len());
            String::from_utf8_lossy(&name_buf[..nul]).to_string()
        }
        Err(_) => "Unknown".to_string(),
    };

    // Read device ID
    let mut id = [0u16; 4];
    let (vendor_id, product_id) = match unsafe { eviocgid(fd, &mut id) } {
        Ok(_) => (id[1], id[2]), // [bustype, vendor, product, version]
        Err(_) => (0, 0),
    };

    // Read physical location
    let mut phys_buf = [0u8; 256];
    let phys = match unsafe { eviocgphys(fd, &mut phys_buf) } {
        Ok(_) => {
            let nul = phys_buf.iter().position(|&b| b == 0).unwrap_or(phys_buf.len());
            String::from_utf8_lossy(&phys_buf[..nul]).to_string()
        }
        Err(_) => String::new(),
    };

    Ok(DeviceInfo {
        path: path.to_path_buf(),
        name,
        vendor_id,
        product_id,
        phys,
    })
}

/// Enumerate all /dev/input/event* devices and return their info.
pub fn enumerate_devices() -> Vec<DeviceInfo> {
    let mut devices = Vec::new();
    let input_dir = Path::new("/dev/input");

    if let Ok(entries) = fs::read_dir(input_dir) {
        let mut paths: Vec<PathBuf> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .map(|n| n.to_string_lossy().starts_with("event"))
                    .unwrap_or(false)
            })
            .collect();
        paths.sort();

        for path in paths {
            match OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(&path)
            {
                Ok(file) => {
                    if let Ok(info) = read_device_info(&path, &file) {
                        devices.push(info);
                    }
                }
                Err(_) => {
                    // Can't open — permission denied or device gone
                }
            }
        }
    }
    devices
}

/// libc bindings we need
mod libc {
    pub use ::libc::*;
}

