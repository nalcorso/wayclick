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

        // --- Trigger list scroll (when cursor is in right panel) ---
        let right_panel_w = 340.0_f32;
        if mx >= sw - right_panel_w {
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

        // Layout constants
        let hud_h = 44.0_f32;
        let status_h = 32.0_f32;
        let canvas_w = sw - right_panel_w;
        let canvas_h = sh - hud_h - status_h;

        let service_panel_h = 80.0_f32;
        let trigger_list_h = 220.0_f32;
        let log_h = (canvas_h - service_panel_h - trigger_list_h).max(60.0);

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
        ui::draw_held_keys(canvas_w, canvas_h, hud_h, &font);

        // Floating key labels
        ui::draw_key_labels(&mut particles, canvas_w, &font);

        // --- UI Panels ---
        let panel_x = sw - right_panel_w;

        ui::draw_hud(
            sw,
            hud_h,
            &perf,
            now - start_time,
            &app_state.connection,
            &font,
            &font_bold,
        );

        ui::draw_service_panel(
            panel_x,
            hud_h,
            right_panel_w,
            service_panel_h,
            &app_state.connection,
            app_state.service_enabled,
            app_state.dry_run,
            &app_state.layer,
            perf.trigger_total,
            &font,
            &font_bold,
        );

        let trigger_y = hud_h + service_panel_h;
        if let Some(clicked_idx) = ui::draw_trigger_list(
            panel_x,
            trigger_y,
            right_panel_w,
            trigger_list_h,
            &app_state.triggers,
            app_state.trigger_scroll,
            app_state.selected_trigger,
            mx,
            my,
            &font,
        ) {
            app_state.selected_trigger = Some(clicked_idx);
            if let Some(entry) = app_state.triggers.get(clicked_idx) {
                app_state.fire_trigger(&entry.info.id.clone());
            }
        }

        let log_y = trigger_y + trigger_list_h;
        ui::draw_event_log(panel_x, log_y, right_panel_w, log_h, &events, &font);

        ui::draw_status_bar(
            sw,
            sh,
            status_h,
            mx,
            my,
            &perf,
            &app_state.connection,
            &font,
        );

        // Panel separator lines
        draw_line(0.0, hud_h, sw, hud_h, 1.0, colors::GRID);
        draw_line(panel_x, hud_h, panel_x, sh - status_h, 1.0, colors::GRID);
        draw_line(0.0, sh - status_h, sw, sh - status_h, 1.0, colors::GRID);

        next_frame().await;
    }
}
