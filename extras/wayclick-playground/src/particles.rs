use crate::colors;
use macroquad::prelude::*;

const MAX_PARTICLES: usize = 4000;

struct Particle {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    color: Color,
    size: f32,
    size_end: f32,
    lifetime: f32,
    age: f32,
    glow: bool,
}

pub struct KeyLabel {
    pub text: String,
    pub y: f32,
    pub alpha: f32,
    pub lifetime: f32,
    pub age: f32,
}

pub struct ParticleSystem {
    particles: Vec<Particle>,
    pub key_labels: Vec<KeyLabel>,
}

impl ParticleSystem {
    pub fn new() -> Self {
        Self {
            particles: Vec::with_capacity(MAX_PARTICLES),
            key_labels: Vec::new(),
        }
    }

    /// Burst of particles radiating from a point (for clicks).
    pub fn spawn_burst(&mut self, x: f32, y: f32, color: Color, count: usize) {
        for _ in 0..count {
            if self.particles.len() >= MAX_PARTICLES {
                break;
            }
            let angle = rand::gen_range(0.0_f32, std::f32::consts::TAU);
            let speed = rand::gen_range(60.0, 280.0);
            let size = rand::gen_range(2.0, 6.0);
            self.particles.push(Particle {
                x,
                y,
                vx: angle.cos() * speed,
                vy: angle.sin() * speed,
                color,
                size,
                size_end: 0.5,
                lifetime: rand::gen_range(0.3, 0.8),
                age: 0.0,
                glow: true,
            });
        }
    }

    /// Larger burst for trigger activation — wider spread, longer lifetime, gold colour.
    pub fn spawn_trigger_burst(&mut self, x: f32, y: f32) {
        let color = colors::TRIGGER_FIRE;
        let count = 50;
        for _ in 0..count {
            if self.particles.len() >= MAX_PARTICLES {
                break;
            }
            let angle = rand::gen_range(0.0_f32, std::f32::consts::TAU);
            let speed = rand::gen_range(80.0, 420.0);
            let size = rand::gen_range(3.0, 9.0);
            self.particles.push(Particle {
                x,
                y,
                vx: angle.cos() * speed,
                vy: angle.sin() * speed,
                color,
                size,
                size_end: 0.5,
                lifetime: rand::gen_range(0.5, 1.2),
                age: 0.0,
                glow: true,
            });
        }
    }

    /// Small trail particle at cursor position.
    pub fn spawn_trail(&mut self, x: f32, y: f32) {
        if self.particles.len() >= MAX_PARTICLES {
            return;
        }
        self.particles.push(Particle {
            x: x + rand::gen_range(-2.0, 2.0),
            y: y + rand::gen_range(-2.0, 2.0),
            vx: rand::gen_range(-10.0, 10.0),
            vy: rand::gen_range(-10.0, 10.0),
            color: colors::TRAIL,
            size: rand::gen_range(1.5, 3.5),
            size_end: 0.2,
            lifetime: rand::gen_range(0.2, 0.5),
            age: 0.0,
            glow: false,
        });
    }

    /// Directional fountain for scroll events.
    /// `main_vx` / `main_vy` define the primary direction (normalised unit vector).
    /// Spread is applied perpendicular to the direction.
    pub fn spawn_fountain(&mut self, x: f32, y: f32, main_vx: f32, main_vy: f32, magnitude: usize) {
        let count = 8 * magnitude.max(1);
        // Perpendicular direction (rotate 90°)
        let perp_vx = -main_vy;
        let perp_vy = main_vx;
        for _ in 0..count {
            if self.particles.len() >= MAX_PARTICLES {
                break;
            }
            let speed = rand::gen_range(100.0, 250.0);
            let spread = rand::gen_range(-40.0, 40.0);
            self.particles.push(Particle {
                x: x + rand::gen_range(-6.0, 6.0),
                y: y + rand::gen_range(-6.0, 6.0),
                vx: main_vx * speed + perp_vx * spread,
                vy: main_vy * speed + perp_vy * spread,
                color: colors::SCROLL,
                size: rand::gen_range(2.0, 5.0),
                size_end: 0.5,
                lifetime: rand::gen_range(0.3, 0.6),
                age: 0.0,
                glow: true,
            });
        }
    }

    /// Floating key label.
    pub fn spawn_key_label(&mut self, text: String) {
        self.key_labels.push(KeyLabel {
            text,
            y: 0.0,
            alpha: 1.0,
            lifetime: 1.2,
            age: 0.0,
        });
    }

    pub fn update(&mut self, dt: f32) {
        // Update particles
        for p in &mut self.particles {
            p.age += dt;
            p.x += p.vx * dt;
            p.y += p.vy * dt;
            // Friction / drag
            p.vx *= 1.0 - 2.5 * dt;
            p.vy *= 1.0 - 2.5 * dt;
        }
        self.particles.retain(|p| p.age < p.lifetime);

        // Update key labels
        for kl in &mut self.key_labels {
            kl.age += dt;
            kl.y += 50.0 * dt; // float upward
            kl.alpha = 1.0 - (kl.age / kl.lifetime).min(1.0);
        }
        self.key_labels.retain(|kl| kl.age < kl.lifetime);
    }

    pub fn draw(&self) {
        for p in &self.particles {
            let t = (p.age / p.lifetime).min(1.0);
            let alpha = (1.0 - t) * p.color.a;
            let size = p.size + (p.size_end - p.size) * t;
            let mut c = p.color;
            c.a = alpha;

            if p.glow {
                // Outer glow (larger, very transparent)
                let mut gc = c;
                gc.a = alpha * 0.15;
                draw_circle(p.x, p.y, size * 3.0, gc);
                gc.a = alpha * 0.35;
                draw_circle(p.x, p.y, size * 1.8, gc);
            }
            // Core
            draw_circle(p.x, p.y, size, c);
        }
    }
}
