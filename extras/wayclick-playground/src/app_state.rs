// Application state for the wayclick-playground, integrating IPC connection with
// the macroquad main loop via mpsc channels.

use crate::events::{EventRing, InputEvent};
use crate::ipc_client::{FocusedWindow, IpcCommand, IpcMessage, ServiceStatus, TriggerInfo};
use crate::particles::ParticleSystem;
use crate::perf::PerfCounters;
use macroquad::prelude::MouseButton;
use std::sync::mpsc::{Receiver, Sender};

/// Whether the IPC connection to the wayclick daemon is available.
#[derive(Debug, Clone, PartialEq)]
pub enum ConnectionStatus {
    /// Attempting initial or reconnection.
    Connecting,
    /// Connected and subscribed.
    Connected,
    /// Connection lost; retrying in background.
    Disconnected,
}

/// A trigger as displayed in the UI, augmented with live active state.
#[derive(Debug, Clone)]
pub struct TriggerEntry {
    pub info: TriggerInfo,
    /// Reflects the latest `TriggerActivated`/`TriggerDeactivated` event.
    pub live_active: bool,
}

/// Full application state. Created once; mutated each frame via `drain_ipc`.
pub struct AppState {
    pub connection: ConnectionStatus,
    pub service_enabled: bool,
    pub dry_run: bool,
    pub layer: String,
    pub triggers: Vec<TriggerEntry>,
    pub trigger_scroll: usize,
    pub selected_trigger: Option<usize>,
    pub focused_window: Option<FocusedWindow>,

    ipc_rx: Receiver<IpcMessage>,
    ipc_cmd_tx: Sender<IpcCommand>,
}

impl AppState {
    pub fn new(ipc_rx: Receiver<IpcMessage>, ipc_cmd_tx: Sender<IpcCommand>) -> Self {
        Self {
            connection: ConnectionStatus::Connecting,
            service_enabled: false,
            dry_run: false,
            layer: String::from("default"),
            triggers: Vec::new(),
            trigger_scroll: 0,
            selected_trigger: None,
            focused_window: None,
            ipc_rx,
            ipc_cmd_tx,
        }
    }

    /// Drain all pending IPC messages and update state + event ring accordingly.
    /// Must be called from the macroquad main thread each frame.
    pub fn drain_ipc(
        &mut self,
        mx: f32,
        my: f32,
        events: &mut EventRing,
        perf: &mut PerfCounters,
        particles: &mut ParticleSystem,
    ) {
        while let Ok(msg) = self.ipc_rx.try_recv() {
            match msg {
                IpcMessage::Connected {
                    status,
                    triggers,
                    initial_focus,
                } => {
                    self.apply_status(&status);
                    self.apply_triggers(triggers);
                    self.connection = ConnectionStatus::Connected;
                    if let Some(fw) = initial_focus {
                        self.focused_window = Some(fw);
                    }
                    events.push(InputEvent::ServiceEvent(format!(
                        "Connected ({})",
                        if self.dry_run { "dry-run" } else { "live" }
                    )));
                }

                IpcMessage::Disconnected => {
                    self.connection = ConnectionStatus::Disconnected;
                    self.focused_window = None;
                    // Clear active flags — we've lost state
                    for t in &mut self.triggers {
                        t.live_active = false;
                    }
                    events.push(InputEvent::ServiceEvent("Disconnected".to_string()));
                }

                IpcMessage::TriggerActivated(id) => {
                    perf.record_trigger();
                    if let Some(entry) = self.triggers.iter_mut().find(|t| t.info.id == id) {
                        entry.live_active = true;
                        entry.info.activate_count += 1;
                    }
                    events.push(InputEvent::TriggerFired {
                        id: id.clone(),
                        active: true,
                    });
                    particles.spawn_trigger_burst(mx, my);
                }

                IpcMessage::TriggerDeactivated(id) => {
                    if let Some(entry) = self.triggers.iter_mut().find(|t| t.info.id == id) {
                        entry.live_active = false;
                    }
                    events.push(InputEvent::TriggerFired {
                        id,
                        active: false,
                    });
                }

                IpcMessage::RawInput { code, value, .. } => {
                    events.push(InputEvent::RawIpcInput { code, value });
                    // Route perf: mouse buttons (272–279) → record_click, others → record_key
                    // Only count presses (value=1), not releases, to keep totals accurate.
                    if value == 1 {
                        let btn = match code {
                            272 => Some(MouseButton::Left),
                            273 => Some(MouseButton::Right),
                            274 => Some(MouseButton::Middle),
                            275..=279 => Some(MouseButton::Unknown),
                            _ => None,
                        };
                        if let Some(btn) = btn {
                            perf.record_click(btn);
                        } else {
                            perf.record_key();
                        }
                    }
                }

                IpcMessage::LayerChanged { from: _, to } => {
                    self.layer = to.clone();
                    events.push(InputEvent::ServiceEvent(format!("Layer → {}", to)));
                }

                IpcMessage::EnabledChanged(enabled) => {
                    self.service_enabled = enabled;
                    let label = if enabled { "Enabled" } else { "Disabled" };
                    events.push(InputEvent::ServiceEvent(label.to_string()));
                }

                IpcMessage::ConfigReloaded => {
                    events.push(InputEvent::ServiceEvent("Config reloaded".to_string()));
                }

                IpcMessage::TriggerListUpdated(triggers) => {
                    self.apply_triggers(triggers);
                }

                IpcMessage::FocusChanged(window) => {
                    let app_id = window
                        .as_ref()
                        .map(|w| w.app_id.clone())
                        .unwrap_or_default();
                    let title = window
                        .as_ref()
                        .map(|w| w.title.clone())
                        .unwrap_or_default();
                    let process_name = window.as_ref().and_then(|w| w.process_name.clone());
                    let xwayland = window.as_ref().map(|w| w.xwayland).unwrap_or(false);
                    self.focused_window = window;
                    if !app_id.is_empty() {
                        events.push(InputEvent::FocusChanged {
                            app_id,
                            title,
                            process_name,
                            xwayland,
                        });
                    }
                }
            }
        }
    }

    /// Send a FireTrigger command for the given trigger ID.
    #[allow(dead_code)]
    pub fn fire_trigger(&self, id: &str) {
        let _ = self
            .ipc_cmd_tx
            .send(IpcCommand::FireTrigger(id.to_string()));
    }

    /// Toggle a trigger's `user_enabled` state via IPC.
    /// Flips the flag optimistically in local state so the UI updates immediately.
    pub fn toggle_trigger_enabled(&mut self, idx: usize) {
        let Some(entry) = self.triggers.get_mut(idx) else {
            return;
        };
        let id = entry.info.id.clone();
        if entry.info.user_enabled {
            entry.info.user_enabled = false;
            let _ = self.ipc_cmd_tx.send(IpcCommand::DisableTrigger(id));
        } else {
            entry.info.user_enabled = true;
            let _ = self.ipc_cmd_tx.send(IpcCommand::EnableTrigger(id));
        }
    }

    /// Request a fresh trigger list from the daemon.
    #[allow(dead_code)]
    pub fn refresh_triggers(&self) {
        let _ = self.ipc_cmd_tx.send(IpcCommand::RefreshTriggers);
    }

    fn apply_status(&mut self, status: &ServiceStatus) {
        self.service_enabled = status.enabled;
        self.layer = status.layer.clone();
        self.dry_run = status.dry_run;
    }

    fn apply_triggers(&mut self, triggers: Vec<TriggerInfo>) {
        // Preserve live_active state for triggers that already exist
        let prev: std::collections::HashMap<String, bool> = self
            .triggers
            .iter()
            .map(|t| (t.info.id.clone(), t.live_active))
            .collect();

        self.triggers = triggers
            .into_iter()
            .map(|info| {
                let live_active = *prev.get(&info.id).unwrap_or(&false);
                TriggerEntry { info, live_active }
            })
            .collect();

        // Keep selection in bounds
        if let Some(sel) = self.selected_trigger {
            if sel >= self.triggers.len() {
                self.selected_trigger = if self.triggers.is_empty() {
                    None
                } else {
                    Some(self.triggers.len() - 1)
                };
            }
        }
    }
}
