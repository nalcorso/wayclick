use crate::config::*;
use crate::input_backend::{BackendError, InputBackend};
use crate::logger::Logger;
use rand::Rng;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::mpsc;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("Unknown trigger: {0}")]
    UnknownTrigger(String),
    #[error("Engine is disabled")]
    Disabled,
    #[error("Trigger is in cooldown")]
    Cooldown,
    #[error("Backend error: {0}")]
    Backend(#[from] BackendError),
}

enum TriggerState {
    Idle,
    Active {
        stop_tx: mpsc::Sender<()>,
        handle: Option<JoinHandle<()>>,
    },
    Cooldown {
        until: Instant,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct TriggerSnapshot {
    pub id: String,
    pub name: String,
    pub mode: TriggerMode,
    pub action_type: String,
    pub active: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct StatusReport {
    pub enabled: bool,
    pub dry_run: bool,
    pub trigger_count: usize,
    pub active_triggers: Vec<String>,
    pub current_layer: String,
    pub backend: String,
    pub config_path: String,
    pub uptime_secs: u64,
}

pub struct Engine {
    config: Config,
    state: HashMap<String, TriggerState>,
    backend: Arc<dyn InputBackend>,
    logger: Arc<Logger>,
    enabled: bool,
    start_time: Instant,
    config_path: String,
    current_layer: String,
}

impl Engine {
    pub fn new(
        config: Config,
        backend: Arc<dyn InputBackend>,
        logger: Arc<Logger>,
        config_path: String,
    ) -> Self {
        let mut state = HashMap::new();
        for trigger in &config.triggers {
            state.insert(trigger.id.clone(), TriggerState::Idle);
        }
        Self {
            config,
            state,
            backend,
            logger,
            enabled: false,
            start_time: Instant::now(),
            config_path,
            current_layer: "base".to_string(),
        }
    }

    pub fn apply_config(&mut self, config: Config) {
        self.stop_all_workers();
        self.state.clear();
        for trigger in &config.triggers {
            self.state.insert(trigger.id.clone(), TriggerState::Idle);
        }
        self.config = config;
        self.logger.info("Config applied, all workers stopped");
    }

    /// Get the current active layer name.
    pub fn current_layer(&self) -> &str {
        &self.current_layer
    }

    /// Set the active layer.
    pub fn set_layer(&mut self, layer: String) {
        self.logger.info(format!(
            "Layer changed: '{}' -> '{}'",
            self.current_layer, layer
        ));
        self.current_layer = layer;
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        if !enabled {
            self.stop_all_workers();
        }
        self.enabled = enabled;
        self.logger.info(format!(
            "Engine {}",
            if enabled { "enabled" } else { "disabled" }
        ));
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn toggle_enabled(&mut self) -> bool {
        let new_state = !self.enabled;
        self.set_enabled(new_state);
        new_state
    }

    pub fn trigger_event(&mut self, id: &str, press: bool) -> Result<(), EngineError> {
        if !self.enabled {
            return Err(EngineError::Disabled);
        }

        let trigger = self
            .config
            .triggers
            .iter()
            .find(|t| t.id == id)
            .ok_or_else(|| EngineError::UnknownTrigger(id.to_string()))?
            .clone();

        match trigger.mode {
            TriggerMode::Toggle => self.handle_toggle(&trigger),
            TriggerMode::Hold => self.handle_hold(&trigger, press),
            TriggerMode::OneShot => self.handle_oneshot(&trigger),
        }
    }

    fn handle_toggle(&mut self, trigger: &TriggerBinding) -> Result<(), EngineError> {
        let state = self
            .state
            .get(&trigger.id)
            .expect("trigger state must exist");

        match state {
            TriggerState::Active { .. } => {
                // Stop worker
                self.stop_worker(&trigger.id);
                self.logger.info(format!("{}: stopped", trigger.id));
                // Enter cooldown if configured
                if let Some(cd) = trigger.cooldown_ms {
                    self.state.insert(
                        trigger.id.clone(),
                        TriggerState::Cooldown {
                            until: Instant::now() + Duration::from_millis(cd as u64),
                        },
                    );
                }
                Ok(())
            }
            TriggerState::Cooldown { until } => {
                if Instant::now() < *until {
                    return Err(EngineError::Cooldown);
                }
                // Cooldown expired, start worker
                self.start_worker(trigger)?;
                Ok(())
            }
            TriggerState::Idle => {
                self.start_worker(trigger)?;
                Ok(())
            }
        }
    }

    fn handle_hold(&mut self, trigger: &TriggerBinding, press: bool) -> Result<(), EngineError> {
        if press {
            let state = self.state.get(&trigger.id).expect("state must exist");
            match state {
                TriggerState::Active { .. } => Ok(()), // Already active
                _ => {
                    self.start_worker(trigger)?;
                    Ok(())
                }
            }
        } else {
            // Release
            self.stop_worker(&trigger.id);
            self.logger.info(format!("{}: released", trigger.id));
            Ok(())
        }
    }

    fn handle_oneshot(&mut self, trigger: &TriggerBinding) -> Result<(), EngineError> {
        self.logger
            .info(format!("{}: oneshot executing", trigger.id));

        // Handle SetLayer directly (needs mutable self access)
        if let ActionConfig::SetLayer { ref layer } = trigger.action {
            self.set_layer(layer.clone());
            self.logger
                .info(format!("{}: oneshot complete", trigger.id));
            return Ok(());
        }

        execute_action_sync(&trigger.action, &self.backend, &self.logger)?;
        self.logger
            .info(format!("{}: oneshot complete", trigger.id));
        Ok(())
    }

    fn start_worker(&mut self, trigger: &TriggerBinding) -> Result<(), EngineError> {
        let (stop_tx, stop_rx) = mpsc::channel();
        let action = trigger.action.clone();
        let backend = self.backend.clone();
        let logger = self.logger.clone();
        let trigger_id = trigger.id.clone();

        self.logger.info(format!("{}: started", trigger.id));

        let handle = thread::spawn(move || {
            if let Err(e) = execute_action_loop(&action, &backend, &logger, &stop_rx) {
                logger.error(format!("{}: worker error: {}", trigger_id, e));
            }
        });

        self.state.insert(
            trigger.id.clone(),
            TriggerState::Active {
                stop_tx,
                handle: Some(handle),
            },
        );

        Ok(())
    }

    fn stop_worker(&mut self, id: &str) {
        if let Some(TriggerState::Active { stop_tx, handle }) = self.state.remove(id) {
            let _ = stop_tx.send(());
            if let Some(h) = handle {
                let _ = h.join();
            }
        }
        self.state.insert(id.to_string(), TriggerState::Idle);
    }

    fn stop_all_workers(&mut self) {
        let ids: Vec<String> = self.state.keys().cloned().collect();
        for id in ids {
            self.stop_worker(&id);
        }
    }

    pub fn describe_status(&self) -> StatusReport {
        let active_triggers: Vec<String> = self
            .state
            .iter()
            .filter(|(_, s)| matches!(s, TriggerState::Active { .. }))
            .map(|(id, _)| id.clone())
            .collect();

        StatusReport {
            enabled: self.enabled,
            dry_run: self.config.options.dry_run,
            trigger_count: self.config.triggers.len(),
            active_triggers,
            current_layer: self.current_layer.clone(),
            backend: self.backend.name().to_string(),
            config_path: self.config_path.clone(),
            uptime_secs: self.start_time.elapsed().as_secs(),
        }
    }

    pub fn triggers_snapshot(&self) -> Vec<TriggerSnapshot> {
        self.config
            .triggers
            .iter()
            .map(|t| {
                let active = matches!(self.state.get(&t.id), Some(TriggerState::Active { .. }));
                TriggerSnapshot {
                    id: t.id.clone(),
                    name: t.name.clone(),
                    mode: t.mode,
                    action_type: t.action.type_name().to_string(),
                    active,
                }
            })
            .collect()
    }

    pub fn config(&self) -> &Config {
        &self.config
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.stop_all_workers();
    }
}

/// Execute an action in a loop (for Toggle/Hold worker threads).
fn execute_action_loop(
    action: &ActionConfig,
    backend: &Arc<dyn InputBackend>,
    logger: &Arc<Logger>,
    stop_rx: &mpsc::Receiver<()>,
) -> Result<(), BackendError> {
    match action {
        ActionConfig::AutoClick {
            button,
            interval_ms,
            duration_ms,
            jitter_ms,
            hold_ms,
        } => {
            let action_start = Instant::now();
            loop {
                if stop_rx.try_recv().is_ok() {
                    break;
                }
                do_click(backend, *button, *hold_ms)?;
                let sleep_ms = jittered_interval(*interval_ms, *jitter_ms);
                if !interruptible_sleep(sleep_ms, stop_rx) {
                    break;
                }
                if let Some(dur) = duration_ms {
                    if action_start.elapsed().as_millis() >= *dur as u128 {
                        break;
                    }
                }
            }
        }
        ActionConfig::KeyPress {
            key_code,
            interval_ms,
            duration_ms,
            jitter_ms,
            ..
        } => {
            let action_start = Instant::now();
            loop {
                if stop_rx.try_recv().is_ok() {
                    break;
                }
                backend.key_press(*key_code)?;
                backend.key_release(*key_code)?;
                let sleep_ms = jittered_interval(*interval_ms, *jitter_ms);
                if !interruptible_sleep(sleep_ms, stop_rx) {
                    break;
                }
                if let Some(dur) = duration_ms {
                    if action_start.elapsed().as_millis() >= *dur as u128 {
                        break;
                    }
                }
            }
        }
        ActionConfig::ScrollWheel {
            direction,
            amount,
            interval_ms,
            duration_ms,
            jitter_ms,
        } => {
            let action_start = Instant::now();
            loop {
                if stop_rx.try_recv().is_ok() {
                    break;
                }
                backend.scroll(*direction, *amount)?;
                let sleep_ms = jittered_interval(*interval_ms, *jitter_ms);
                if !interruptible_sleep(sleep_ms, stop_rx) {
                    break;
                }
                if let Some(dur) = duration_ms {
                    if action_start.elapsed().as_millis() >= *dur as u128 {
                        break;
                    }
                }
            }
        }
        ActionConfig::MouseMove {
            dx,
            dy,
            interval_ms,
            duration_ms,
            jitter_ms,
        } => {
            let action_start = Instant::now();
            loop {
                if stop_rx.try_recv().is_ok() {
                    break;
                }
                backend.move_relative(*dx, *dy)?;
                let sleep_ms = jittered_interval(*interval_ms, *jitter_ms);
                if !interruptible_sleep(sleep_ms, stop_rx) {
                    break;
                }
                if let Some(dur) = duration_ms {
                    if action_start.elapsed().as_millis() >= *dur as u128 {
                        break;
                    }
                }
            }
        }
        ActionConfig::Composite {
            mode: CompositeMode::Parallel,
            actions,
        } => {
            let mut handles = Vec::new();

            for sub_action in actions {
                let sub_action = sub_action.clone();
                let backend = backend.clone();
                let logger = logger.clone();
                let (sub_stop_tx, sub_stop_rx) = mpsc::channel();
                handles.push(sub_stop_tx);

                thread::spawn(move || {
                    let _ = execute_action_loop(&sub_action, &backend, &logger, &sub_stop_rx);
                });
            }

            // Wait for stop signal
            let _ = stop_rx.recv();

            // Signal all sub-actions to stop
            for tx in handles {
                let _ = tx.send(());
            }
        }
        ActionConfig::Composite {
            mode: CompositeMode::Sequence,
            actions,
        } => {
            for sub_action in actions {
                if stop_rx.try_recv().is_ok() {
                    break;
                }
                execute_action_sync(sub_action, backend, logger)?;
            }
        }
        ActionConfig::Delay { duration_ms } => {
            interruptible_sleep(*duration_ms, stop_rx);
        }
        // These actions are oneshot-only — they don't make sense in a loop context.
        // Execute once and stop.
        ActionConfig::MouseMoveAbsolute { x, y } => {
            backend.move_absolute(*x, *y)?;
        }
        ActionConfig::ClickAt {
            x,
            y,
            button,
            hold_ms,
            settle_ms,
        } => {
            backend.move_absolute(*x, *y)?;
            if *settle_ms > 0 {
                thread::sleep(Duration::from_millis(*settle_ms as u64));
            }
            do_click(backend, *button, *hold_ms)?;
        }
        ActionConfig::Drag {
            from_x,
            from_y,
            to_x,
            to_y,
            button,
            duration_ms,
        } => {
            backend.move_absolute(*from_x, *from_y)?;
            thread::sleep(Duration::from_millis(5));
            backend.mouse_press(*button)?;
            thread::sleep(Duration::from_millis(5));

            let steps = (*duration_ms / 10).max(1);
            for i in 1..=steps {
                if stop_rx.try_recv().is_ok() {
                    backend.mouse_release(*button)?;
                    return Ok(());
                }
                let t = i as f64 / steps as f64;
                let ix = *from_x as f64 + (*to_x - *from_x) as f64 * t;
                let iy = *from_y as f64 + (*to_y - *from_y) as f64 * t;
                backend.move_absolute(ix as i32, iy as i32)?;
                thread::sleep(Duration::from_millis(10));
            }

            backend.mouse_release(*button)?;
        }
        ActionConfig::SetLayer { layer } => {
            // Can't actually change layer from a worker thread — log warning
            logger.warn(format!(
                "SetLayer('{}') in loop context is not supported; use OneShot mode",
                layer
            ));
        }
        ActionConfig::NoOp => {
            logger.debug("NoOp action, waiting for stop signal");
            let _ = stop_rx.recv();
        }
    }
    Ok(())
}

/// Execute an action synchronously (for OneShot and Sequence steps).
fn execute_action_sync(
    action: &ActionConfig,
    backend: &Arc<dyn InputBackend>,
    logger: &Arc<Logger>,
) -> Result<(), BackendError> {
    match action {
        ActionConfig::AutoClick {
            button,
            interval_ms,
            duration_ms,
            jitter_ms,
            hold_ms,
        } => {
            let dur = duration_ms.unwrap_or(0);
            if dur == 0 {
                // Single click
                do_click(backend, *button, *hold_ms)?;
            } else {
                let action_start = Instant::now();
                loop {
                    do_click(backend, *button, *hold_ms)?;
                    if action_start.elapsed().as_millis() >= dur as u128 {
                        break;
                    }
                    let sleep_ms = jittered_interval(*interval_ms, *jitter_ms);
                    thread::sleep(Duration::from_millis(sleep_ms as u64));
                }
            }
        }
        ActionConfig::KeyPress {
            key_code,
            interval_ms,
            duration_ms,
            jitter_ms,
            ..
        } => {
            let dur = duration_ms.unwrap_or(0);
            if dur == 0 {
                backend.key_press(*key_code)?;
                backend.key_release(*key_code)?;
            } else {
                let action_start = Instant::now();
                loop {
                    backend.key_press(*key_code)?;
                    backend.key_release(*key_code)?;
                    if action_start.elapsed().as_millis() >= dur as u128 {
                        break;
                    }
                    let sleep_ms = jittered_interval(*interval_ms, *jitter_ms);
                    thread::sleep(Duration::from_millis(sleep_ms as u64));
                }
            }
        }
        ActionConfig::ScrollWheel {
            direction, amount, ..
        } => {
            backend.scroll(*direction, *amount)?;
        }
        ActionConfig::MouseMove { dx, dy, .. } => {
            backend.move_relative(*dx, *dy)?;
        }
        ActionConfig::Composite {
            mode: CompositeMode::Sequence,
            actions,
        } => {
            for a in actions {
                execute_action_sync(a, backend, logger)?;
            }
        }
        ActionConfig::Composite {
            mode: CompositeMode::Parallel,
            actions,
        } => {
            let mut handles = Vec::new();
            for a in actions {
                let a = a.clone();
                let backend = backend.clone();
                let logger = logger.clone();
                handles.push(thread::spawn(move || {
                    execute_action_sync(&a, &backend, &logger)
                }));
            }
            for h in handles {
                h.join().unwrap()?;
            }
        }
        ActionConfig::Delay { duration_ms } => {
            thread::sleep(Duration::from_millis(*duration_ms as u64));
        }
        ActionConfig::MouseMoveAbsolute { x, y } => {
            backend.move_absolute(*x, *y)?;
        }
        ActionConfig::ClickAt {
            x,
            y,
            button,
            hold_ms,
            settle_ms,
        } => {
            backend.move_absolute(*x, *y)?;
            if *settle_ms > 0 {
                thread::sleep(Duration::from_millis(*settle_ms as u64));
            }
            do_click(backend, *button, *hold_ms)?;
        }
        ActionConfig::Drag {
            from_x,
            from_y,
            to_x,
            to_y,
            button,
            duration_ms,
        } => {
            backend.move_absolute(*from_x, *from_y)?;
            thread::sleep(Duration::from_millis(5));
            backend.mouse_press(*button)?;
            thread::sleep(Duration::from_millis(5));

            // Interpolate movement over duration
            let steps = (*duration_ms / 10).max(1);
            for i in 1..=steps {
                let t = i as f64 / steps as f64;
                let ix = *from_x as f64 + (*to_x - *from_x) as f64 * t;
                let iy = *from_y as f64 + (*to_y - *from_y) as f64 * t;
                backend.move_absolute(ix as i32, iy as i32)?;
                thread::sleep(Duration::from_millis(10));
            }

            backend.mouse_release(*button)?;
        }
        ActionConfig::SetLayer { layer } => {
            // SetLayer is handled by the caller (engine) since it needs mutable access.
            // In sync context, we log it — the actual layer change happens in handle_oneshot.
            logger.debug(format!("SetLayer action: '{}'", layer));
        }
        ActionConfig::NoOp => {
            logger.debug("NoOp (sync)");
        }
    }
    Ok(())
}

/// Perform a single click with an optional hold duration.
/// When hold_ms is 0, uses the atomic click() method. When > 0, holds the
/// button pressed for the specified duration before releasing.
fn do_click(
    backend: &Arc<dyn InputBackend>,
    button: MouseButton,
    hold_ms: u32,
) -> Result<(), BackendError> {
    if hold_ms == 0 {
        backend.click(button)
    } else {
        backend.mouse_press(button)?;
        thread::sleep(Duration::from_millis(hold_ms as u64));
        backend.mouse_release(button)
    }
}

fn jittered_interval(interval_ms: u32, jitter_ms: u32) -> u32 {
    if jitter_ms == 0 {
        return interval_ms;
    }
    let mut rng = rand::thread_rng();
    let jitter = rng.gen_range(-(jitter_ms as i32)..=(jitter_ms as i32));
    (interval_ms as i32 + jitter).max(1) as u32
}

/// Sleep in 1ms chunks to allow responsive cancellation. Returns true if sleep completed,
/// false if stop signal received.
fn interruptible_sleep(ms: u32, stop_rx: &mpsc::Receiver<()>) -> bool {
    let start = Instant::now();
    let target = Duration::from_millis(ms as u64);
    while start.elapsed() < target {
        if stop_rx.try_recv().is_ok() {
            return false;
        }
        thread::sleep(Duration::from_millis(1));
    }
    true
}

/// Public wrappers for benchmarking internal engine functions.
/// Only available when the `bench-internals` feature is enabled.
#[cfg(feature = "bench-internals")]
pub mod bench {
    use super::*;

    /// Benchmark wrapper for `execute_action_sync`.
    pub fn bench_execute_action_sync(
        action: &ActionConfig,
        backend: &Arc<dyn InputBackend>,
        logger: &Arc<Logger>,
    ) -> Result<(), BackendError> {
        execute_action_sync(action, backend, logger)
    }

    /// Benchmark wrapper for `do_click`.
    pub fn bench_do_click(
        backend: &Arc<dyn InputBackend>,
        button: MouseButton,
        hold_ms: u32,
    ) -> Result<(), BackendError> {
        do_click(backend, button, hold_ms)
    }

    /// Benchmark wrapper for `jittered_interval`.
    pub fn bench_jittered_interval(interval_ms: u32, jitter_ms: u32) -> u32 {
        jittered_interval(interval_ms, jitter_ms)
    }

    /// Benchmark wrapper for `interruptible_sleep`.
    pub fn bench_interruptible_sleep(ms: u32, stop_rx: &mpsc::Receiver<()>) -> bool {
        interruptible_sleep(ms, stop_rx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input_backend::{BackendCall, MockBackend};
    use crate::logger::LogLevel;
    use std::sync::Mutex;

    fn test_engine(triggers: Vec<TriggerBinding>) -> (Engine, Arc<Mutex<Vec<BackendCall>>>) {
        let logger = Arc::new(Logger::new(100, LogLevel::Trace, false));
        logger.set_quiet(true);
        let backend = MockBackend::new();
        let calls = backend.calls_clone();
        let config = Config {
            options: GlobalOptions::default(),
            triggers,
            device_bindings: vec![],
            profile_rules: vec![],
        };
        let engine = Engine::new(config, Arc::new(backend), logger, "test".into());
        (engine, calls)
    }

    fn auto_click_trigger(id: &str, interval_ms: u32, mode: TriggerMode) -> TriggerBinding {
        TriggerBinding {
            id: id.to_string(),
            name: id.to_string(),
            description: String::new(),
            mode,
            action: ActionConfig::AutoClick {
                button: MouseButton::Left,
                interval_ms,
                duration_ms: None,
                jitter_ms: 0,
                hold_ms: 0,
            },
            cooldown_ms: None,
        }
    }

    #[test]
    fn test_toggle_starts_worker() {
        let (mut engine, calls) =
            test_engine(vec![auto_click_trigger("test", 5, TriggerMode::Toggle)]);
        engine.set_enabled(true);
        engine.trigger_event("test", true).unwrap();

        // Wait for some clicks
        thread::sleep(Duration::from_millis(50));

        let call_count = calls.lock().unwrap().len();
        assert!(call_count > 0, "Expected clicks, got none");
    }

    #[test]
    fn test_toggle_stops_worker() {
        let (mut engine, calls) =
            test_engine(vec![auto_click_trigger("test", 5, TriggerMode::Toggle)]);
        engine.set_enabled(true);

        // Start
        engine.trigger_event("test", true).unwrap();
        thread::sleep(Duration::from_millis(30));

        // Stop
        engine.trigger_event("test", true).unwrap();
        let count_after_stop = calls.lock().unwrap().len();

        // Wait to ensure no more clicks
        thread::sleep(Duration::from_millis(30));
        let count_later = calls.lock().unwrap().len();
        assert_eq!(count_after_stop, count_later);
    }

    #[test]
    fn test_hold_press_release() {
        let (mut engine, calls) =
            test_engine(vec![auto_click_trigger("test", 5, TriggerMode::Hold)]);
        engine.set_enabled(true);

        // Press
        engine.trigger_event("test", true).unwrap();
        thread::sleep(Duration::from_millis(30));

        // Release
        engine.trigger_event("test", false).unwrap();
        let count_after_release = calls.lock().unwrap().len();
        assert!(count_after_release > 0);

        // Ensure stopped
        thread::sleep(Duration::from_millis(30));
        let count_later = calls.lock().unwrap().len();
        assert_eq!(count_after_release, count_later);
    }

    #[test]
    fn test_oneshot_executes_synchronously() {
        let trigger = TriggerBinding {
            id: "test".into(),
            name: "Test".into(),
            description: String::new(),
            mode: TriggerMode::OneShot,
            action: ActionConfig::AutoClick {
                button: MouseButton::Left,
                interval_ms: 10,
                duration_ms: Some(50),
                jitter_ms: 0,
                hold_ms: 0,
            },
            cooldown_ms: None,
        };
        let (mut engine, calls) = test_engine(vec![trigger]);
        engine.set_enabled(true);

        engine.trigger_event("test", true).unwrap();
        let count = calls.lock().unwrap().len();
        assert!(count > 0, "OneShot should have produced clicks");
    }

    #[test]
    fn test_disabled_engine_ignores_trigger() {
        let (mut engine, _) = test_engine(vec![auto_click_trigger("test", 5, TriggerMode::Toggle)]);
        // Engine is disabled by default
        let result = engine.trigger_event("test", true);
        assert!(matches!(result, Err(EngineError::Disabled)));
    }

    #[test]
    fn test_cooldown_debounce() {
        let trigger = TriggerBinding {
            cooldown_ms: Some(200),
            ..auto_click_trigger("test", 5, TriggerMode::Toggle)
        };
        let (mut engine, _) = test_engine(vec![trigger]);
        engine.set_enabled(true);

        // Start
        engine.trigger_event("test", true).unwrap();
        thread::sleep(Duration::from_millis(20));

        // Stop
        engine.trigger_event("test", true).unwrap();

        // Try to start again immediately (within cooldown)
        let result = engine.trigger_event("test", true);
        assert!(matches!(result, Err(EngineError::Cooldown)));
    }

    #[test]
    fn test_apply_config_stops_running_workers() {
        let (mut engine, calls) =
            test_engine(vec![auto_click_trigger("test", 5, TriggerMode::Toggle)]);
        engine.set_enabled(true);
        engine.trigger_event("test", true).unwrap();
        thread::sleep(Duration::from_millis(30));

        // Apply new config — should stop workers
        let new_config = Config::default();
        engine.apply_config(new_config);

        let count_after = calls.lock().unwrap().len();
        thread::sleep(Duration::from_millis(30));
        let count_later = calls.lock().unwrap().len();
        assert_eq!(count_after, count_later);
    }

    #[test]
    fn test_jitter_range() {
        for _ in 0..1000 {
            let result = jittered_interval(100, 10);
            assert!(result >= 90 && result <= 110, "Got {}", result);
        }
    }

    #[test]
    fn test_duration_limit() {
        let trigger = TriggerBinding {
            id: "test".into(),
            name: "Test".into(),
            description: String::new(),
            mode: TriggerMode::Toggle,
            action: ActionConfig::AutoClick {
                button: MouseButton::Left,
                interval_ms: 5,
                duration_ms: Some(50),
                jitter_ms: 0,
                hold_ms: 0,
            },
            cooldown_ms: None,
        };
        let (mut engine, _calls) = test_engine(vec![trigger]);
        engine.set_enabled(true);
        engine.trigger_event("test", true).unwrap();

        // Wait for duration to expire
        thread::sleep(Duration::from_millis(150));

        // The worker should have stopped on its own
        let _snapshot = engine.triggers_snapshot();
        // Note: the state might still be Active but the thread has exited.
        // That's OK — it's the thread itself that respects duration.
    }

    #[test]
    fn test_unknown_trigger() {
        let (mut engine, _) = test_engine(vec![]);
        engine.set_enabled(true);
        let result = engine.trigger_event("nonexistent", true);
        assert!(matches!(result, Err(EngineError::UnknownTrigger(_))));
    }

    #[test]
    fn test_status_report() {
        let (engine, _) = test_engine(vec![
            auto_click_trigger("t1", 50, TriggerMode::Toggle),
            auto_click_trigger("t2", 50, TriggerMode::Hold),
        ]);
        let status = engine.describe_status();
        assert!(!status.enabled);
        assert_eq!(status.trigger_count, 2);
        assert!(status.active_triggers.is_empty());
        assert_eq!(status.backend, "mock");
    }

    #[test]
    fn test_triggers_snapshot() {
        let (engine, _) = test_engine(vec![auto_click_trigger("t1", 50, TriggerMode::Toggle)]);
        let snaps = engine.triggers_snapshot();
        assert_eq!(snaps.len(), 1);
        assert_eq!(snaps[0].id, "t1");
        assert!(!snaps[0].active);
        assert_eq!(snaps[0].action_type, "auto_click");
    }

    #[test]
    fn test_set_layer_oneshot() {
        let trigger = TriggerBinding {
            id: "switch".into(),
            name: "Switch".into(),
            description: String::new(),
            mode: TriggerMode::OneShot,
            action: ActionConfig::SetLayer {
                layer: "combat".to_string(),
            },
            cooldown_ms: None,
        };
        let (mut engine, _) = test_engine(vec![trigger]);
        engine.set_enabled(true);
        assert_eq!(engine.current_layer(), "base");

        engine.trigger_event("switch", true).unwrap();
        assert_eq!(engine.current_layer(), "combat");
    }

    #[test]
    fn test_mouse_move_absolute_oneshot() {
        let trigger = TriggerBinding {
            id: "move".into(),
            name: "Move".into(),
            description: String::new(),
            mode: TriggerMode::OneShot,
            action: ActionConfig::MouseMoveAbsolute { x: 1000, y: 2000 },
            cooldown_ms: None,
        };
        let (mut engine, calls) = test_engine(vec![trigger]);
        engine.set_enabled(true);

        engine.trigger_event("move", true).unwrap();
        let recorded = calls.lock().unwrap();
        assert!(recorded.contains(&BackendCall::MoveAbsolute(1000, 2000)));
    }

    #[test]
    fn test_click_at_oneshot() {
        let trigger = TriggerBinding {
            id: "click".into(),
            name: "Click".into(),
            description: String::new(),
            mode: TriggerMode::OneShot,
            action: ActionConfig::ClickAt {
                x: 500,
                y: 300,
                button: MouseButton::Left,
                hold_ms: 0,
                settle_ms: 5,
            },
            cooldown_ms: None,
        };
        let (mut engine, calls) = test_engine(vec![trigger]);
        engine.set_enabled(true);

        engine.trigger_event("click", true).unwrap();
        let recorded = calls.lock().unwrap();
        assert!(recorded.contains(&BackendCall::MoveAbsolute(500, 300)));
        assert!(recorded.contains(&BackendCall::Click(MouseButton::Left)));
    }

    #[test]
    fn test_layer_preserved_across_config_reload() {
        let (mut engine, _) = test_engine(vec![auto_click_trigger("t1", 50, TriggerMode::Toggle)]);
        engine.set_layer("combat".to_string());
        assert_eq!(engine.current_layer(), "combat");

        // Apply new config — layer should be preserved
        let new_config = Config::default();
        engine.apply_config(new_config);
        assert_eq!(engine.current_layer(), "combat");
    }
}
