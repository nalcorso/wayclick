// SPDX-License-Identifier: MIT
use macroquad::prelude::Color;

// Background
pub const BG: Color = Color::new(0.039, 0.055, 0.102, 1.0); // #0a0e1a
pub const GRID: Color = Color::new(0.102, 0.125, 0.251, 1.0); // #1a2040

// Event type colors
pub const LEFT_CLICK: Color = Color::new(0.0, 1.0, 1.0, 1.0); // #00ffff cyan
pub const RIGHT_CLICK: Color = Color::new(1.0, 0.0, 1.0, 1.0); // #ff00ff magenta
pub const MIDDLE_CLICK: Color = Color::new(1.0, 0.843, 0.0, 1.0); // #ffd700 gold
pub const SIDE_CLICK: Color = Color::new(1.0, 0.533, 0.0, 1.0); // #ff8800 orange
pub const SCROLL: Color = Color::new(0.0, 1.0, 0.533, 1.0); // #00ff88 green
pub const KEYBOARD: Color = Color::new(0.878, 0.878, 1.0, 1.0); // #e0e0ff silver
pub const TRAIL: Color = Color::new(0.533, 0.267, 1.0, 0.25); // #8844ff40 purple

// UI
pub const HUD_BG: Color = Color::new(0.02, 0.03, 0.07, 0.85);
pub const LOG_BG: Color = Color::new(0.02, 0.03, 0.07, 0.75);
pub const STATUS_BG: Color = Color::new(0.02, 0.03, 0.07, 0.85);
pub const TEXT: Color = Color::new(0.9, 0.92, 0.96, 1.0);
pub const TEXT_DIM: Color = Color::new(0.45, 0.48, 0.55, 1.0);
pub const ACCENT: Color = Color::new(0.0, 0.8, 1.0, 1.0); // bright cyan
pub const TITLE: Color = Color::new(0.4, 0.85, 1.0, 1.0);

// Service / trigger status
pub const TRIGGER_ACTIVE: Color = Color::new(0.0, 1.0, 0.42, 1.0); // #00ff6b bright green
pub const TRIGGER_IDLE: Color = Color::new(0.35, 0.42, 0.55, 1.0); // dim slate-blue
pub const TRIGGER_DISABLED: Color = Color::new(0.8, 0.25, 0.25, 1.0); // dim red
pub const SERVICE_ONLINE: Color = Color::new(0.0, 1.0, 0.42, 1.0); // same as TRIGGER_ACTIVE
pub const SERVICE_OFFLINE: Color = Color::new(1.0, 0.45, 0.2, 1.0); // orange-red
pub const TRIGGER_FIRE: Color = Color::new(1.0, 0.9, 0.2, 1.0); // gold burst
pub const LAYER_BADGE: Color = Color::new(0.65, 0.3, 1.0, 1.0); // violet

// Event log source indicators
pub const SOURCE_IPC: Color = Color::new(0.0, 0.8, 1.0, 0.35); // dim cyan — normal IPC path
pub const SOURCE_LOCAL: Color = Color::new(1.0, 0.7, 0.2, 0.85); // amber — macroquad fallback

// Focus tracker
pub const FOCUS_CHANGE: Color = Color::new(0.7, 0.5, 1.0, 0.9); // soft lavender
pub const FOCUS_WIDGET_BG: Color = Color::new(0.06, 0.04, 0.12, 0.9); // deep purple tint
