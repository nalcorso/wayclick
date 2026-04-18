use crate::config::{MouseButton, ScrollDirection};
use crate::logger::Logger;
use std::sync::{Arc, Mutex};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum BackendError {
    #[error("Backend not initialized")]
    NotInitialized,
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Backend error: {0}")]
    Other(String),
}

pub trait InputBackend: Send + Sync {
    fn init(&mut self) -> Result<(), BackendError>;
    fn click(&self, button: MouseButton) -> Result<(), BackendError>;
    fn mouse_press(&self, button: MouseButton) -> Result<(), BackendError>;
    fn mouse_release(&self, button: MouseButton) -> Result<(), BackendError>;
    fn key_press(&self, key_code: u32) -> Result<(), BackendError>;
    fn key_release(&self, key_code: u32) -> Result<(), BackendError>;
    fn scroll(&self, direction: ScrollDirection, amount: i32) -> Result<(), BackendError>;
    fn move_relative(&self, dx: i32, dy: i32) -> Result<(), BackendError>;
    fn name(&self) -> &str;
}

/// Dry-run backend: logs all calls, never fails.
pub struct LoggingBackend {
    logger: Arc<Logger>,
}

impl LoggingBackend {
    pub fn new(logger: Arc<Logger>) -> Self {
        Self { logger }
    }
}

impl InputBackend for LoggingBackend {
    fn init(&mut self) -> Result<(), BackendError> {
        self.logger.info("LoggingBackend initialized (dry-run mode)");
        Ok(())
    }

    fn click(&self, button: MouseButton) -> Result<(), BackendError> {
        self.logger
            .debug(format!("DRY RUN click {:?}", button));
        Ok(())
    }

    fn mouse_press(&self, button: MouseButton) -> Result<(), BackendError> {
        self.logger
            .debug(format!("DRY RUN mouse_press {:?}", button));
        Ok(())
    }

    fn mouse_release(&self, button: MouseButton) -> Result<(), BackendError> {
        self.logger
            .debug(format!("DRY RUN mouse_release {:?}", button));
        Ok(())
    }

    fn key_press(&self, key_code: u32) -> Result<(), BackendError> {
        self.logger
            .debug(format!("DRY RUN key_press code={}", key_code));
        Ok(())
    }

    fn key_release(&self, key_code: u32) -> Result<(), BackendError> {
        self.logger
            .debug(format!("DRY RUN key_release code={}", key_code));
        Ok(())
    }

    fn scroll(&self, direction: ScrollDirection, amount: i32) -> Result<(), BackendError> {
        self.logger
            .debug(format!("DRY RUN scroll {:?} amount={}", direction, amount));
        Ok(())
    }

    fn move_relative(&self, dx: i32, dy: i32) -> Result<(), BackendError> {
        self.logger
            .debug(format!("DRY RUN move_relative dx={} dy={}", dx, dy));
        Ok(())
    }

    fn name(&self) -> &str {
        "logging"
    }
}

/// Records all calls for assertion in tests.
#[derive(Debug, Clone, PartialEq)]
pub enum BackendCall {
    Click(MouseButton),
    MousePress(MouseButton),
    MouseRelease(MouseButton),
    KeyPress(u32),
    KeyRelease(u32),
    Scroll(ScrollDirection, i32),
    MoveRelative(i32, i32),
}

pub struct MockBackend {
    pub calls: Arc<Mutex<Vec<BackendCall>>>,
}

impl MockBackend {
    pub fn new() -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn calls_clone(&self) -> Arc<Mutex<Vec<BackendCall>>> {
        self.calls.clone()
    }

    pub fn get_calls(&self) -> Vec<BackendCall> {
        self.calls.lock().unwrap().clone()
    }
}

impl InputBackend for MockBackend {
    fn init(&mut self) -> Result<(), BackendError> {
        Ok(())
    }

    fn click(&self, button: MouseButton) -> Result<(), BackendError> {
        self.calls.lock().unwrap().push(BackendCall::Click(button));
        Ok(())
    }

    fn mouse_press(&self, button: MouseButton) -> Result<(), BackendError> {
        self.calls
            .lock()
            .unwrap()
            .push(BackendCall::MousePress(button));
        Ok(())
    }

    fn mouse_release(&self, button: MouseButton) -> Result<(), BackendError> {
        self.calls
            .lock()
            .unwrap()
            .push(BackendCall::MouseRelease(button));
        Ok(())
    }

    fn key_press(&self, key_code: u32) -> Result<(), BackendError> {
        self.calls
            .lock()
            .unwrap()
            .push(BackendCall::KeyPress(key_code));
        Ok(())
    }

    fn key_release(&self, key_code: u32) -> Result<(), BackendError> {
        self.calls
            .lock()
            .unwrap()
            .push(BackendCall::KeyRelease(key_code));
        Ok(())
    }

    fn scroll(&self, direction: ScrollDirection, amount: i32) -> Result<(), BackendError> {
        self.calls
            .lock()
            .unwrap()
            .push(BackendCall::Scroll(direction, amount));
        Ok(())
    }

    fn move_relative(&self, dx: i32, dy: i32) -> Result<(), BackendError> {
        self.calls
            .lock()
            .unwrap()
            .push(BackendCall::MoveRelative(dx, dy));
        Ok(())
    }

    fn name(&self) -> &str {
        "mock"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logger::LogLevel;

    #[test]
    fn test_logging_backend() {
        let logger = Arc::new(Logger::new(100, LogLevel::Trace, false));
        logger.set_quiet(true);
        let mut backend = LoggingBackend::new(logger.clone());
        backend.init().unwrap();
        backend.click(MouseButton::Left).unwrap();
        backend.key_press(57).unwrap();
        backend.key_release(57).unwrap();
        backend.scroll(ScrollDirection::Down, 3).unwrap();
        backend.move_relative(10, 20).unwrap();
        assert_eq!(backend.name(), "logging");

        let entries = logger.all_entries();
        assert!(entries.len() >= 5);
    }

    #[test]
    fn test_mock_backend() {
        let mut backend = MockBackend::new();
        backend.init().unwrap();
        backend.click(MouseButton::Left).unwrap();
        backend.key_press(57).unwrap();
        backend.scroll(ScrollDirection::Up, 1).unwrap();
        backend.move_relative(5, -5).unwrap();

        let calls = backend.get_calls();
        assert_eq!(calls.len(), 4);
        assert_eq!(calls[0], BackendCall::Click(MouseButton::Left));
        assert_eq!(calls[1], BackendCall::KeyPress(57));
        assert_eq!(calls[2], BackendCall::Scroll(ScrollDirection::Up, 1));
        assert_eq!(calls[3], BackendCall::MoveRelative(5, -5));
        assert_eq!(backend.name(), "mock");
    }
}
