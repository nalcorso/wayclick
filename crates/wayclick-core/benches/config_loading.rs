//! Benchmarks for Lua config parsing and loading.
//!
//! Measures the cost of parsing Lua configurations of varying complexity.
//! Uses tempfile to create isolated config directories.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use std::fs;
use std::sync::Arc;
use tempfile::TempDir;

use wayclick_core::logger::{LogLevel, Logger};
use wayclick_core::lua_api::load_config;

fn write_config(dir: &TempDir, lua_code: &str) {
    fs::write(dir.path().join("init.lua"), lua_code).unwrap();
}

fn quiet_logger() -> Arc<Logger> {
    let logger = Logger::new(10, LogLevel::Error, false);
    logger.set_quiet(true);
    Arc::new(logger)
}

fn bench_load_minimal(c: &mut Criterion) {
    let dir = TempDir::new().unwrap();
    write_config(
        &dir,
        r#"
wayclick.register_trigger({
    id = "bench",
    action = wayclick.auto_click({ button = "left", interval_ms = 50 }),
})
"#,
    );

    c.bench_function("load_config/minimal_1_trigger", |b| {
        b.iter(|| load_config(&dir.path().join("init.lua"), &quiet_logger()).unwrap())
    });
}

fn bench_load_medium(c: &mut Criterion) {
    let dir = TempDir::new().unwrap();
    let mut lua = String::from("-- 20 triggers with varied actions\n");
    for i in 0..20 {
        let action = match i % 3 {
            0 => "wayclick.auto_click({ button = \"left\", interval_ms = 50 })".to_string(),
            1 => "wayclick.key_press({ key = \"space\", interval_ms = 100 })".to_string(),
            _ => "wayclick.scroll({ direction = \"down\", amount = 3 })".to_string(),
        };
        let mode = if i % 2 == 0 { "oneshot" } else { "toggle" };
        lua.push_str(&format!(
            r#"
wayclick.register_trigger({{
    id = "trigger_{i}",
    trigger = "KEY_A",
    mode = "{mode}",
    action = {action},
}})
"#,
            i = i,
            mode = mode,
            action = action,
        ));
    }
    write_config(&dir, &lua);

    c.bench_function("load_config/medium_20_triggers", |b| {
        b.iter(|| load_config(&dir.path().join("init.lua"), &quiet_logger()).unwrap())
    });
}

fn bench_load_complex(c: &mut Criterion) {
    let dir = TempDir::new().unwrap();
    let mut lua = String::from("-- 100 triggers with sequences and nesting\n");
    for i in 0..100 {
        if i % 5 == 0 {
            lua.push_str(&format!(
                r#"
wayclick.register_trigger({{
    id = "trigger_{i}",
    trigger = "KEY_A",
    mode = "oneshot",
    action = wayclick.sequence({{ actions = {{
        wayclick.auto_click({{ button = "left" }}),
        wayclick.delay({{ ms = 10 }}),
        wayclick.key_press({{ key = "space" }}),
    }} }}),
}})
"#,
                i = i,
            ));
        } else {
            lua.push_str(&format!(
                r#"
wayclick.register_trigger({{
    id = "trigger_{i}",
    trigger = "KEY_A",
    mode = "toggle",
    action = wayclick.auto_click({{ button = "left", interval_ms = {interval} }}),
}})
"#,
                i = i,
                interval = 20 + (i % 100),
            ));
        }
    }
    write_config(&dir, &lua);

    c.bench_function("load_config/complex_100_triggers", |b| {
        b.iter(|| load_config(&dir.path().join("init.lua"), &quiet_logger()).unwrap())
    });
}

fn bench_load_scaling(c: &mut Criterion) {
    let mut group = c.benchmark_group("config_scaling");

    for trigger_count in [1, 10, 50, 100] {
        let dir = TempDir::new().unwrap();
        let mut lua = String::new();
        for i in 0..trigger_count {
            lua.push_str(&format!(
                r#"
wayclick.register_trigger({{
    id = "t{i}",
    trigger = "KEY_A",
    action = wayclick.auto_click({{ button = "left" }}),
}})
"#,
                i = i,
            ));
        }
        write_config(&dir, &lua);

        group.bench_with_input(
            BenchmarkId::new("trigger_count", trigger_count),
            &trigger_count,
            |b, _| {
                b.iter(|| load_config(&dir.path().join("init.lua"), &quiet_logger()).unwrap());
            },
        );
    }

    group.finish();
}

fn bench_deep_nesting(c: &mut Criterion) {
    let mut group = c.benchmark_group("config_nesting");

    for depth in [1, 5, 10, 20] {
        let dir = TempDir::new().unwrap();
        // Build nested sequence: wayclick.sequence({ actions = { wayclick.sequence({ ... }) } })
        let mut inner = "wayclick.auto_click({ button = \"left\" })".to_string();
        for _ in 0..depth {
            inner = format!("wayclick.sequence({{ actions = {{ {} }} }})", inner);
        }
        let lua = format!(
            r#"
wayclick.register_trigger({{
    id = "nested",
    trigger = "BTN_SIDE",
    mode = "oneshot",
    action = {},
}})
"#,
            inner
        );
        write_config(&dir, &lua);

        group.bench_with_input(BenchmarkId::new("depth", depth), &depth, |b, _| {
            b.iter(|| load_config(&dir.path().join("init.lua"), &quiet_logger()).unwrap());
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_load_minimal,
    bench_load_medium,
    bench_load_complex,
    bench_load_scaling,
    bench_deep_nesting,
);
criterion_main!(benches);
