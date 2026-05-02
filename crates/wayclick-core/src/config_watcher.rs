// SPDX-License-Identifier: MIT
// ConfigWatcher — watches Lua config files for changes and triggers reload.

use crate::logger::Logger;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, SystemTime};

pub struct ConfigWatcher {
    config_dir: PathBuf,
    logger: Arc<Logger>,
    shutdown: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl ConfigWatcher {
    pub fn new(config_dir: PathBuf, logger: Arc<Logger>) -> Self {
        Self {
            config_dir,
            logger,
            shutdown: Arc::new(AtomicBool::new(false)),
            handle: None,
        }
    }

    /// Start watching for changes. The callback is invoked when any .lua file changes.
    pub fn start<F>(&mut self, callback: F)
    where
        F: Fn() + Send + 'static,
    {
        let config_dir = self.config_dir.clone();
        let logger = self.logger.clone();
        let shutdown = self.shutdown.clone();

        // Take initial snapshot
        let mut timestamps = scan_lua_files(&config_dir);
        logger.info(format!(
            "ConfigWatcher: watching {} files in {:?}",
            timestamps.len(),
            config_dir
        ));

        self.handle = Some(thread::spawn(move || {
            while !shutdown.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(500));

                let new_timestamps = scan_lua_files(&config_dir);
                if new_timestamps != timestamps {
                    logger.info("ConfigWatcher: change detected, triggering reload");
                    callback();
                    timestamps = new_timestamps;
                }
            }
            logger.info("ConfigWatcher: stopped");
        }));
    }

    /// Signal a manual reload (e.g., from SIGHUP).
    pub fn shutdown_flag(&self) -> Arc<AtomicBool> {
        self.shutdown.clone()
    }

    pub fn stop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for ConfigWatcher {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Scan all *.lua files in the config directory tree and return their modification times.
fn scan_lua_files(dir: &Path) -> HashMap<PathBuf, SystemTime> {
    let mut result = HashMap::new();
    if let Ok(entries) = walk_dir(dir) {
        for path in entries {
            if path.extension().map(|e| e == "lua").unwrap_or(false) {
                if let Ok(meta) = fs::metadata(&path) {
                    if let Ok(modified) = meta.modified() {
                        result.insert(path, modified);
                    }
                }
            }
        }
    }
    result
}

fn walk_dir(dir: &Path) -> Result<Vec<PathBuf>, std::io::Error> {
    let mut files = Vec::new();
    if !dir.is_dir() {
        return Ok(files);
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            files.extend(walk_dir(&path)?);
        } else {
            files.push(path);
        }
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn test_scan_lua_files() {
        let dir = tempfile::tempdir().unwrap();
        File::create(dir.path().join("init.lua"))
            .unwrap()
            .write_all(b"-- init")
            .unwrap();
        let sub = dir.path().join("lua");
        fs::create_dir_all(&sub).unwrap();
        File::create(sub.join("helper.lua"))
            .unwrap()
            .write_all(b"-- helper")
            .unwrap();
        File::create(sub.join("readme.txt"))
            .unwrap()
            .write_all(b"not lua")
            .unwrap();

        let files = scan_lua_files(dir.path());
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn test_config_watcher_detects_change() {
        let dir = tempfile::tempdir().unwrap();
        let lua_file = dir.path().join("init.lua");
        File::create(&lua_file)
            .unwrap()
            .write_all(b"-- v1")
            .unwrap();

        let logger = Arc::new(Logger::new(100, crate::logger::LogLevel::Trace, false));
        logger.set_quiet(true);

        let reload_count = Arc::new(AtomicUsize::new(0));
        let reload_count_clone = reload_count.clone();

        let mut watcher = ConfigWatcher::new(dir.path().to_path_buf(), logger);
        watcher.start(move || {
            reload_count_clone.fetch_add(1, Ordering::Relaxed);
        });

        // Modify the file
        thread::sleep(Duration::from_millis(100));
        File::create(&lua_file)
            .unwrap()
            .write_all(b"-- v2")
            .unwrap();

        // Wait for detection
        thread::sleep(Duration::from_millis(800));

        watcher.stop();
        assert!(reload_count.load(Ordering::Relaxed) >= 1);
    }
}
