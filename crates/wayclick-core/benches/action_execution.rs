//! Benchmarks for the engine action execution hot path.
//!
//! These benchmarks measure pure engine overhead using NullBackend (zero I/O cost).
//! Only deterministic code paths are benchmarked — sleep-heavy paths (interruptible_sleep,
//! hold_ms > 0) are excluded from Criterion and tested separately.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use std::sync::Arc;

use wayclick_core::config::*;
use wayclick_core::engine::bench;
use wayclick_core::engine::Engine;
use wayclick_core::event_bus::EventBus;
use wayclick_core::input_backend::NullBackend;
use wayclick_core::logger::{LogLevel, Logger};

fn null_backend() -> Arc<dyn wayclick_core::input_backend::InputBackend> {
    Arc::new(NullBackend)
}

fn quiet_logger() -> Arc<Logger> {
    let logger = Logger::new(10, LogLevel::Error, false);
    logger.set_quiet(true);
    Arc::new(logger)
}

fn bench_engine(triggers: Vec<TriggerBinding>) -> Engine {
    let config = Config {
        triggers,
        ..Config::default()
    };
    Engine::new(
        config,
        null_backend(),
        quiet_logger(),
        Arc::new(EventBus::new()),
        "/dev/null".to_string(),
    )
}

// --- do_click benchmarks ---

fn bench_do_click(c: &mut Criterion) {
    let backend = null_backend();

    c.bench_function("do_click/instant", |b| {
        b.iter(|| bench::bench_do_click(&backend, black_box(MouseButton::Left), 0))
    });
}

// --- execute_action_sync benchmarks ---

fn bench_execute_action_sync(c: &mut Criterion) {
    let backend = null_backend();
    let logger = quiet_logger();

    let mut group = c.benchmark_group("execute_action_sync");

    // Single click (duration_ms=0 → one click, no loop)
    group.bench_function("click", |b| {
        let action = ActionConfig::AutoClick {
            button: MouseButton::Left,
            interval_ms: 50,
            duration_ms: Some(0),
            jitter_ms: 0,
            hold_ms: 0,
        };
        b.iter(|| bench::bench_execute_action_sync(black_box(&action), &backend, &logger))
    });

    // Single keypress
    group.bench_function("key_press", |b| {
        let action = ActionConfig::KeyPress {
            key_name: "a".into(),
            key_code: 30,
            interval_ms: 50,
            duration_ms: Some(0),
            jitter_ms: 0,
        };
        b.iter(|| bench::bench_execute_action_sync(black_box(&action), &backend, &logger))
    });

    // Mouse move relative
    group.bench_function("mouse_move", |b| {
        let action = ActionConfig::MouseMove {
            dx: 10,
            dy: 5,
            interval_ms: 50,
            duration_ms: Some(0),
            jitter_ms: 0,
        };
        b.iter(|| bench::bench_execute_action_sync(black_box(&action), &backend, &logger))
    });

    // Scroll wheel
    group.bench_function("scroll", |b| {
        let action = ActionConfig::ScrollWheel {
            direction: ScrollDirection::Down,
            amount: 3,
            interval_ms: 50,
            duration_ms: Some(0),
            jitter_ms: 0,
        };
        b.iter(|| bench::bench_execute_action_sync(black_box(&action), &backend, &logger))
    });

    // Move absolute
    group.bench_function("move_absolute", |b| {
        let action = ActionConfig::MouseMoveAbsolute { x: 500, y: 300 };
        b.iter(|| bench::bench_execute_action_sync(black_box(&action), &backend, &logger))
    });

    // ClickAt with settle_ms=0 (no sleep — pure dispatch overhead)
    group.bench_function("click_at_no_settle", |b| {
        let action = ActionConfig::ClickAt {
            x: 100,
            y: 200,
            button: MouseButton::Left,
            hold_ms: 0,
            settle_ms: 0,
        };
        b.iter(|| bench::bench_execute_action_sync(black_box(&action), &backend, &logger))
    });

    // ClickAt with default settle_ms=5 (includes 5ms sleep)
    group.bench_function("click_at_settle_5ms", |b| {
        let action = ActionConfig::ClickAt {
            x: 100,
            y: 200,
            button: MouseButton::Left,
            hold_ms: 0,
            settle_ms: 5,
        };
        b.iter(|| bench::bench_execute_action_sync(black_box(&action), &backend, &logger))
    });

    // NoOp
    group.bench_function("noop", |b| {
        let action = ActionConfig::NoOp;
        b.iter(|| bench::bench_execute_action_sync(black_box(&action), &backend, &logger))
    });

    // SetLayer
    group.bench_function("set_layer", |b| {
        let action = ActionConfig::SetLayer {
            layer: "gaming".into(),
        };
        b.iter(|| bench::bench_execute_action_sync(black_box(&action), &backend, &logger))
    });

    group.finish();
}

// --- Sequence benchmarks ---

fn bench_sequences(c: &mut Criterion) {
    let backend = null_backend();
    let logger = quiet_logger();

    let mut group = c.benchmark_group("composite");

    // 3-action sequence (typical macro)
    group.bench_function("sequence_3", |b| {
        let action = ActionConfig::Composite {
            mode: CompositeMode::Sequence,
            actions: vec![
                ActionConfig::AutoClick {
                    button: MouseButton::Left,
                    interval_ms: 50,
                    duration_ms: Some(0),
                    jitter_ms: 0,
                    hold_ms: 0,
                },
                ActionConfig::KeyPress {
                    key_name: "a".into(),
                    key_code: 30,
                    interval_ms: 50,
                    duration_ms: Some(0),
                    jitter_ms: 0,
                },
                ActionConfig::AutoClick {
                    button: MouseButton::Right,
                    interval_ms: 50,
                    duration_ms: Some(0),
                    jitter_ms: 0,
                    hold_ms: 0,
                },
            ],
        };
        b.iter(|| bench::bench_execute_action_sync(black_box(&action), &backend, &logger))
    });

    // 10-action sequence
    group.bench_function("sequence_10", |b| {
        let actions: Vec<ActionConfig> = (0..10)
            .map(|_| ActionConfig::AutoClick {
                button: MouseButton::Left,
                interval_ms: 50,
                duration_ms: Some(0),
                jitter_ms: 0,
                hold_ms: 0,
            })
            .collect();
        let action = ActionConfig::Composite {
            mode: CompositeMode::Sequence,
            actions,
        };
        b.iter(|| bench::bench_execute_action_sync(black_box(&action), &backend, &logger))
    });

    // Nested sequence (depth 5, each with 1 action)
    group.bench_function("nested_sequence_5", |b| {
        let mut action = ActionConfig::AutoClick {
            button: MouseButton::Left,
            interval_ms: 50,
            duration_ms: Some(0),
            jitter_ms: 0,
            hold_ms: 0,
        };
        for _ in 0..5 {
            action = ActionConfig::Composite {
                mode: CompositeMode::Sequence,
                actions: vec![action],
            };
        }
        b.iter(|| bench::bench_execute_action_sync(black_box(&action), &backend, &logger))
    });

    // Nested sequence (depth 20 — stress test)
    group.bench_function("nested_sequence_20", |b| {
        let mut action = ActionConfig::AutoClick {
            button: MouseButton::Left,
            interval_ms: 50,
            duration_ms: Some(0),
            jitter_ms: 0,
            hold_ms: 0,
        };
        for _ in 0..20 {
            action = ActionConfig::Composite {
                mode: CompositeMode::Sequence,
                actions: vec![action],
            };
        }
        b.iter(|| bench::bench_execute_action_sync(black_box(&action), &backend, &logger))
    });

    // Parallel with 4 actions (spawns 4 threads)
    group.bench_function("parallel_4", |b| {
        let actions: Vec<ActionConfig> = (0..4)
            .map(|_| ActionConfig::AutoClick {
                button: MouseButton::Left,
                interval_ms: 50,
                duration_ms: Some(0),
                jitter_ms: 0,
                hold_ms: 0,
            })
            .collect();
        let action = ActionConfig::Composite {
            mode: CompositeMode::Parallel,
            actions,
        };
        b.iter(|| bench::bench_execute_action_sync(black_box(&action), &backend, &logger))
    });

    group.finish();
}

// --- jittered_interval benchmarks ---

fn bench_jittered_interval(c: &mut Criterion) {
    let mut group = c.benchmark_group("jittered_interval");

    group.bench_function("no_jitter", |b| {
        b.iter(|| bench::bench_jittered_interval(black_box(50), black_box(0)))
    });

    group.bench_function("with_jitter_5ms", |b| {
        b.iter(|| bench::bench_jittered_interval(black_box(50), black_box(5)))
    });

    group.bench_function("with_jitter_20ms", |b| {
        b.iter(|| bench::bench_jittered_interval(black_box(50), black_box(20)))
    });

    group.finish();
}

// --- Engine toggle start/stop lifecycle ---

fn bench_toggle_lifecycle(c: &mut Criterion) {
    c.bench_function("toggle_start_stop", |b| {
        let trigger = TriggerBinding {
            id: "bench".into(),
            name: "Bench".into(),
            description: String::new(),
            mode: TriggerMode::Toggle,
            action: ActionConfig::NoOp,
            cooldown_ms: None,
        };
        let mut engine = bench_engine(vec![trigger]);
        engine.set_enabled(true);

        b.iter(|| {
            // press=true starts the toggle, press=true again stops it
            engine.trigger_event("bench", true).unwrap();
            std::thread::yield_now();
            engine.trigger_event("bench", true).unwrap();
        });
    });
}

// --- Scaling benchmarks (parameterized) ---

fn bench_sequence_scaling(c: &mut Criterion) {
    let backend = null_backend();
    let logger = quiet_logger();
    let mut group = c.benchmark_group("sequence_scaling");

    for size in [1, 5, 10, 25, 50] {
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, &size| {
            let actions: Vec<ActionConfig> = (0..size)
                .map(|_| ActionConfig::AutoClick {
                    button: MouseButton::Left,
                    interval_ms: 50,
                    duration_ms: Some(0),
                    jitter_ms: 0,
                    hold_ms: 0,
                })
                .collect();
            let action = ActionConfig::Composite {
                mode: CompositeMode::Sequence,
                actions,
            };
            b.iter(|| bench::bench_execute_action_sync(black_box(&action), &backend, &logger));
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_do_click,
    bench_execute_action_sync,
    bench_sequences,
    bench_jittered_interval,
    bench_toggle_lifecycle,
    bench_sequence_scaling,
);
criterion_main!(benches);
