// SPDX-License-Identifier: MIT
use macroquad::prelude::MouseButton;
use std::collections::VecDeque;

const RATE_WINDOW: f32 = 5.0; // seconds for rolling average

pub struct PerfCounters {
    // Click counts per button
    pub left_total: u64,
    pub right_total: u64,
    pub middle_total: u64,
    pub extra_total: u64,
    pub scroll_total: u64,
    pub key_total: u64,
    pub trigger_total: u64,

    // Rolling window timestamps for rate calculation
    click_times: VecDeque<f32>,
    scroll_times: VecDeque<f32>,
    event_times: VecDeque<f32>,

    // Cached rates (updated per tick)
    pub click_rate: f32,
    pub scroll_rate: f32,
    pub event_rate: f32,

    elapsed: f32,

    // Held buttons tracking
    pub held_left: bool,
    pub held_right: bool,
    pub held_middle: bool,
}

impl PerfCounters {
    pub fn new() -> Self {
        Self {
            left_total: 0,
            right_total: 0,
            middle_total: 0,
            extra_total: 0,
            scroll_total: 0,
            key_total: 0,
            trigger_total: 0,
            click_times: VecDeque::new(),
            scroll_times: VecDeque::new(),
            event_times: VecDeque::new(),
            click_rate: 0.0,
            scroll_rate: 0.0,
            event_rate: 0.0,
            elapsed: 0.0,
            held_left: false,
            held_right: false,
            held_middle: false,
        }
    }

    pub fn record_click(&mut self, btn: MouseButton) {
        match btn {
            MouseButton::Left => self.left_total += 1,
            MouseButton::Right => self.right_total += 1,
            MouseButton::Middle => self.middle_total += 1,
            MouseButton::Unknown => self.extra_total += 1,
        }
        self.click_times.push_back(self.elapsed);
        self.event_times.push_back(self.elapsed);
    }

    pub fn record_scroll(&mut self) {
        self.scroll_total += 1;
        self.scroll_times.push_back(self.elapsed);
        self.event_times.push_back(self.elapsed);
    }

    pub fn record_key(&mut self) {
        self.key_total += 1;
        self.event_times.push_back(self.elapsed);
    }

    pub fn record_trigger(&mut self) {
        self.trigger_total += 1;
        self.event_times.push_back(self.elapsed);
    }

    pub fn tick(&mut self, dt: f32) {
        self.elapsed += dt;
        let cutoff = self.elapsed - RATE_WINDOW;

        // Prune old entries
        while self.click_times.front().is_some_and(|&t| t < cutoff) {
            self.click_times.pop_front();
        }
        while self.scroll_times.front().is_some_and(|&t| t < cutoff) {
            self.scroll_times.pop_front();
        }
        while self.event_times.front().is_some_and(|&t| t < cutoff) {
            self.event_times.pop_front();
        }

        // Calculate rates
        let window = RATE_WINDOW.min(self.elapsed);
        if window > 0.0 {
            self.click_rate = self.click_times.len() as f32 / window;
            self.scroll_rate = self.scroll_times.len() as f32 / window;
            self.event_rate = self.event_times.len() as f32 / window;
        }

        // Track held buttons
        self.held_left = macroquad::prelude::is_mouse_button_down(MouseButton::Left);
        self.held_right = macroquad::prelude::is_mouse_button_down(MouseButton::Right);
        self.held_middle = macroquad::prelude::is_mouse_button_down(MouseButton::Middle);
    }

    pub fn total_clicks(&self) -> u64 {
        self.left_total + self.right_total + self.middle_total + self.extra_total
    }
}
