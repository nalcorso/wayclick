// SPDX-License-Identifier: MIT
#![no_main]
use libfuzzer_sys::fuzz_target;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use wayclick_core::logger::{LogLevel, Logger};
use wayclick_core::lua_api::load_config;

fuzz_target!(|data: &[u8]| {
    // Write fuzz data to a temp file and try to load it as a Lua config
    if let Ok(content) = std::str::from_utf8(data) {
        let dir = std::env::temp_dir().join("wayclick_fuzz");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("fuzz_init.lua");
        if let Ok(mut f) = std::fs::File::create(&path) {
            let _ = f.write_all(content.as_bytes());
            drop(f);

            let logger = Arc::new(Logger::new(10, LogLevel::Error, false));
            logger.set_quiet(true);
            let _ = load_config(&path, &logger);
        }
        let _ = std::fs::remove_file(&path);
    }
});
