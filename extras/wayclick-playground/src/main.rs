// SPDX-License-Identifier: MIT
use macroquad::prelude::*;

mod app_state;
mod colors;
mod events;
mod ipc_client;
mod particles;
mod perf;
mod ui;

use app_state::{AppState, ConnectionStatus};
use events::EventRing;
use ipc_client::spawn_ipc_thread;
use particles::ParticleSystem;
use perf::PerfCounters;
use ui::SidebarLayout;

fn window_conf() -> Conf {
    Conf {
        window_title: "wayclick playground".to_string(),
        window_width: 1280,
        window_height: 800,
        window_resizable: true,
        ..Default::default()
    }
}

#[macroquad::main(window_conf)]
async fn main() {
    let font =
        load_ttf_font_from_bytes(include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf"))
            .expect("failed to load font");
    let font_bold =
        load_ttf_font_from_bytes(include_bytes!("../assets/fonts/JetBrainsMono-Bold.ttf"))
            .expect("failed to load bold font");

    let bg_material = ui::shaders::create_bg_material();
    let bloom_material = ui::shaders::create_bloom_material();

    let mut events = EventRing::new(200);
    let mut perf = PerfCounters::new();
    let mut particles = ParticleSystem::new();
    let mut prev_mouse = mouse_position();
    let start_time = get_time();

    // Spawn IPC background thread and initialise application state
    let (ipc_rx, ipc_cmd_tx) = spawn_ipc_thread();
    let mut app_state = AppState::new(ipc_rx, ipc_cmd_tx);

    // Render target for bloom pass
    let mut bloom_target = render_target(screen_width() as u32, screen_height() as u32);
    bloom_target.texture.set_filter(FilterMode::Linear);

    const RIGHT_PANEL_W: f32 = 340.0;
    const HUD_H: f32 = 44.0;
    const STATUS_H: f32 = 32.0;

    loop {
        let dt = get_frame_time();
        let now = get_time();
        let (mx, my) = mouse_position();
        let sw = screen_width();
        let sh = screen_height();

        // Resize bloom target if window size changed
        let (tw, th) = (
            bloom_target.texture.width() as u32,
            bloom_target.texture.height() as u32,
        );
        if tw != sw as u32 || th != sh as u32 {
            bloom_target = render_target(sw as u32, sh as u32);
            bloom_target.texture.set_filter(FilterMode::Linear);
        }

        // --- IPC drain (must happen before input polling) ---
        app_state.drain_ipc(mx, my, &mut events, &mut perf, &mut particles);

        let ipc_connected = matches!(app_state.connection, ConnectionStatus::Connected);

        // --- Input polling ---
        // When IPC is connected, skip macroquad keyboard + Mouse::Unknown;
        // those events arrive via InputReceived IPC events instead.
        events::poll_input(
            &mut events,
            &mut perf,
            &mut particles,
            mx,
            my,
            &mut prev_mouse,
            ipc_connected,
        );
        perf.tick(dt);

        // --- Compute sidebar layout ---
        let layout = SidebarLayout::compute(
            sw,
            sh,
            HUD_H,
            STATUS_H,
            RIGHT_PANEL_W,
            app_state.focus_expanded,
            app_state.triggers_expanded,
            app_state.log_expanded,
            app_state.triggers.len(),
        );

        // --- Trigger list scroll (when cursor is in triggers section) ---
        if app_state.triggers_expanded
            && mx >= layout.panel_x
            && my >= layout.triggers_y
            && my < layout.triggers_y + layout.triggers_h
        {
            let (_, sy) = mouse_wheel();
            if sy > 0.5 && app_state.trigger_scroll > 0 {
                app_state.trigger_scroll -= 1;
            } else if sy < -0.5 {
                let max_scroll = app_state.triggers.len().saturating_sub(1);
                if app_state.trigger_scroll < max_scroll {
                    app_state.trigger_scroll += 1;
                }
            }
        }

        // --- Particle update ---
        particles.update(dt);

        let canvas_w = sw - RIGHT_PANEL_W;
        let canvas_h = sh - HUD_H - STATUS_H;

        // ============================================================
        // Pass 1: Render particles to off-screen target for bloom
        // ============================================================
        set_camera(&Camera2D {
            render_target: Some(bloom_target.clone()),
            zoom: vec2(2.0 / sw, 2.0 / sh),
            target: vec2(sw / 2.0, sh / 2.0),
            ..Default::default()
        });
        clear_background(Color::new(0.0, 0.0, 0.0, 0.0));
        particles.draw();
        set_default_camera();

        // ============================================================
        // Pass 2: Main render
        // ============================================================
        clear_background(colors::BG);

        // Animated background grid
        if let Some(ref mat) = bg_material {
            gl_use_material(mat);
            mat.set_uniform("u_time", now as f32);
            mat.set_uniform("u_resolution", vec2(sw, sh));
            draw_rectangle(0.0, 0.0, sw, sh, WHITE);
            gl_use_default_material();
        }

        // Draw particles (with bloom composite)
        if let Some(ref mat) = bloom_material {
            gl_use_material(mat);
            draw_texture_ex(
                &bloom_target.texture,
                0.0,
                0.0,
                WHITE,
                DrawTextureParams {
                    dest_size: Some(vec2(sw, sh)),
                    ..Default::default()
                },
            );
            gl_use_default_material();
        }
        particles.draw();

        // Glowing cursor
        ui::draw_cursor(mx, my, now as f32);

        // Held keys display
        ui::draw_held_keys(canvas_w, canvas_h, HUD_H, &font);

        // Floating key labels
        ui::draw_key_labels(&mut particles, canvas_w, &font);

        // --- UI Panels ---
        ui::draw_hud(
            sw,
            HUD_H,
            &perf,
            now - start_time,
            &app_state.connection,
            app_state.service_enabled,
            app_state.dry_run,
            &app_state.layer,
            &font,
            &font_bold,
        );

        // Focused window (top of sidebar)
        if ui::draw_focus_widget(
            layout.panel_x,
            layout.focus_y,
            layout.panel_w,
            layout.focus_h,
            app_state.focused_window.as_ref(),
            app_state.focus_expanded,
            mx,
            my,
            &font,
        ) {
            app_state.focus_expanded = !app_state.focus_expanded;
        }

        // Triggers list
        let (trigger_click, trigger_header_click) = ui::draw_trigger_list(
            layout.panel_x,
            layout.triggers_y,
            layout.panel_w,
            layout.triggers_h,
            &app_state.triggers,
            app_state.trigger_scroll,
            app_state.selected_trigger,
            mx,
            my,
            app_state.triggers_expanded,
            &font,
        );
        if trigger_header_click {
            app_state.triggers_expanded = !app_state.triggers_expanded;
            app_state.trigger_scroll = 0;
        }
        if let Some(idx) = trigger_click {
            app_state.selected_trigger = Some(idx);
            app_state.toggle_trigger_enabled(idx);
        }

        // Event log (fills remaining space)
        if ui::draw_event_log(
            layout.panel_x,
            layout.log_y,
            layout.panel_w,
            layout.log_h,
            &events,
            app_state.log_expanded,
            mx,
            my,
            &font,
        ) {
            app_state.log_expanded = !app_state.log_expanded;
        }

        ui::draw_status_bar(
            sw,
            sh,
            STATUS_H,
            mx,
            my,
            &perf,
            &app_state.connection,
            app_state.service_enabled,
            app_state.dry_run,
            &font,
        );

        // Panel separator lines
        draw_line(0.0, HUD_H, sw, HUD_H, 1.0, colors::GRID);
        draw_line(
            layout.panel_x,
            HUD_H,
            layout.panel_x,
            sh - STATUS_H,
            1.0,
            colors::GRID,
        );
        draw_line(0.0, sh - STATUS_H, sw, sh - STATUS_H, 1.0, colors::GRID);

        next_frame().await;
    }
}
