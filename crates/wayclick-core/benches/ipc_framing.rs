//! Benchmarks for IPC frame encoding/decoding.
//!
//! Measures serialization overhead for various payload sizes and
//! full roundtrip over a Unix socket pair.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use serde_json::{json, Value};
use std::io::Cursor;
use std::os::unix::net::UnixStream;

use wayclick_core::ipc::{decode_frame, encode_frame};

fn small_payload() -> Value {
    json!({
        "jsonrpc": "2.0",
        "result": "pong",
        "id": 1
    })
}

fn medium_payload() -> Value {
    let triggers: Vec<Value> = (0..20)
        .map(|i| {
            json!({
                "id": format!("trigger_{}", i),
                "name": format!("Trigger {}", i),
                "mode": "toggle",
                "active": i % 2 == 0
            })
        })
        .collect();

    json!({
        "jsonrpc": "2.0",
        "result": {
            "enabled": true,
            "layer": "default",
            "triggers": triggers,
            "uptime_seconds": 12345
        },
        "id": 42
    })
}

fn large_payload() -> Value {
    let triggers: Vec<Value> = (0..50)
        .map(|i| {
            json!({
                "id": format!("trigger_{}", i),
                "name": format!("Complex Trigger Configuration {}", i),
                "mode": if i % 3 == 0 { "toggle" } else { "hold" },
                "active": i % 2 == 0,
                "description": format!("This is trigger {} with a longer description for benchmarking purposes", i),
                "cooldown_ms": 100 + i * 10,
                "action": {
                    "type": "sequence",
                    "actions": [
                        {"type": "auto_click", "button": "left"},
                        {"type": "delay", "duration_ms": 50},
                        {"type": "key_press", "key": "KEY_A"}
                    ]
                }
            })
        })
        .collect();

    json!({
        "jsonrpc": "2.0",
        "result": {
            "enabled": true,
            "layer": "gaming",
            "triggers": triggers,
            "uptime_seconds": 99999,
            "version": "0.3.0"
        },
        "id": 100
    })
}

// --- Encode benchmarks ---

fn bench_encode(c: &mut Criterion) {
    let mut group = c.benchmark_group("ipc_encode");

    let payloads = [
        ("small", small_payload()),
        ("medium", medium_payload()),
        ("large", large_payload()),
    ];

    for (name, payload) in &payloads {
        group.bench_with_input(BenchmarkId::from_parameter(name), payload, |b, payload| {
            b.iter(|| encode_frame(payload).unwrap());
        });
    }

    group.finish();
}

// --- Decode benchmarks ---

fn bench_decode(c: &mut Criterion) {
    let mut group = c.benchmark_group("ipc_decode");

    let payloads = [
        ("small", small_payload()),
        ("medium", medium_payload()),
        ("large", large_payload()),
    ];

    for (name, payload) in &payloads {
        let encoded = encode_frame(payload).unwrap();
        group.bench_with_input(BenchmarkId::from_parameter(name), &encoded, |b, encoded| {
            b.iter(|| {
                let mut cursor = Cursor::new(encoded.as_slice());
                decode_frame(&mut cursor).unwrap()
            });
        });
    }

    group.finish();
}

// --- Roundtrip benchmarks ---

fn bench_roundtrip(c: &mut Criterion) {
    let mut group = c.benchmark_group("ipc_roundtrip");

    let payloads = [
        ("small", small_payload()),
        ("medium", medium_payload()),
        ("large", large_payload()),
    ];

    for (name, payload) in &payloads {
        group.bench_with_input(BenchmarkId::from_parameter(name), payload, |b, payload| {
            let (mut writer, mut reader) = UnixStream::pair().unwrap();
            // Set non-blocking off (default) for consistent timing
            writer.set_nonblocking(false).unwrap();
            reader.set_nonblocking(false).unwrap();

            b.iter(|| {
                let frame = encode_frame(payload).unwrap();
                std::io::Write::write_all(&mut writer, &frame).unwrap();
                decode_frame(&mut reader).unwrap()
            });
        });
    }

    group.finish();
}

// --- Payload size scaling ---

fn bench_encode_scaling(c: &mut Criterion) {
    let mut group = c.benchmark_group("ipc_encode_scaling");

    for trigger_count in [1, 10, 25, 50] {
        let triggers: Vec<Value> = (0..trigger_count)
            .map(|i| {
                json!({
                    "id": format!("t{}", i),
                    "name": format!("Trigger {}", i),
                    "active": true
                })
            })
            .collect();
        let payload = json!({
            "jsonrpc": "2.0",
            "result": {"triggers": triggers},
            "id": 1
        });

        group.bench_with_input(
            BenchmarkId::new("triggers", trigger_count),
            &payload,
            |b, payload| {
                b.iter(|| encode_frame(payload).unwrap());
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_encode,
    bench_decode,
    bench_roundtrip,
    bench_encode_scaling,
);
criterion_main!(benches);
