// SPDX-License-Identifier: MIT
#![no_main]
use libfuzzer_sys::fuzz_target;
use std::io::Cursor;
use wayclick_core::ipc::{decode_frame, encode_frame};

fuzz_target!(|data: &[u8]| {
    // Test decode with arbitrary bytes — must not panic
    let mut cursor = Cursor::new(data);
    let _ = decode_frame(&mut cursor);

    // If valid UTF-8 JSON, test round-trip
    if let Ok(s) = std::str::from_utf8(data) {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(s) {
            if let Ok(encoded) = encode_frame(&value) {
                let mut cursor2 = Cursor::new(&encoded[..]);
                let decoded = decode_frame(&mut cursor2);
                assert!(decoded.is_ok(), "Round-trip decode failed");
            }
        }
    }
});
