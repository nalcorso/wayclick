// SPDX-License-Identifier: MIT
use crate::MutexExt;
use std::collections::VecDeque;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::SystemTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl fmt::Display for LogLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LogLevel::Trace => write!(f, "TRACE"),
            LogLevel::Debug => write!(f, "DEBUG"),
            LogLevel::Info => write!(f, "INFO"),
            LogLevel::Warn => write!(f, "WARN"),
            LogLevel::Error => write!(f, "ERROR"),
        }
    }
}

impl LogLevel {
    pub fn from_str_level(s: &str) -> Option<LogLevel> {
        match s.to_lowercase().as_str() {
            "trace" => Some(LogLevel::Trace),
            "debug" => Some(LogLevel::Debug),
            "info" => Some(LogLevel::Info),
            "warn" => Some(LogLevel::Warn),
            "error" => Some(LogLevel::Error),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct LogEntry {
    pub timestamp: SystemTime,
    pub level: LogLevel,
    pub message: String,
}

impl LogEntry {
    pub fn format_iso8601(&self) -> String {
        let duration = self
            .timestamp
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default();
        let secs = duration.as_secs();
        let millis = duration.subsec_millis();
        // Simple ISO-8601-ish format without pulling in chrono
        format!("{}.{:03}", secs, millis)
    }

    pub fn format_human(&self) -> String {
        format!(
            "[{}] [{}] {}",
            self.format_iso8601(),
            self.level,
            self.message
        )
    }

    pub fn format_json(&self) -> String {
        let ts = self.format_iso8601();
        serde_json::json!({
            "timestamp": ts,
            "level": self.level.to_string(),
            "message": self.message,
        })
        .to_string()
    }
}

pub struct Logger {
    capacity: usize,
    entries: Mutex<VecDeque<LogEntry>>,
    min_level: LogLevel,
    json_mode: bool,
    /// If true, suppress stdout/stderr output (for tests)
    quiet: AtomicBool,
}

impl Logger {
    pub fn new(capacity: usize, min_level: LogLevel, json_mode: bool) -> Self {
        Self {
            capacity,
            entries: Mutex::new(VecDeque::with_capacity(capacity)),
            min_level,
            json_mode,
            quiet: AtomicBool::new(false),
        }
    }

    pub fn set_quiet(&self, quiet: bool) {
        self.quiet.store(quiet, Ordering::Relaxed);
    }

    fn log(&self, level: LogLevel, message: String) {
        if level < self.min_level {
            return;
        }

        let entry = LogEntry {
            timestamp: SystemTime::now(),
            level,
            message,
        };

        if !self.quiet.load(Ordering::Relaxed) {
            let formatted = if self.json_mode {
                entry.format_json()
            } else {
                entry.format_human()
            };

            match level {
                LogLevel::Warn | LogLevel::Error => {
                    eprintln!("{}", formatted);
                }
                _ => {
                    println!("{}", formatted);
                }
            }
        }

        let mut entries = self.entries.lock_or_recover();
        if entries.len() >= self.capacity {
            entries.pop_front();
        }
        entries.push_back(entry);
    }

    pub fn trace(&self, msg: impl Into<String>) {
        self.log(LogLevel::Trace, msg.into());
    }

    pub fn debug(&self, msg: impl Into<String>) {
        self.log(LogLevel::Debug, msg.into());
    }

    pub fn info(&self, msg: impl Into<String>) {
        self.log(LogLevel::Info, msg.into());
    }

    pub fn warn(&self, msg: impl Into<String>) {
        self.log(LogLevel::Warn, msg.into());
    }

    pub fn error(&self, msg: impl Into<String>) {
        self.log(LogLevel::Error, msg.into());
    }

    pub fn recent(&self, n: usize) -> Vec<LogEntry> {
        let entries = self.entries.lock_or_recover();
        let start = if entries.len() > n {
            entries.len() - n
        } else {
            0
        };
        entries.iter().skip(start).cloned().collect()
    }

    pub fn all_entries(&self) -> Vec<LogEntry> {
        self.entries.lock_or_recover().iter().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_logger_basic() {
        let logger = Logger::new(10, LogLevel::Info, false);
        logger.set_quiet(true);
        logger.info("hello");
        logger.warn("warning");
        logger.trace("should be filtered");

        let entries = logger.all_entries();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].message, "hello");
        assert_eq!(entries[0].level, LogLevel::Info);
        assert_eq!(entries[1].message, "warning");
    }

    #[test]
    fn test_logger_capacity() {
        let logger = Logger::new(3, LogLevel::Trace, false);
        logger.set_quiet(true);
        for i in 0..5 {
            logger.info(format!("msg{}", i));
        }
        let entries = logger.all_entries();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].message, "msg2");
        assert_eq!(entries[2].message, "msg4");
    }

    #[test]
    fn test_logger_recent() {
        let logger = Logger::new(10, LogLevel::Trace, false);
        logger.set_quiet(true);
        for i in 0..5 {
            logger.info(format!("msg{}", i));
        }
        let recent = logger.recent(2);
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].message, "msg3");
        assert_eq!(recent[1].message, "msg4");
    }

    #[test]
    fn test_log_level_ordering() {
        assert!(LogLevel::Trace < LogLevel::Debug);
        assert!(LogLevel::Debug < LogLevel::Info);
        assert!(LogLevel::Info < LogLevel::Warn);
        assert!(LogLevel::Warn < LogLevel::Error);
    }

    #[test]
    fn test_json_format() {
        let logger = Logger::new(10, LogLevel::Info, true);
        logger.set_quiet(true);
        logger.info("test message");
        let entries = logger.all_entries();
        let json = entries[0].format_json();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["level"], "INFO");
        assert_eq!(parsed["message"], "test message");
    }

    #[test]
    fn test_log_level_from_str() {
        assert_eq!(LogLevel::from_str_level("info"), Some(LogLevel::Info));
        assert_eq!(LogLevel::from_str_level("WARN"), Some(LogLevel::Warn));
        assert_eq!(LogLevel::from_str_level("invalid"), None);
    }
}
