#![allow(clippy::too_many_arguments)]

pub mod shaders;

use macroquad::prelude::*;

use crate::app_state::{ConnectionStatus, TriggerEntry};
use crate::colors;
use crate::events::EventRing;
use crate::ipc_client::FocusedWindow;
use crate::particles::ParticleSystem;
use crate::perf::PerfCounters;

// ─── Layout ───────────────────────────────────────────────────────────────

pub const SECTION_HEADER_H: f32 = 30.0;
pub const FOCUS_CONTENT_H: f32 = 68.0;
pub const TRIGGER_ROW_H: f32 = 26.0;
pub const TRIGGER_MAX_ROWS: usize = 10;
pub const TRIGGER_PAD: f32 = 14.0;
pub const LOG_MIN_H: f32 = 80.0;

/// Pre-computed sidebar section heights and Y positions for one frame.
pub struct SidebarLayout {
    pub panel_x: f32,
    pub panel_w: f32,
    pub focus_y: f32,
    pub focus_h: f32,
    pub triggers_y: f32,
    pub triggers_h: f32,
    pub log_y: f32,
    pub log_h: f32,
}

impl SidebarLayout {
    pub fn compute(
        sw: f32,
        sh: f32,
        hud_h: f32,
        status_h: f32,
        panel_w: f32,
        focus_expanded: bool,
        triggers_expanded: bool,
        log_expanded: bool,
        trigger_count: usize,
    ) -> Self {
        let panel_x = sw - panel_w;
        let available = sh - hud_h - status_h;

        let focus_h = if focus_expanded {
            SECTION_HEADER_H + FOCUS_CONTENT_H
        } else {
            SECTION_HEADER_H
        };

        let triggers_h = if triggers_expanded {
            let rows = trigger_count.min(TRIGGER_MAX_ROWS);
            let content = (rows as f32 * TRIGGER_ROW_H + TRIGGER_PAD).max(TRIGGER_ROW_H);
            SECTION_HEADER_H + content
        } else {
            SECTION_HEADER_H
        };

        let log_h = if log_expanded {
            (available - focus_h - triggers_h).max(LOG_MIN_H)
        } else {
            SECTION_HEADER_H
        };

        let focus_y = hud_h;
        let triggers_y = focus_y + focus_h;
        let log_y = triggers_y + triggers_h;

        SidebarLayout {
            panel_x,
            panel_w,
            focus_y,
            focus_h,
            triggers_y,
            triggers_h,
            log_y,
            log_h,
        }
    }
}

// ─── Section header helper ────────────────────────────────────────────────

/// Draw a clickable section header bar. Returns true if clicked this frame.
fn draw_section_header(
    x: f32,
    y: f32,
    w: f32,
    title: &str,
    badge: Option<&str>,
    expanded: bool,
    mx: f32,
    my: f32,
    font: &Font,
) -> bool {
    let h = SECTION_HEADER_H;
    let hovered = mx >= x && mx < x + w && my >= y && my < y + h;

    if hovered {
        draw_rectangle(x, y, w, h, Color::new(1.0, 1.0, 1.0, 0.04));
    }

    let chevron = if expanded { "▼" } else { "▶" };
    let ty = y + h * 0.68;
    let sz: u16 = 13;

    draw_text_ex(
        chevron,
        x + 8.0,
        ty,
        TextParams {
            font_size: sz,
            font: Some(font),
            color: colors::TEXT_DIM,
            ..Default::default()
        },
    );

    draw_text_ex(
        title,
        x + 24.0,
        ty,
        TextParams {
            font_size: 13,
            font: Some(font),
            color: colors::TITLE,
            ..Default::default()
        },
    );

    if let Some(badge_text) = badge {
        let title_w = measure_text(title, Some(font), sz, 1.0).width;
        draw_text_ex(
            badge_text,
            x + 24.0 + title_w + 6.0,
            ty,
            TextParams {
                font_size: 12,
                font: Some(font),
                color: colors::TEXT_DIM,
                ..Default::default()
            },
        );
    }

    draw_line(x, y + h, x + w, y + h, 1.0, colors::GRID);

    hovered && is_mouse_button_pressed(MouseButton::Left)
}

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
    service_enabled: bool,
    dry_run: bool,
    layer: &str,
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

    // Connection badge + layer/state at far right
    let (badge_text, badge_color) = match connection {
        ConnectionStatus::Connected => ("● LIVE", colors::SERVICE_ONLINE),
        ConnectionStatus::Connecting => ("◌ SYNC", colors::MIDDLE_CLICK),
        ConnectionStatus::Disconnected => ("○ OFFLINE", colors::SERVICE_OFFLINE),
    };
    let badge_dims = measure_text(badge_text, Some(font), sz, 1.0);

    // When connected, compose layer + state annotations (rendered separately for distinct colors)
    let (layer_ann, state_ann) = if matches!(connection, ConnectionStatus::Connected) {
        let layer_part = if !layer.is_empty() {
            format!("  ↪ {}  ", truncate_str(layer, 12))
        } else {
            "  ".to_string()
        };
        let state_part = if dry_run {
            "DRY-RUN"
        } else if service_enabled {
            "ENABLED"
        } else {
            "DISABLED"
        };
        (layer_part, state_part.to_string())
    } else {
        (String::new(), String::new())
    };
    let layer_dims = measure_text(&layer_ann, Some(font), sz - 2, 1.0);
    let state_dims = measure_text(&state_ann, Some(font), sz - 2, 1.0);
    let ann_total_w = layer_dims.width + state_dims.width;

    let state_color = if dry_run {
        colors::MIDDLE_CLICK
    } else if service_enabled {
        colors::TRIGGER_ACTIVE
    } else {
        colors::TRIGGER_DISABLED
    };

    let badge_x = width - badge_dims.width - ann_total_w - 12.0;
    draw_text_ex(
        badge_text,
        badge_x,
        y,
        TextParams {
            font_size: sz,
            font: Some(font),
            color: badge_color,
            ..Default::default()
        },
    );
    if !layer_ann.is_empty() {
        draw_text_ex(
            &layer_ann,
            badge_x + badge_dims.width,
            y,
            TextParams {
                font_size: sz - 2,
                font: Some(font),
                color: colors::LAYER_BADGE,
                ..Default::default()
            },
        );
    }
    if !state_ann.is_empty() {
        draw_text_ex(
            &state_ann,
            badge_x + badge_dims.width + layer_dims.width,
            y,
            TextParams {
                font_size: sz - 2,
                font: Some(font),
                color: state_color,
                ..Default::default()
            },
        );
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

// ─── Trigger list panel ────────────────────────────────────────────────────

/// Draw the trigger list panel.
/// Returns `(clicked_item_index, header_was_clicked)`.
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
    expanded: bool,
    font: &Font,
) -> (Option<usize>, bool) {
    draw_rectangle(x, y, w, h, colors::LOG_BG);

    let badge = if triggers.is_empty() {
        None
    } else {
        Some(format!("({})", triggers.len()))
    };
    let header_clicked = draw_section_header(
        x,
        y,
        w,
        "TRIGGERS",
        badge.as_deref(),
        expanded,
        mx,
        my,
        font,
    );

    if !expanded {
        return (None, header_clicked);
    }

    let pad = 8.0;
    let sz: u16 = 13;
    let row_h = TRIGGER_ROW_H;
    let content_y = y + SECTION_HEADER_H;

    if triggers.is_empty() {
        draw_text_ex(
            "No triggers loaded",
            x + pad,
            content_y + 20.0,
            TextParams {
                font_size: sz - 1,
                font: Some(font),
                color: colors::TEXT_DIM,
                ..Default::default()
            },
        );
        return (None, header_clicked);
    }

    let list_y_start = content_y + 4.0;
    let visible_rows = ((h - SECTION_HEADER_H - 12.0) / row_h).max(0.0) as usize;
    let mut clicked = None;

    for (vis_idx, abs_idx) in (scroll..).enumerate().take(visible_rows) {
        if abs_idx >= triggers.len() {
            break;
        }
        let entry = &triggers[abs_idx];
        let ry = list_y_start + vis_idx as f32 * row_h;
        if ry + row_h > y + h - 4.0 {
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

    if triggers.len() > visible_rows + scroll {
        let shown = (scroll + visible_rows).min(triggers.len());
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

    (clicked, header_clicked)
}

// ─── Focused window widget ─────────────────────────────────────────────────

/// Draw the focused window panel.
/// Returns true if the section header was clicked (toggle expand/collapse).
pub fn draw_focus_widget(
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    window: Option<&FocusedWindow>,
    expanded: bool,
    mx: f32,
    my: f32,
    font: &Font,
) -> bool {
    draw_rectangle(x, y, w, h, colors::FOCUS_WIDGET_BG);

    let header_clicked = draw_section_header(
        x,
        y,
        w,
        "FOCUSED WINDOW",
        None,
        expanded,
        mx,
        my,
        font,
    );

    if !expanded {
        return header_clicked;
    }

    let sz: u16 = 13;
    let content_y = y + SECTION_HEADER_H;

    match window {
        None => {
            draw_text_ex(
                "⊙  —",
                x + 8.0,
                content_y + 18.0,
                TextParams {
                    font_size: sz,
                    font: Some(font),
                    color: colors::TEXT_DIM,
                    ..Default::default()
                },
            );
        }
        Some(fw) => {
            let name = fw
                .process_name
                .as_deref()
                .filter(|s| !s.is_empty())
                .unwrap_or(fw.app_id.as_str());
            let xw_tag = if fw.xwayland { " [XWayland]" } else { "" };
            let app_text = format!("⊙  {}{}", truncate_str(name, 22), xw_tag);
            draw_text_ex(
                &app_text,
                x + 8.0,
                content_y + 16.0,
                TextParams {
                    font_size: sz,
                    font: Some(font),
                    color: colors::FOCUS_CHANGE,
                    ..Default::default()
                },
            );

            if !fw.title.is_empty() {
                let title_text = truncate_str(&fw.title, 34);
                draw_text_ex(
                    &title_text,
                    x + 8.0,
                    content_y + 34.0,
                    TextParams {
                        font_size: sz - 1,
                        font: Some(font),
                        color: colors::TEXT_DIM,
                        ..Default::default()
                    },
                );
            }
        }
    }
    header_clicked
}

// ─── Event log (right panel) ──────────────────────────────────────────────

/// Draw the event log panel.
/// Returns true if the section header was clicked (toggle expand/collapse).
pub fn draw_event_log(
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    events: &EventRing,
    expanded: bool,
    mx: f32,
    my: f32,
    font: &Font,
) -> bool {
    draw_rectangle(x, y, w, h, colors::LOG_BG);

    let header_clicked =
        draw_section_header(x, y, w, "EVENT LOG", None, expanded, mx, my, font);

    if !expanded {
        return header_clicked;
    }

    let sz: u16 = 13;
    let line_h = 18.0;
    let pad = 8.0;
    // content_top is the top edge of the first row (below the header).
    // Text baseline is placed at 78% down each row so glyphs stay within
    // the row and never bleed up into the header area.
    let content_top = y + SECTION_HEADER_H + 4.0;
    let baseline_offset = line_h * 0.78;

    let max_lines = ((h - SECTION_HEADER_H - 4.0) / line_h).max(0.0) as usize;
    let start = events.len().saturating_sub(max_lines);

    for (i, te) in events.iter().skip(start).enumerate() {
        let row_top = content_top + i as f32 * line_h;
        if row_top + line_h > y + h {
            break;
        }
        let ly = row_top + baseline_offset;

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

        let (src_glyph, src_color) = if te.event.is_local_source() {
            ("○", colors::SOURCE_LOCAL)
        } else {
            ("●", colors::SOURCE_IPC)
        };
        draw_text_ex(
            src_glyph,
            x + pad + 50.0,
            ly,
            TextParams {
                font_size: sz,
                font: Some(font),
                color: src_color,
                ..Default::default()
            },
        );

        let label = te.event.label();
        let color = te.event.color();
        draw_text_ex(
            &label,
            x + pad + 64.0,
            ly,
            TextParams {
                font_size: sz,
                font: Some(font),
                color,
                ..Default::default()
            },
        );
    }

    header_clicked
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
    service_enabled: bool,
    dry_run: bool,
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

    // Operational state at far right (only when connected)
    if matches!(connection, ConnectionStatus::Connected) {
        let (state_text, state_color) = if dry_run {
            ("DRY-RUN", colors::MIDDLE_CLICK)
        } else if service_enabled {
            ("ENABLED", colors::TRIGGER_ACTIVE)
        } else {
            ("DISABLED", colors::TRIGGER_DISABLED)
        };
        let sd = measure_text(state_text, Some(font), sz, 1.0);
        draw_text_ex(
            state_text,
            width - sd.width - 12.0,
            ty,
            TextParams {
                font_size: sz,
                font: Some(font),
                color: state_color,
                ..Default::default()
            },
        );
    }
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
