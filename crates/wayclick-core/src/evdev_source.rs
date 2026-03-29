use std::path::PathBuf;
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
