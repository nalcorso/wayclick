#![allow(clippy::too_many_arguments)]

pub mod shaders;

use macroquad::prelude::*;

use crate::app_state::{ConnectionStatus, TriggerEntry};
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
    for (dx, dy) in [(-1.0, 0.0), (1.0, 0.0), (0.0, -1.0), (0.0, 1.0)] {
        draw_text_ex(text, x + dx, y + dy, params(shadow));
    }
    draw_text_ex(text, x, y, params(color));
}

fn truncate_str(s: &str, max_chars: usize) -> String {
    let mut chars = s.chars();
    let collected: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{}…", &collected[..collected.len().saturating_sub(1)])
    } else {
        collected
    }
}

fn trigger_mode_badge(mode: &str) -> &'static str {
    match mode.to_lowercase().as_str() {
        "toggle" => "TOGGLE",
        "hold" => "HOLD",
        "oneshot" | "one_shot" => "ONCE",
        "sequence" => "SEQ",
        _ => "???",
    }
}

// ─── HUD bar (top) ────────────────────────────────────────────────────────

pub fn draw_hud(
    width: f32,
    height: f32,
    perf: &PerfCounters,
    session_secs: f64,
    connection: &ConnectionStatus,
    font: &Font,
    font_bold: &Font,
) {
    draw_rectangle(0.0, 0.0, width, height, colors::HUD_BG);

    let y = height * 0.62;
    let sz: u16 = 15;

    draw_text_ex(
        "⚡ PLAYGROUND",
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
        (
            format!("Clicks/s: {:.1}", perf.click_rate),
            colors::LEFT_CLICK,
        ),
        (format!("Total: {}", perf.total_clicks()), colors::TEXT),
        (
            format!("Events/s: {:.0}", perf.event_rate),
            colors::TEXT_DIM,
        ),
        (format_duration(session_secs), colors::TEXT_DIM),
    ];

    let mut x = 175.0;
    for (text, color) in &items {
        draw_text_outlined(text, x, y, sz as f32, *color, Some(font));
        let w = measure_text(text, Some(font), sz, 1.0).width;
        x += w + 28.0;
    }

    // Connection badge at far right
    let (badge_text, badge_color) = match connection {
        ConnectionStatus::Connected => ("● LIVE", colors::SERVICE_ONLINE),
        ConnectionStatus::Connecting => ("◌ SYNC", colors::MIDDLE_CLICK),
        ConnectionStatus::Disconnected => ("○ OFFLINE", colors::SERVICE_OFFLINE),
    };
    let badge_dims = measure_text(badge_text, Some(font), sz, 1.0);
    draw_text_ex(
        badge_text,
        width - badge_dims.width - 12.0,
        y,
        TextParams {
            font_size: sz,
            font: Some(font),
            color: badge_color,
            ..Default::default()
        },
    );
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

// ─── Service status panel ─────────────────────────────────────────────────

pub fn draw_service_panel(
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    connection: &ConnectionStatus,
    enabled: bool,
    dry_run: bool,
    layer: &str,
    trigger_total: u64,
    font: &Font,
    font_bold: &Font,
) {
    draw_rectangle(x, y, w, h, colors::LOG_BG);
    let pad = 8.0;
    let sz: u16 = 13;

    draw_text_ex(
        "WAYCLICK SERVICE",
        x + pad,
        y + 18.0,
        TextParams {
            font_size: 14,
            font: Some(font_bold),
            color: colors::TITLE,
            ..Default::default()
        },
    );
    draw_line(x + pad, y + 24.0, x + w - pad, y + 24.0, 1.0, colors::GRID);

    let (conn_str, conn_color) = match connection {
        ConnectionStatus::Connected => ("● CONNECTED", colors::SERVICE_ONLINE),
        ConnectionStatus::Connecting => ("◌ CONNECTING", colors::MIDDLE_CLICK),
        ConnectionStatus::Disconnected => ("○ OFFLINE", colors::SERVICE_OFFLINE),
    };
    draw_text_ex(
        conn_str,
        x + pad,
        y + 42.0,
        TextParams {
            font_size: sz,
            font: Some(font),
            color: conn_color,
            ..Default::default()
        },
    );

    if matches!(connection, ConnectionStatus::Connected) {
        let layer_text = format!("↪ {}", truncate_str(layer, 14));
        let ld = measure_text(&layer_text, Some(font), sz - 1, 1.0);
        draw_text_ex(
            &layer_text,
            x + w - ld.width - pad,
            y + 42.0,
            TextParams {
                font_size: sz - 1,
                font: Some(font),
                color: colors::LAYER_BADGE,
                ..Default::default()
            },
        );

        let state_str = if dry_run {
            "DRY-RUN"
        } else if enabled {
            "ENABLED"
        } else {
            "DISABLED"
        };
        let state_color = if dry_run {
            colors::MIDDLE_CLICK
        } else if enabled {
            colors::TRIGGER_ACTIVE
        } else {
            colors::TRIGGER_DISABLED
        };
        draw_text_ex(
            state_str,
            x + pad,
            y + 62.0,
            TextParams {
                font_size: sz - 1,
                font: Some(font),
                color: state_color,
                ..Default::default()
            },
        );

        let act_text = format!("{} activations", trigger_total);
        let ad = measure_text(&act_text, Some(font), sz - 2, 1.0);
        draw_text_ex(
            &act_text,
            x + w - ad.width - pad,
            y + 62.0,
            TextParams {
                font_size: sz - 2,
                font: Some(font),
                color: colors::TEXT_DIM,
                ..Default::default()
            },
        );
    }

    draw_line(x + pad, y + h - 1.0, x + w - pad, y + h - 1.0, 1.0, colors::GRID);
}

// ─── Trigger list panel ────────────────────────────────────────────────────

/// Draw the trigger list. Returns the index of the clicked trigger (if any).
pub fn draw_trigger_list(
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    triggers: &[TriggerEntry],
    scroll: usize,
    selected: Option<usize>,
    mx: f32,
    my: f32,
    font: &Font,
) -> Option<usize> {
    draw_rectangle(x, y, w, h, colors::LOG_BG);

    let pad = 8.0;
    let sz: u16 = 13;
    let row_h = 26.0;

    draw_text_ex(
        "TRIGGERS",
        x + pad,
        y + 18.0,
        TextParams {
            font_size: 14,
            font: Some(font),
            color: colors::TITLE,
            ..Default::default()
        },
    );
    if !triggers.is_empty() {
        let count_text = format!("({})", triggers.len());
        draw_text_ex(
            &count_text,
            x + pad + 72.0,
            y + 18.0,
            TextParams {
                font_size: 12,
                font: Some(font),
                color: colors::TEXT_DIM,
                ..Default::default()
            },
        );
    }
    draw_line(x + pad, y + 24.0, x + w - pad, y + 24.0, 1.0, colors::GRID);

    if triggers.is_empty() {
        draw_text_ex(
            "No triggers loaded",
            x + pad,
            y + 46.0,
            TextParams {
                font_size: sz - 1,
                font: Some(font),
                color: colors::TEXT_DIM,
                ..Default::default()
            },
        );
        draw_line(x + pad, y + h - 1.0, x + w - pad, y + h - 1.0, 1.0, colors::GRID);
        return None;
    }

    let list_y_start = y + 32.0;
    let visible_rows = ((h - 42.0) / row_h).max(0.0) as usize;
    let mut clicked = None;

    for (vis_idx, abs_idx) in (scroll..).enumerate().take(visible_rows) {
        if abs_idx >= triggers.len() {
            break;
        }
        let entry = &triggers[abs_idx];
        let ry = list_y_start + vis_idx as f32 * row_h;
        if ry + row_h > y + h - 10.0 {
            break;
        }

        let hovered = mx >= x && mx < x + w && my >= ry && my < ry + row_h;
        let is_selected = selected == Some(abs_idx);

        if is_selected {
            let mut bg = colors::ACCENT;
            bg.a = 0.12;
            draw_rectangle(x, ry, w, row_h, bg);
        } else if hovered {
            draw_rectangle(x, ry, w, row_h, Color::new(1.0, 1.0, 1.0, 0.05));
        }

        if hovered && is_mouse_button_pressed(MouseButton::Left) {
            clicked = Some(abs_idx);
        }

        let (indicator, ind_color) = if !entry.info.user_enabled {
            ("◎", colors::TRIGGER_DISABLED)
        } else if entry.live_active || entry.info.active {
            ("●", colors::TRIGGER_ACTIVE)
        } else {
            ("○", colors::TRIGGER_IDLE)
        };

        let text_y = ry + row_h * 0.72;
        draw_text_ex(
            indicator,
            x + pad,
            text_y,
            TextParams {
                font_size: sz,
                font: Some(font),
                color: ind_color,
                ..Default::default()
            },
        );

        let name_color = if is_selected || hovered {
            colors::TEXT
        } else {
            Color::new(0.8, 0.83, 0.9, 1.0)
        };
        let name = truncate_str(&entry.info.name, 20);
        draw_text_ex(
            &name,
            x + pad + 16.0,
            text_y,
            TextParams {
                font_size: sz,
                font: Some(font),
                color: name_color,
                ..Default::default()
            },
        );

        let badge = trigger_mode_badge(&entry.info.mode);
        let badge_dims = measure_text(badge, Some(font), sz - 2, 1.0);
        draw_text_ex(
            badge,
            x + w - badge_dims.width - pad,
            text_y,
            TextParams {
                font_size: sz - 2,
                font: Some(font),
                color: colors::TEXT_DIM,
                ..Default::default()
            },
        );
    }

    if triggers.len() > visible_rows {
        let max_shown = scroll + visible_rows;
        let shown = max_shown.min(triggers.len());
        let scroll_text = format!("↕ {}/{}", shown, triggers.len());
        draw_text_ex(
            &scroll_text,
            x + pad,
            y + h - 4.0,
            TextParams {
                font_size: 11,
                font: Some(font),
                color: colors::TEXT_DIM,
                ..Default::default()
            },
        );
    }

    draw_line(x + pad, y + h - 1.0, x + w - pad, y + h - 1.0, 1.0, colors::GRID);
    clicked
}

// ─── Event log (right panel) ──────────────────────────────────────────────

pub fn draw_event_log(x: f32, y: f32, w: f32, h: f32, events: &EventRing, font: &Font) {
    draw_rectangle(x, y, w, h, colors::LOG_BG);

    let sz: u16 = 13;
    let line_h = 18.0;
    let pad = 8.0;

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
    draw_line(x + pad, y + 26.0, x + w - pad, y + 26.0, 1.0, colors::GRID);

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
    connection: &ConnectionStatus,
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

    let totals = format!(
        "L:{} R:{} M:{} X:{} Scroll:{} Keys:{}",
        perf.left_total,
        perf.right_total,
        perf.middle_total,
        perf.extra_total,
        perf.scroll_total,
        perf.key_total
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

    // LIVE / OFFLINE badge at far right
    let (live_text, live_color) = match connection {
        ConnectionStatus::Connected => ("● LIVE", colors::SERVICE_ONLINE),
        ConnectionStatus::Connecting => ("◌ SYNC", colors::MIDDLE_CLICK),
        ConnectionStatus::Disconnected => ("○ OFFLINE", colors::SERVICE_OFFLINE),
    };
    let ld = measure_text(live_text, Some(font), sz, 1.0);
    draw_text_ex(
        live_text,
        width - ld.width - 12.0,
        ty,
        TextParams {
            font_size: sz,
            font: Some(font),
            color: live_color,
            ..Default::default()
        },
    );
}

// ─── Glowing cursor ───────────────────────────────────────────────────────

pub fn draw_cursor(x: f32, y: f32, time: f32) {
    let pulse = (time * 3.0).sin() * 0.15 + 0.85;

    let mut c = colors::ACCENT;
    c.a = 0.06 * pulse;
    draw_circle(x, y, 28.0, c);
    c.a = 0.12 * pulse;
    draw_circle(x, y, 18.0, c);
    c.a = 0.25 * pulse;
    draw_circle(x, y, 10.0, c);

    c.a = 0.7;
    draw_circle(x, y, 4.0, c);
    c.a = 1.0;
    draw_circle(x, y, 2.0, c);

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

pub fn draw_key_labels(particles: &mut ParticleSystem, canvas_w: f32, font: &Font) {
    let canvas_center_x = canvas_w / 2.0;
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
