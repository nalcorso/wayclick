use macroquad::prelude::*;

mod colors;
mod events;
mod particles;
mod perf;
mod ui;

use events::EventRing;
use particles::ParticleSystem;
use perf::PerfCounters;

fn window_conf() -> Conf {
    Conf {
        window_title: "wayclick input-viz".to_string(),
        window_width: 1280,
        window_height: 800,
        window_resizable: true,
        ..Default::default()
    }
}

#[macroquad::main(window_conf)]
async fn main() {
    let font = load_ttf_font_from_bytes(include_bytes!(
        "../assets/fonts/JetBrainsMono-Regular.ttf"
    ))
    .expect("failed to load font");
    let font_bold = load_ttf_font_from_bytes(include_bytes!(
        "../assets/fonts/JetBrainsMono-Bold.ttf"
    ))
    .expect("failed to load bold font");

    let bg_material = ui::shaders::create_bg_material();
    let bloom_material = ui::shaders::create_bloom_material();

    let mut events = EventRing::new(200);
    let mut perf = PerfCounters::new();
    let mut particles = ParticleSystem::new();
    let mut prev_mouse = mouse_position();
    let start_time = get_time();

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
        let (tw, th) = (bloom_target.texture.width() as u32, bloom_target.texture.height() as u32);
        if tw != sw as u32 || th != sh as u32 {
            bloom_target = render_target(sw as u32, sh as u32);
            bloom_target.texture.set_filter(FilterMode::Linear);
        }

        // --- Input polling ---
        events::poll_input(&mut events, &mut perf, &mut particles, mx, my, &mut prev_mouse);
        perf.tick(dt);

        // --- Particle update ---
        particles.update(dt);

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

        // Canvas area (left of log panel)
        let log_width = 300.0_f32;
        let hud_h = 44.0_f32;
        let status_h = 32.0_f32;
        let canvas_w = sw - log_width;
        let canvas_h = sh - hud_h - status_h;

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
        // Also draw raw particles for core brightness
        particles.draw();

        // Glowing cursor
        ui::draw_cursor(mx, my, now as f32);

        // Held keys display
        ui::draw_held_keys(canvas_w, canvas_h, hud_h, &font);

        // Floating key labels
        ui::draw_key_labels(&mut particles, &font);

        // --- UI Panels ---
        ui::draw_hud(sw, hud_h, &perf, now - start_time, &font, &font_bold);
        ui::draw_event_log(
            sw - log_width, hud_h, log_width, canvas_h,
            &events, &font,
        );
        ui::draw_status_bar(
            sw, sh, status_h, mx, my, &perf, &font,
        );

        // Panel separator lines
        draw_line(0.0, hud_h, sw, hud_h, 1.0, colors::GRID);
        draw_line(sw - log_width, hud_h, sw - log_width, sh - status_h, 1.0, colors::GRID);
        draw_line(0.0, sh - status_h, sw, sh - status_h, 1.0, colors::GRID);

        next_frame().await;
    }
}
