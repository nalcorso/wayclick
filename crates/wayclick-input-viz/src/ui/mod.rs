pub mod shaders;

use macroquad::prelude::*;

use crate::colors;
use crate::events::EventRing;
use crate::particles::ParticleSystem;
use crate::perf::PerfCounters;

// ─── Text helpers ──────────────────────────────────────────────────────────

fn draw_text_outlined(text: &str, x: f32, y: f32, size: f32, color: Color, font: Option<&Font>) {
    let shadow = Color::new(0.0, 0.0, 0.0, color.a * 0.7);
    let params = |c: Color| TextParams {
        font_size: size as u16,
        font,
        color: c,
        ..Default::default()
    };
    // Shadow offsets
    for (dx, dy) in [(-1.0, 0.0), (1.0, 0.0), (0.0, -1.0), (0.0, 1.0)] {
        draw_text_ex(text, x + dx, y + dy, params(shadow));
    }
    draw_text_ex(text, x, y, params(color));
}

// ─── HUD bar (top) ────────────────────────────────────────────────────────

pub fn draw_hud(
    width: f32,
    height: f32,
    perf: &PerfCounters,
    session_secs: f64,
    font: &Font,
    font_bold: &Font,
) {
    draw_rectangle(0.0, 0.0, width, height, colors::HUD_BG);

    let y = height * 0.62;
    let sz: u16 = 15;

    // Title
    draw_text_ex(
        "⚡ INPUT VIZ",
        12.0,
        y,
        TextParams {
            font_size: sz,
            font: Some(font_bold),
            color: colors::TITLE,
            ..Default::default()
        },
    );

    let fps = get_fps();
    let fps_color = if fps >= 55 {
        colors::SCROLL
    } else if fps >= 30 {
        colors::MIDDLE_CLICK
    } else {
        colors::RIGHT_CLICK
    };

    let items = [
        (format!("FPS: {fps}"), fps_color),
        (format!("Clicks/s: {:.1}", perf.click_rate), colors::LEFT_CLICK),
        (
            format!("Total: {}", perf.total_clicks()),
            colors::TEXT,
        ),
        (
            format!("Events/s: {:.0}", perf.event_rate),
            colors::TEXT_DIM,
        ),
        (format_duration(session_secs), colors::TEXT_DIM),
    ];

    let mut x = 160.0;
    for (text, color) in &items {
        draw_text_outlined(text, x, y, sz as f32, *color, Some(font));
        let w = measure_text(text, Some(font), sz, 1.0).width;
        x += w + 28.0;
    }
}

fn format_duration(secs: f64) -> String {
    let s = secs as u64;
    let h = s / 3600;
    let m = (s % 3600) / 60;
    let sec = s % 60;
    if h > 0 {
        format!("{h}:{m:02}:{sec:02}")
    } else {
        format!("{m}:{sec:02}")
    }
}

// ─── Event log (right panel) ──────────────────────────────────────────────

pub fn draw_event_log(
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    events: &EventRing,
    font: &Font,
) {
    draw_rectangle(x, y, w, h, colors::LOG_BG);

    let sz: u16 = 13;
    let line_h = 18.0;
    let pad = 8.0;

    // Header
    draw_text_ex(
        "EVENT LOG",
        x + pad,
        y + 20.0,
        TextParams {
            font_size: 14,
            font: Some(font),
            color: colors::TITLE,
            ..Default::default()
        },
    );

    // Separator
    draw_line(x + pad, y + 26.0, x + w - pad, y + 26.0, 1.0, colors::GRID);

    // Events (newest at bottom, render bottom-up to fill visible area)
    let max_lines = ((h - 36.0) / line_h) as usize;
    let start = if events.len() > max_lines {
        events.len() - max_lines
    } else {
        0
    };

    for (i, te) in events.iter().skip(start).enumerate() {
        let ly = y + 42.0 + i as f32 * line_h;
        if ly + line_h > y + h {
            break;
        }

        // Timestamp
        let ts = format!("{:.1}s", te.time);
        draw_text_ex(
            &ts,
            x + pad,
            ly,
            TextParams {
                font_size: sz,
                font: Some(font),
                color: colors::TEXT_DIM,
                ..Default::default()
            },
        );

        // Event label
        let label = te.event.label();
        let color = te.event.color();
        draw_text_ex(
            &label,
            x + pad + 60.0,
            ly,
            TextParams {
                font_size: sz,
                font: Some(font),
                color,
                ..Default::default()
            },
        );
    }
}

// ─── Status bar (bottom) ──────────────────────────────────────────────────

pub fn draw_status_bar(
    width: f32,
    height: f32,
    bar_h: f32,
    mx: f32,
    my: f32,
    perf: &PerfCounters,
    font: &Font,
) {
    let y = height - bar_h;
    draw_rectangle(0.0, y, width, bar_h, colors::STATUS_BG);

    let ty = y + bar_h * 0.65;
    let sz: u16 = 13;

    let pos = format!("Mouse: ({:.0}, {:.0})", mx, my);
    draw_text_ex(
        &pos,
        12.0,
        ty,
        TextParams {
            font_size: sz,
            font: Some(font),
            color: colors::TEXT,
            ..Default::default()
        },
    );

    // Held buttons
    let mut held = String::from("Held: ");
    if perf.held_left {
        held.push_str("L ");
    }
    if perf.held_right {
        held.push_str("R ");
    }
    if perf.held_middle {
        held.push_str("M ");
    }
    if !perf.held_left && !perf.held_right && !perf.held_middle {
        held.push('—');
    }
    draw_text_ex(
        &held,
        200.0,
        ty,
        TextParams {
            font_size: sz,
            font: Some(font),
            color: colors::ACCENT,
            ..Default::default()
        },
    );

    // Totals
    let totals = format!(
        "L:{} R:{} M:{} Scroll:{} Keys:{}",
        perf.left_total, perf.right_total, perf.middle_total,
        perf.scroll_total, perf.key_total
    );
    draw_text_ex(
        &totals,
        380.0,
        ty,
        TextParams {
            font_size: sz,
            font: Some(font),
            color: colors::TEXT_DIM,
            ..Default::default()
        },
    );
}

// ─── Glowing cursor ───────────────────────────────────────────────────────

pub fn draw_cursor(x: f32, y: f32, time: f32) {
    let pulse = (time * 3.0).sin() * 0.15 + 0.85;

    // Outer glow rings
    let mut c = colors::ACCENT;
    c.a = 0.06 * pulse;
    draw_circle(x, y, 28.0, c);
    c.a = 0.12 * pulse;
    draw_circle(x, y, 18.0, c);
    c.a = 0.25 * pulse;
    draw_circle(x, y, 10.0, c);

    // Core
    c.a = 0.7;
    draw_circle(x, y, 4.0, c);
    c.a = 1.0;
    draw_circle(x, y, 2.0, c);

    // Crosshair lines
    c.a = 0.2;
    draw_line(x - 20.0, y, x - 6.0, y, 1.0, c);
    draw_line(x + 6.0, y, x + 20.0, y, 1.0, c);
    draw_line(x, y - 20.0, x, y - 6.0, 1.0, c);
    draw_line(x, y + 6.0, x, y + 20.0, 1.0, c);
}

// ─── Held keys display ────────────────────────────────────────────────────

pub fn draw_held_keys(canvas_w: f32, canvas_h: f32, hud_h: f32, font: &Font) {
    let keys = get_keys_down();
    if keys.is_empty() {
        return;
    }

    let labels: Vec<String> = keys.iter().map(|k| format!("{:?}", k)).collect();
    let text = format!("HELD: [{}]", labels.join(" + "));

    let sz: u16 = 16;
    let dims = measure_text(&text, Some(font), sz, 1.0);
    let x = (canvas_w - dims.width) / 2.0;
    let y = hud_h + canvas_h - 60.0;

    // Background pill
    draw_rectangle(
        x - 12.0,
        y - dims.height - 6.0,
        dims.width + 24.0,
        dims.height + 14.0,
        colors::HUD_BG,
    );

    draw_text_outlined(&text, x, y, sz as f32, colors::KEYBOARD, Some(font));
}

// ─── Floating key labels ──────────────────────────────────────────────────

pub fn draw_key_labels(particles: &mut ParticleSystem, font: &Font) {
    let canvas_center_x = (screen_width() - 300.0) / 2.0;
    let canvas_bottom = screen_height() - 32.0 - 80.0;

    for kl in &particles.key_labels {
        let sz: u16 = 22;
        let dims = measure_text(&kl.text, Some(font), sz, 1.0);
        let x = canvas_center_x - dims.width / 2.0;
        let y = canvas_bottom - kl.y;

        let mut color = colors::KEYBOARD;
        color.a = kl.alpha;
        draw_text_outlined(&kl.text, x, y, sz as f32, color, Some(font));
    }
}
