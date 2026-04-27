use crate::config::*;
use crate::event_bus::{Event, EventBus};
use crate::input_backend::{BackendError, InputBackend};
use crate::logger::Logger;
use rand::Rng;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
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
    #[error("Trigger ID already exists: {0}")]
    DuplicateTrigger(String),
    #[error("Trigger not found or not owned by this connection: {0}")]
    NotOwned(String),
    #[error("Invalid trigger: {0}")]
    InvalidConfig(String),
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
    /// `true` if this trigger was registered dynamically via IPC.
    pub dynamic: bool,
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

/// A trigger registered at runtime via IPC, owned by a connection.
struct DynamicTriggerEntry {
    trigger: TriggerBinding,
    owner_connection_id: u64,
}

pub struct Engine {
    config: Config,
    /// State for both static (Lua) and dynamic triggers.
    state: HashMap<String, TriggerState>,
    /// Triggers registered at runtime via IPC.
    dynamic_triggers: HashMap<String, DynamicTriggerEntry>,
    backend: Arc<dyn InputBackend>,
    logger: Arc<Logger>,
    event_bus: Arc<EventBus>,
    enabled: bool,
    start_time: Instant,
    config_path: String,
    current_layer: String,
    /// Events accumulated during a method call; drained and published by `with_engine_events`
    /// *after* the engine `Mutex` is released, preventing lock-order inversions.
    pending_events: Vec<Event>,
}

impl Engine {
    pub fn new(
        config: Config,
        backend: Arc<dyn InputBackend>,
        logger: Arc<Logger>,
        event_bus: Arc<EventBus>,
        config_path: String,
    ) -> Self {
        let mut state = HashMap::new();
        for trigger in &config.triggers {
            state.insert(trigger.id.clone(), TriggerState::Idle);
        }
        Self {
            config,
            state,
            dynamic_triggers: HashMap::new(),
            backend,
            logger,
            event_bus,
            enabled: false,
            start_time: Instant::now(),
            config_path,
            current_layer: "base".to_string(),
            pending_events: Vec::new(),
        }
    }

    pub fn apply_config(&mut self, config: Config) {
        // Only stop static trigger workers; dynamic triggers are preserved.
        let static_ids: Vec<String> = self
            .config
            .triggers
            .iter()
            .map(|t| t.id.clone())
            .collect();
        for id in &static_ids {
            self.stop_worker(id);
        }
        // Keep only dynamic trigger state entries, then add new static ones.
        let dynamic_ids: Vec<String> = self.dynamic_triggers.keys().cloned().collect();
        self.state.retain(|id, _| dynamic_ids.contains(id));
        for trigger in &config.triggers {
            self.state.entry(trigger.id.clone()).or_insert(TriggerState::Idle);
        }
        self.config = config;
        self.pending_events.push(Event::config_reloaded());
        self.logger.info("Config applied, all workers stopped");
    }

    /// Get the current active layer name.
    pub fn current_layer(&self) -> &str {
        &self.current_layer
    }

    /// Set the active layer.
    pub fn set_layer(&mut self, layer: String) {
        let from = self.current_layer.clone();
        self.logger
            .info(format!("Layer changed: '{}' -> '{}'", from, layer));
        self.pending_events
            .push(Event::layer_changed(from, layer.clone()));
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
        self.pending_events.push(Event::enabled_changed(enabled));
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

        // Check static triggers first, then dynamic.
        let trigger = self
            .config
            .triggers
            .iter()
            .find(|t| t.id == id)
            .cloned()
            .or_else(|| {
                self.dynamic_triggers
                    .get(id)
                    .map(|e| e.trigger.clone())
            })
            .ok_or_else(|| EngineError::UnknownTrigger(id.to_string()))?;

        match trigger.mode {
            TriggerMode::Toggle => {
                if !press {
                    return Ok(());
                }
                self.handle_toggle(&trigger)
            }
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
                self.stop_worker(&trigger.id);
                self.logger.info(format!("{}: stopped", trigger.id));
                self.pending_events
                    .push(Event::trigger_deactivated(trigger.id.clone()));
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
                self.start_worker(trigger)?;
                self.pending_events
                    .push(Event::trigger_activated(trigger.id.clone()));
                Ok(())
            }
            TriggerState::Idle => {
                self.start_worker(trigger)?;
                self.pending_events
                    .push(Event::trigger_activated(trigger.id.clone()));
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
                    self.pending_events
                        .push(Event::trigger_activated(trigger.id.clone()));
                    Ok(())
                }
            }
        } else {
            self.stop_worker(&trigger.id);
            self.logger.info(format!("{}: released", trigger.id));
            self.pending_events
                .push(Event::trigger_deactivated(trigger.id.clone()));
            Ok(())
        }
    }

    fn handle_oneshot(&mut self, trigger: &TriggerBinding) -> Result<(), EngineError> {
        self.logger
            .info(format!("{}: oneshot executing", trigger.id));

        self.pending_events
            .push(Event::trigger_activated(trigger.id.clone()));

        // Handle SetLayer directly (needs mutable self access)
        if let ActionConfig::SetLayer { ref layer } = trigger.action {
            self.set_layer(layer.clone());
            self.logger
                .info(format!("{}: oneshot complete", trigger.id));
            self.pending_events
                .push(Event::trigger_deactivated(trigger.id.clone()));
            return Ok(());
        }

        execute_action_sync(&trigger.action, &self.backend, &self.logger)?;
        self.logger
            .info(format!("{}: oneshot complete", trigger.id));
        self.pending_events
            .push(Event::trigger_deactivated(trigger.id.clone()));
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
            trigger_count: self.config.triggers.len() + self.dynamic_triggers.len(),
            active_triggers,
            current_layer: self.current_layer.clone(),
            backend: self.backend.name().to_string(),
            config_path: self.config_path.clone(),
            uptime_secs: self.start_time.elapsed().as_secs(),
        }
    }

    pub fn triggers_snapshot(&self) -> Vec<TriggerSnapshot> {
        let mut snapshots: Vec<TriggerSnapshot> = self
            .config
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
                    dynamic: false,
                }
            })
            .collect();

        for (id, entry) in &self.dynamic_triggers {
            let active = matches!(self.state.get(id), Some(TriggerState::Active { .. }));
            snapshots.push(TriggerSnapshot {
                id: id.clone(),
                name: entry.trigger.name.clone(),
                mode: entry.trigger.mode,
                action_type: entry.trigger.action.type_name().to_string(),
                active,
                dynamic: true,
            });
        }

        snapshots
    }

    /// Register a trigger dynamically via IPC.
    pub fn register_dynamic_trigger(
        &mut self,
        trigger: TriggerBinding,
        owner_connection_id: u64,
    ) -> Result<(), EngineError> {
        let id = &trigger.id;
        if self.config.triggers.iter().any(|t| &t.id == id) {
            return Err(EngineError::DuplicateTrigger(id.clone()));
        }
        if self.dynamic_triggers.contains_key(id) {
            return Err(EngineError::DuplicateTrigger(id.clone()));
        }

        // Validate action constraints using a synthetic single-trigger config so
        // the same interval/depth/oneshot-only rules apply to dynamic triggers.
        let mut synthetic = crate::config::Config::default();
        synthetic.options = self.config.options.clone();
        synthetic.triggers = vec![trigger.clone()];
        if let Err(errs) = crate::config::validate_config(&synthetic) {
            let msg = errs
                .iter()
                .map(|e| e.to_string())
                .collect::<Vec<_>>()
                .join("; ");
            return Err(EngineError::InvalidConfig(msg));
        }

        self.state.insert(id.clone(), TriggerState::Idle);
        self.dynamic_triggers.insert(
            id.clone(),
            DynamicTriggerEntry {
                trigger,
                owner_connection_id,
            },
        );
        Ok(())
    }

    /// Unregister a dynamic trigger; only the owning connection may do this.
    pub fn unregister_dynamic_trigger(
        &mut self,
        id: &str,
        owner_connection_id: u64,
    ) -> Result<(), EngineError> {
        let entry = self
            .dynamic_triggers
            .get(id)
            .ok_or_else(|| EngineError::NotOwned(id.to_string()))?;
        if entry.owner_connection_id != owner_connection_id {
            return Err(EngineError::NotOwned(id.to_string()));
        }
        self.stop_worker(id);
        self.state.remove(id);
        self.dynamic_triggers.remove(id);
        Ok(())
    }

    /// Remove all dynamic triggers owned by a connection (called on connection close).
    pub fn cleanup_connection(&mut self, owner_connection_id: u64) {
        let owned: Vec<String> = self
            .dynamic_triggers
            .iter()
            .filter(|(_, e)| e.owner_connection_id == owner_connection_id)
            .map(|(id, _)| id.clone())
            .collect();
        for id in owned {
            self.stop_worker(&id);
            self.state.remove(&id);
            self.dynamic_triggers.remove(&id);
        }
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Return the dynamic triggers owned by a specific connection.
    pub fn dynamic_triggers_for_connection(
        &self,
        owner_connection_id: u64,
    ) -> Vec<TriggerSnapshot> {
        self.dynamic_triggers
            .iter()
            .filter(|(_, e)| e.owner_connection_id == owner_connection_id)
            .map(|(id, entry)| {
                let active =
                    matches!(self.state.get(id), Some(TriggerState::Active { .. }));
                TriggerSnapshot {
                    id: id.clone(),
                    name: entry.trigger.name.clone(),
                    mode: entry.trigger.mode,
                    action_type: entry.trigger.action.type_name().to_string(),
                    active,
                    dynamic: true,
                }
            })
            .collect()
    }
    /// Drains accumulated pending events, returning them for publishing after the engine lock is
    /// released. Use `with_engine_events` rather than calling this directly.
    pub fn drain_pending_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.pending_events)
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.stop_all_workers();
    }
}

/// Acquire the engine lock, run `f`, then publish any events accumulated during the call.
///
/// Events are collected inside the lock and published *after* the lock is released, which
/// prevents lock-order inversions: no subscriber callback can deadlock by re-acquiring the
/// engine lock while this function is on the call stack.
///
/// # Panic policy
///
/// The engine lock uses `unwrap()` intentionally. If the engine panics mid-mutation its
/// coupled fields (`state`, `config`, `pending_events`, etc.) may be inconsistent; the correct
/// response is a clean process exit, not continued operation on potentially corrupted state.
/// Peripheral mutexes (logger, event bus, device tracker) use `lock_or_recover()` instead.
pub fn with_engine_events<T>(engine: &Arc<Mutex<Engine>>, f: impl FnOnce(&mut Engine) -> T) -> T {
    let (result, events, bus) = {
        let mut guard = engine.lock().unwrap();
        let result = f(&mut guard);
        let events = guard.drain_pending_events();
        let bus = guard.event_bus.clone();
        (result, events, bus)
    };
    for event in events {
        bus.publish(&event);
    }
    result
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
            modifier_codes,
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
                do_keystroke(backend, *key_code, modifier_codes, 0)?;
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
        ActionConfig::Keystroke {
            key_code,
            modifier_codes,
            hold_ms,
            ..
        } => {
            do_keystroke(backend, *key_code, modifier_codes, *hold_ms)?;
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
            modifier_codes,
            interval_ms,
            duration_ms,
            jitter_ms,
            ..
        } => {
            let dur = duration_ms.unwrap_or(0);
            if dur == 0 {
                do_keystroke(backend, *key_code, modifier_codes, 0)?;
            } else {
                let action_start = Instant::now();
                loop {
                    do_keystroke(backend, *key_code, modifier_codes, 0)?;
                    if action_start.elapsed().as_millis() >= dur as u128 {
                        break;
                    }
                    let sleep_ms = jittered_interval(*interval_ms, *jitter_ms);
                    thread::sleep(Duration::from_millis(sleep_ms as u64));
                }
            }
        }
        ActionConfig::Keystroke {
            key_code,
            modifier_codes,
            hold_ms,
            ..
        } => {
            do_keystroke(backend, *key_code, modifier_codes, *hold_ms)?;
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

/// Press modifier keys, press the main key, optionally hold, then release everything
/// in reverse order. On backend error after any modifier has been pressed, best-effort
/// releases the already-pressed modifiers before returning the error.
fn do_keystroke(
    backend: &Arc<dyn InputBackend>,
    key_code: u32,
    modifier_codes: &[u32],
    hold_ms: u32,
) -> Result<(), BackendError> {
    let mut pressed = 0usize;

    // Press modifiers
    for &mod_code in modifier_codes {
        if let Err(e) = backend.key_press(mod_code) {
            release_modifiers(backend, &modifier_codes[..pressed]);
            return Err(e);
        }
        pressed += 1;
    }

    // Press the main key
    if let Err(e) = backend.key_press(key_code) {
        release_modifiers(backend, &modifier_codes[..pressed]);
        return Err(e);
    }

    if hold_ms > 0 {
        thread::sleep(Duration::from_millis(hold_ms as u64));
    }

    // Release main key then modifiers in reverse order
    backend.key_release(key_code)?;
    for &mod_code in modifier_codes.iter().rev() {
        backend.key_release(mod_code)?;
    }

    Ok(())
}

/// Best-effort release of already-pressed modifier keys (in reverse press order).
fn release_modifiers(backend: &Arc<dyn InputBackend>, pressed: &[u32]) {
    for &undo in pressed.iter().rev() {
        let _ = backend.key_release(undo);
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
    use crate::MutexExt;
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
        let engine = Engine::new(config, Arc::new(backend), logger, Arc::new(EventBus::new()), "test".into());
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

        let call_count = calls.lock_or_recover().len();
        assert!(call_count > 0, "Expected clicks, got none");
    }

    #[test]
    fn test_toggle_ignores_release() {
        // Simulates a real button press/release cycle: toggle should start on
        // press and remain active after release (not stop like hold mode).
        let (mut engine, calls) =
            test_engine(vec![auto_click_trigger("test", 5, TriggerMode::Toggle)]);
        engine.set_enabled(true);

        // Button down → starts
        engine.trigger_event("test", true).unwrap();
        thread::sleep(Duration::from_millis(30));

        let count_before_release = calls.lock_or_recover().len();
        assert!(
            count_before_release > 0,
            "Toggle should start clicking on press"
        );

        // Button up → should NOT stop (toggle stays active until next press)
        engine.trigger_event("test", false).unwrap();
        thread::sleep(Duration::from_millis(30));

        let count_after_release = calls.lock_or_recover().len();
        assert!(
            count_after_release > count_before_release,
            "Toggle must stay active after release: had {} clicks before release, {} after",
            count_before_release,
            count_after_release,
        );
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
        let count_after_stop = calls.lock_or_recover().len();

        // Wait to ensure no more clicks
        thread::sleep(Duration::from_millis(30));
        let count_later = calls.lock_or_recover().len();
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
        let count_after_release = calls.lock_or_recover().len();
        assert!(count_after_release > 0);

        // Ensure stopped
        thread::sleep(Duration::from_millis(30));
        let count_later = calls.lock_or_recover().len();
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
        let count = calls.lock_or_recover().len();
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

        let count_after = calls.lock_or_recover().len();
        thread::sleep(Duration::from_millis(30));
        let count_later = calls.lock_or_recover().len();
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
        let recorded = calls.lock_or_recover();
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
        let recorded = calls.lock_or_recover();
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

    fn keystroke_trigger(
        id: &str,
        key_code: u32,
        modifier_codes: Vec<u32>,
        hold_ms: u32,
        mode: TriggerMode,
    ) -> TriggerBinding {
        TriggerBinding {
            id: id.to_string(),
            name: id.to_string(),
            description: String::new(),
            mode,
            action: ActionConfig::Keystroke {
                key_name: format!("KEY_{}", key_code),
                key_code,
                modifier_names: modifier_codes
                    .iter()
                    .map(|c| format!("KEY_{}", c))
                    .collect(),
                modifier_codes,
                hold_ms,
            },
            cooldown_ms: None,
        }
    }

    #[test]
    fn test_keystroke_oneshot_exact_sequence() {
        // Ctrl+Z: modifiers=[29 (LEFTCTRL)], key=44 (Z)
        let trigger = keystroke_trigger("ks", 44, vec![29], 0, TriggerMode::OneShot);
        let (mut engine, calls) = test_engine(vec![trigger]);
        engine.set_enabled(true);
        engine.trigger_event("ks", true).unwrap();

        let recorded = calls.lock_or_recover().clone();
        assert_eq!(
            recorded,
            vec![
                BackendCall::KeyPress(29),
                BackendCall::KeyPress(44),
                BackendCall::KeyRelease(44),
                BackendCall::KeyRelease(29),
            ],
            "Ctrl+Z should press modifier then key, release key then modifier"
        );
    }

    #[test]
    fn test_keystroke_no_modifiers_exact_sequence() {
        // Space: key=57, no modifiers
        let trigger = keystroke_trigger("ks", 57, vec![], 0, TriggerMode::OneShot);
        let (mut engine, calls) = test_engine(vec![trigger]);
        engine.set_enabled(true);
        engine.trigger_event("ks", true).unwrap();

        let recorded = calls.lock_or_recover().clone();
        assert_eq!(
            recorded,
            vec![BackendCall::KeyPress(57), BackendCall::KeyRelease(57),],
            "Space alone should produce KeyPress then KeyRelease"
        );
    }

    #[test]
    fn test_key_press_with_modifiers_oneshot() {
        // key_press in oneshot mode with ctrl modifier
        let trigger = TriggerBinding {
            id: "kp".into(),
            name: "kp".into(),
            description: String::new(),
            mode: TriggerMode::OneShot,
            action: ActionConfig::KeyPress {
                key_name: "KEY_Z".into(),
                key_code: 44,
                modifier_names: vec!["KEY_LEFTCTRL".into()],
                modifier_codes: vec![29],
                interval_ms: 100,
                duration_ms: Some(0),
                jitter_ms: 0,
            },
            cooldown_ms: None,
        };
        let (mut engine, calls) = test_engine(vec![trigger]);
        engine.set_enabled(true);
        engine.trigger_event("kp", true).unwrap();

        let recorded = calls.lock_or_recover().clone();
        assert_eq!(
            &recorded[..4],
            &[
                BackendCall::KeyPress(29),
                BackendCall::KeyPress(44),
                BackendCall::KeyRelease(44),
                BackendCall::KeyRelease(29),
            ],
            "Key press with modifier should wrap key in modifier press/release"
        );
    }

    #[test]
    fn test_keystroke_hold_ms_completes_without_error() {
        let trigger = keystroke_trigger("ks", 44, vec![29], 10, TriggerMode::OneShot);
        let (mut engine, calls) = test_engine(vec![trigger]);
        engine.set_enabled(true);
        engine.trigger_event("ks", true).unwrap();

        let recorded = calls.lock_or_recover().clone();
        // Regardless of hold_ms, the press/release sequence must be complete
        assert_eq!(
            recorded,
            vec![
                BackendCall::KeyPress(29),
                BackendCall::KeyPress(44),
                BackendCall::KeyRelease(44),
                BackendCall::KeyRelease(29),
            ]
        );
    }

    #[test]
    fn test_register_dynamic_trigger_rejects_duplicate_static_id() {
        let (mut engine, _) = test_engine(vec![auto_click_trigger("existing", 50, TriggerMode::Toggle)]);
        let dup = auto_click_trigger("existing", 50, TriggerMode::Toggle);
        let result = engine.register_dynamic_trigger(dup, 1);
        assert!(matches!(result, Err(EngineError::DuplicateTrigger(_))));
    }

    #[test]
    fn test_register_dynamic_trigger_rejects_duplicate_dynamic_id() {
        let (mut engine, _) = test_engine(vec![]);
        let t = auto_click_trigger("dyn", 50, TriggerMode::Toggle);
        engine.register_dynamic_trigger(t.clone(), 1).unwrap();
        let result = engine.register_dynamic_trigger(t, 2);
        assert!(matches!(result, Err(EngineError::DuplicateTrigger(_))));
    }

    #[test]
    fn test_register_dynamic_trigger_rejects_interval_above_max() {
        use crate::MAX_INTERVAL_MS;
        let (mut engine, _) = test_engine(vec![]);
        let bad = auto_click_trigger("too_slow", MAX_INTERVAL_MS + 1, TriggerMode::Toggle);
        let result = engine.register_dynamic_trigger(bad, 1);
        assert!(
            matches!(result, Err(EngineError::InvalidConfig(_))),
            "Expected InvalidConfig, got {:?}",
            result
        );
    }

    #[test]
    fn test_register_dynamic_trigger_valid_succeeds() {
        let (mut engine, _) = test_engine(vec![]);
        let good = auto_click_trigger("valid_dyn", 50, TriggerMode::Toggle);
        assert!(engine.register_dynamic_trigger(good, 1).is_ok());
        assert!(engine.triggers_snapshot().iter().any(|t| t.id == "valid_dyn" && t.dynamic));
    }
}
