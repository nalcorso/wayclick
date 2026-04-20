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
        }
    }

    pub fn color(&self) -> Color {
        match self {
            InputEvent::Click(btn) | InputEvent::Release(btn) => btn_color(*btn),
            InputEvent::Scroll { .. } => colors::SCROLL,
            InputEvent::KeyDown(_) | InputEvent::KeyUp(_) => colors::KEYBOARD,
            InputEvent::Move { .. } => colors::TRAIL,
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

/// Poll all macroquad input each frame and record events.
pub fn poll_input(
    events: &mut EventRing,
    perf: &mut PerfCounters,
    particles: &mut ParticleSystem,
    mx: f32,
    my: f32,
    prev_mouse: &mut (f32, f32),
) {
    let buttons = [
        MouseButton::Left,
        MouseButton::Right,
        MouseButton::Middle,
        MouseButton::Unknown, // side/extra/forward/back (all map to Unknown in macroquad)
    ];
    for btn in buttons {
        if is_mouse_button_pressed(btn) {
            events.push(InputEvent::Click(btn));
            perf.record_click(btn);
            particles.spawn_burst(mx, my, btn_color(btn), 35);
        }
        if is_mouse_button_released(btn) {
            events.push(InputEvent::Release(btn));
        }
    }

    let (sx, sy) = mouse_wheel();
    if sx.abs() > 0.01 || sy.abs() > 0.01 {
        events.push(InputEvent::Scroll { x: sx, y: sy });
        perf.record_scroll();
        let dir = if sy > 0.0 { -1.0 } else { 1.0 };
        particles.spawn_fountain(mx, my, dir, sy.abs().max(sx.abs()) as usize);
    }

    // Keyboard
    for key in get_keys_pressed() {
        events.push(InputEvent::KeyDown(key));
        perf.record_key();
        particles.spawn_key_label(format!("{:?}", key));
    }
    for key in get_keys_released() {
        events.push(InputEvent::KeyUp(key));
    }

    // Movement trail (don't log every frame, only when moved)
    let dx = mx - prev_mouse.0;
    let dy = my - prev_mouse.1;
    let dist = (dx * dx + dy * dy).sqrt();
    if dist > 2.0 {
        particles.spawn_trail(mx, my);
    }
    *prev_mouse = (mx, my);
}
