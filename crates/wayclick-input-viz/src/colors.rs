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
