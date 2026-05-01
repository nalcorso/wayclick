use macroquad::prelude::*;

/// Animated background grid shader — subtle pulsing grid lines with
/// a radial vignette and breathing effect.
pub fn create_bg_material() -> Option<Material> {
    let fragment = r#"#version 100
precision mediump float;

varying vec2 uv;

uniform float u_time;
uniform vec2 u_resolution;

void main() {
    vec2 pos = uv * u_resolution;

    // Base dark navy
    vec3 bg = vec3(0.039, 0.055, 0.102);

    // Grid lines
    float grid_size = 48.0;
    vec2 grid = abs(fract(pos / grid_size) - 0.5);
    float line = min(grid.x, grid.y);
    float grid_mask = 1.0 - smoothstep(0.0, 0.03, line);

    // Breathing pulse
    float pulse = sin(u_time * 0.8) * 0.3 + 0.7;

    // Grid color (dim blue-purple)
    vec3 grid_color = vec3(0.10, 0.12, 0.25) * pulse;
    bg = mix(bg, grid_color, grid_mask * 0.5);

    // Subtle radial vignette
    vec2 center = (uv - 0.5) * 2.0;
    float vignette = 1.0 - dot(center, center) * 0.3;
    bg *= vignette;

    // Very subtle scan line effect
    float scan = sin(pos.y * 1.5 + u_time * 2.0) * 0.008 + 1.0;
    bg *= scan;

    gl_FragColor = vec4(bg, 1.0);
}
"#;

    let vertex = DEFAULT_VERTEX_SHADER;

    match load_material(
        ShaderSource::Glsl { vertex, fragment },
        MaterialParams {
            uniforms: vec![
                UniformDesc::new("u_time", UniformType::Float1),
                UniformDesc::new("u_resolution", UniformType::Float2),
            ],
            ..Default::default()
        },
    ) {
        Ok(mat) => Some(mat),
        Err(e) => {
            eprintln!("Warning: background shader failed to compile: {e}");
            None
        }
    }
}

/// Simple bloom / glow post-processing shader.
/// Applied to the particle render target — brightens and softly blurs.
pub fn create_bloom_material() -> Option<Material> {
    let fragment = r#"#version 100
precision mediump float;

varying vec2 uv;
uniform sampler2D Texture;

void main() {
    vec2 tex_size = vec2(1.0 / 1280.0, 1.0 / 800.0);

    // Sample center and neighbors for a 5-tap blur
    vec4 center = texture2D(Texture, uv);
    vec4 blur = center * 0.4;
    blur += texture2D(Texture, uv + vec2( tex_size.x * 2.0, 0.0)) * 0.15;
    blur += texture2D(Texture, uv + vec2(-tex_size.x * 2.0, 0.0)) * 0.15;
    blur += texture2D(Texture, uv + vec2(0.0,  tex_size.y * 2.0)) * 0.15;
    blur += texture2D(Texture, uv + vec2(0.0, -tex_size.y * 2.0)) * 0.15;

    // Additive bloom: original + brightened blur
    vec4 bloom = center + blur * 1.2;
    bloom.a = max(center.a, blur.a);

    gl_FragColor = bloom;
}
"#;

    let vertex = DEFAULT_VERTEX_SHADER;

    match load_material(
        ShaderSource::Glsl { vertex, fragment },
        MaterialParams::default(),
    ) {
        Ok(mat) => Some(mat),
        Err(e) => {
            eprintln!("Warning: bloom shader failed to compile: {e}");
            None
        }
    }
}

/// Default vertex shader for macroquad materials.
const DEFAULT_VERTEX_SHADER: &str = r#"#version 100
precision mediump float;

attribute vec3 position;
attribute vec2 texcoord;

varying vec2 uv;

uniform mat4 Model;
uniform mat4 Projection;

void main() {
    gl_Position = Projection * Model * vec4(position, 1.0);
    uv = texcoord;
}
"#;
