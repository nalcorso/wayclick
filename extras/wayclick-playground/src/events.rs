use macroquad::prelude::*;
use std::collections::VecDeque;

use crate::colors;
use crate::particles::ParticleSystem;
use crate::perf::PerfCounters;

#[derive(Clone, Debug)]
pub enum InputEvent {
    Click(MouseButton),
    Release(MouseButton),
    Scroll {
        x: f32,
        y: f32,
    },
    KeyDown(KeyCode),
    KeyUp(KeyCode),
    #[allow(dead_code)]
    Move {
        x: f32,
        y: f32,
    },
    /// A wayclick trigger was activated (active=true) or deactivated (active=false).
    TriggerFired { id: String, active: bool },
    /// A service-level status message (connection, layer change, enable/disable).
    ServiceEvent(String),
    /// A raw evdev key/button event sourced from the IPC InputReceived event.
    /// `value`: 1 = press, 0 = release.
    RawIpcInput { code: u16, value: i32 },
}

#[derive(Clone, Debug)]
pub struct TimedEvent {
    pub event: InputEvent,
    pub time: f64,
}

pub struct EventRing {
    events: VecDeque<TimedEvent>,
    capacity: usize,
}

impl EventRing {
    pub fn new(capacity: usize) -> Self {
        Self {
            events: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    pub fn push(&mut self, event: InputEvent) {
        if self.events.len() >= self.capacity {
            self.events.pop_front();
        }
        self.events.push_back(TimedEvent {
            event,
            time: get_time(),
        });
    }

    pub fn iter(&self) -> impl Iterator<Item = &TimedEvent> {
        self.events.iter()
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }
}

impl InputEvent {
    pub fn is_local_source(&self) -> bool {
        matches!(
            self,
            InputEvent::Click(_)
                | InputEvent::Release(_)
                | InputEvent::Scroll { .. }
                | InputEvent::KeyDown(_)
                | InputEvent::KeyUp(_)
                | InputEvent::Move { .. }
        )
    }

    pub fn label(&self) -> String {
        match self {
            InputEvent::Click(btn) => format!("{} ↓", btn_name(*btn)),
            InputEvent::Release(btn) => format!("{} ↑", btn_name(*btn)),
            InputEvent::Scroll { x, y } => {
                let dir = if *y > 0.0 {
                    "UP"
                } else if *y < 0.0 {
                    "DOWN"
                } else if *x > 0.0 {
                    "RIGHT"
                } else {
                    "LEFT"
                };
                let mag = y.abs().max(x.abs());
                if mag > 1.0 {
                    format!("SCROLL {} ×{}", dir, mag as i32)
                } else {
                    format!("SCROLL {}", dir)
                }
            }
            InputEvent::KeyDown(k) => format!("{:?} ↓", k),
            InputEvent::KeyUp(k) => format!("{:?} ↑", k),
            InputEvent::Move { x, y } => format!("MOVE ({:.0}, {:.0})", x, y),
            InputEvent::TriggerFired { id, active } => {
                if *active {
                    format!("▶ {}", id)
                } else {
                    format!("■ {}", id)
                }
            }
            InputEvent::ServiceEvent(msg) => msg.clone(),
            InputEvent::RawIpcInput { code, value } => {
                let arrow = if *value == 1 { "↓" } else { "↑" };
                format!("{} {}", evdev_name(*code), arrow)
            }
        }
    }

    pub fn color(&self) -> Color {
        match self {
            InputEvent::Click(btn) | InputEvent::Release(btn) => btn_color(*btn),
            InputEvent::Scroll { .. } => colors::SCROLL,
            InputEvent::KeyDown(_) | InputEvent::KeyUp(_) => colors::KEYBOARD,
            InputEvent::Move { .. } => colors::TRAIL,
            InputEvent::TriggerFired { active, .. } => {
                if *active {
                    colors::TRIGGER_FIRE
                } else {
                    colors::TRIGGER_IDLE
                }
            }
            InputEvent::ServiceEvent(_) => colors::SERVICE_ONLINE,
            InputEvent::RawIpcInput { code, .. } => match code {
                272 => colors::LEFT_CLICK,
                273 => colors::RIGHT_CLICK,
                274 => colors::MIDDLE_CLICK,
                275..=279 => colors::SIDE_CLICK,
                _ => colors::KEYBOARD,
            },
        }
    }
}

pub fn btn_name(btn: MouseButton) -> &'static str {
    match btn {
        MouseButton::Left => "BTN_LEFT",
        MouseButton::Right => "BTN_RIGHT",
        MouseButton::Middle => "BTN_MIDDLE",
        MouseButton::Unknown => "BTN_UNKNOWN",
    }
}

pub fn btn_color(btn: MouseButton) -> Color {
    match btn {
        MouseButton::Left => colors::LEFT_CLICK,
        MouseButton::Right => colors::RIGHT_CLICK,
        MouseButton::Middle => colors::MIDDLE_CLICK,
        MouseButton::Unknown => colors::SIDE_CLICK,
    }
}

/// Map an evdev key/button code to a human-readable name.
/// Covers common keys not represented by macroquad's KeyCode abstraction.
pub fn evdev_name(code: u16) -> &'static str {
    match code {
        // Mouse buttons (BTN_*)
        272 => "BTN_LEFT",
        273 => "BTN_RIGHT",
        274 => "BTN_MIDDLE",
        275 => "BTN_SIDE",
        276 => "BTN_EXTRA",
        277 => "BTN_FORWARD",
        278 => "BTN_BACK",
        279 => "BTN_TASK",

        // Common keyboard keys
        1 => "KEY_ESC",
        14 => "KEY_BACKSPACE",
        15 => "KEY_TAB",
        28 => "KEY_ENTER",
        29 => "KEY_LEFTCTRL",
        42 => "KEY_LEFTSHIFT",
        54 => "KEY_RIGHTSHIFT",
        56 => "KEY_LEFTALT",
        97 => "KEY_RIGHTCTRL",
        100 => "KEY_RIGHTALT",
        125 => "KEY_LEFTMETA",
        126 => "KEY_RIGHTMETA",

        // Navigation
        99 => "KEY_SYSRQ",
        110 => "KEY_INSERT",
        111 => "KEY_DELETE",
        102 => "KEY_HOME",
        107 => "KEY_END",
        104 => "KEY_PAGEUP",
        109 => "KEY_PAGEDOWN",
        103 => "KEY_UP",
        108 => "KEY_DOWN",
        105 => "KEY_LEFT",
        106 => "KEY_RIGHT",
        119 => "KEY_PAUSE",

        // Function extras
        140 => "KEY_CALC",
        142 => "KEY_SLEEP",
        143 => "KEY_WAKEUP",

        // Media / volume
        113 => "KEY_MUTE",
        114 => "KEY_VOLUMEDOWN",
        115 => "KEY_VOLUMEUP",
        128 => "KEY_STOP",
        161 => "KEY_EJECTCD",
        163 => "KEY_NEXTSONG",
        164 => "KEY_PLAYPAUSE",
        165 => "KEY_PREVIOUSSONG",
        166 => "KEY_STOPCD",
        172 => "KEY_HOMEPAGE",
        173 => "KEY_MAIL",
        174 => "KEY_COMPUTER",
        176 => "KEY_NEXTSONG2",
        208 => "KEY_PLAYCD",
        209 => "KEY_PAUSECD",

        // Numpad
        55 => "KEY_KPASTERISK",
        69 => "KEY_NUMLOCK",
        71 => "KEY_KP7",
        72 => "KEY_KP8",
        73 => "KEY_KP9",
        74 => "KEY_KPMINUS",
        75 => "KEY_KP4",
        76 => "KEY_KP5",
        77 => "KEY_KP6",
        78 => "KEY_KPPLUS",
        79 => "KEY_KP1",
        80 => "KEY_KP2",
        81 => "KEY_KP3",
        82 => "KEY_KP0",
        83 => "KEY_KPDOT",
        96 => "KEY_KPENTER",
        98 => "KEY_KPSLASH",

        _ => "KEY_UNKNOWN",
    }
}

/// Poll macroquad input each frame and record events.
/// `skip_keyboard_extra`: when true (IPC connected), skip keyboard keys and
/// Mouse::Unknown — those are sourced from IPC instead.
pub fn poll_input(
    events: &mut EventRing,
    perf: &mut PerfCounters,
    particles: &mut ParticleSystem,
    mx: f32,
    my: f32,
    prev_mouse: &mut (f32, f32),
    skip_keyboard_extra: bool,
) {
    // Always poll the basic three mouse buttons; Unknown only when offline
    let all_buttons = [
        MouseButton::Left,
        MouseButton::Right,
        MouseButton::Middle,
        MouseButton::Unknown,
    ];
    let button_count = if skip_keyboard_extra { 3 } else { 4 };
    for btn in &all_buttons[..button_count] {
        let btn = *btn;
        if is_mouse_button_pressed(btn) {
            // When IPC is connected, it is the authoritative source for button events in
            // the log; only spawn particles and update perf counters from macroquad here.
            if !skip_keyboard_extra {
                events.push(InputEvent::Click(btn));
            }
            perf.record_click(btn);
            particles.spawn_burst(mx, my, btn_color(btn), 35);
        }
        if is_mouse_button_released(btn) && !skip_keyboard_extra {
            events.push(InputEvent::Release(btn));
        }
    }

    let (sx, sy) = mouse_wheel();
    if sx.abs() > 0.01 || sy.abs() > 0.01 {
        events.push(InputEvent::Scroll { x: sx, y: sy });
        perf.record_scroll();
        let magnitude = sy.abs().max(sx.abs()) as usize;
        // Particles follow the dominant scroll axis
        let (main_vx, main_vy) = if sy.abs() >= sx.abs() {
            // Vertical: scroll up (sy>0) → particles go up (vy<0)
            (0.0, if sy > 0.0 { -1.0 } else { 1.0 })
        } else {
            // Horizontal: scroll right (sx>0) → particles go right
            (if sx > 0.0 { 1.0 } else { -1.0 }, 0.0)
        };
        particles.spawn_fountain(mx, my, main_vx, main_vy, magnitude);
    }

    // Keyboard — only when IPC is not providing key events
    if !skip_keyboard_extra {
        for key in get_keys_pressed() {
            events.push(InputEvent::KeyDown(key));
            perf.record_key();
            particles.spawn_key_label(format!("{:?}", key));
        }
        for key in get_keys_released() {
            events.push(InputEvent::KeyUp(key));
        }
    }

    // Movement trail (independent of IPC mode)
    let dx = mx - prev_mouse.0;
    let dy = my - prev_mouse.1;
    let dist = (dx * dx + dy * dy).sqrt();
    if dist > 2.0 {
        particles.spawn_trail(mx, my);
    }
    *prev_mouse = (mx, my);
}
