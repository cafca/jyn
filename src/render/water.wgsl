// Water underlay shader for jyn's "Current & Still" design.
//
// One material, four modes (settings.mode):
//   0 — deep-water background: radial gradient #123642 → #0a2029 → #071820
//   1 — current spine: vertical cyan gradient + upward-scrolling stripes
//   2 — waterline band: bottom band of an ephemeral card with luminous top edge
//   3 — glow: soft radial falloff used as the spine's light skirt
//
// settings.rect maps the quad's local uv into full-element uv space so that
// scroll-clipped quads (shrunk to their visible intersection) keep their
// gradients anchored to the full element.

#import bevy_sprite::mesh2d_vertex_output::VertexOutput
#import bevy_sprite::mesh2d_view_bindings::globals

struct WaterSettings {
    tint: vec4<f32>,
    rect: vec4<f32>,
    mode: u32,
    element_height_px: f32,
    band_frac: f32,
    _pad: f32,
}

@group(2) @binding(0) var<uniform> settings: WaterSettings;

fn full_uv(local_uv: vec2<f32>) -> vec2<f32> {
    return mix(settings.rect.xy, settings.rect.zw, local_uv);
}

fn background(uv: vec2<f32>) -> vec4<f32> {
    let deep_a = vec3<f32>(0.0706, 0.2118, 0.2588); // #123642
    let deep_b = vec3<f32>(0.0392, 0.1255, 0.1608); // #0a2029
    let deep_c = vec3<f32>(0.0275, 0.0941, 0.1255); // #071820
    let center = vec2<f32>(0.2, 0.0);
    let t = clamp(length((uv - center) * vec2<f32>(1.1, 0.95)), 0.0, 1.0);
    var color = mix(deep_a, deep_b, clamp(t / 0.6, 0.0, 1.0));
    color = mix(color, deep_c, clamp((t - 0.6) / 0.4, 0.0, 1.0));
    return vec4<f32>(color, 1.0);
}

fn spine(uv: vec2<f32>) -> vec4<f32> {
    // Vertical intensity: faint at the ends, luminous through the middle.
    let ends = min(uv.y, 1.0 - uv.y);
    let core = 0.05 + 0.5 * smoothstep(0.0, 0.18, ends);

    // Horizontal profile: brightest along the quad's center line.
    let ridge = 1.0 - abs(uv.x - 0.5) * 2.0;
    let profile = pow(clamp(ridge, 0.0, 1.0), 1.4);

    var color = vec3<f32>(0.235, 0.882, 0.882); // rgba(60,225,225)
    var alpha = core * profile;

    // Upward-scrolling stripes: 2px streaks repeating every 48px, period 3.2s.
    let stripe_len_px = 48.0;
    let phase = fract(uv.y * settings.element_height_px / stripe_len_px - globals.time / 3.2);
    if phase < (2.0 / stripe_len_px) * 12.0 {
        color = mix(color, vec3<f32>(1.0, 1.0, 1.0), 0.28);
        alpha = min(alpha + 0.18 * profile, 1.0);
    }

    return vec4<f32>(color, alpha);
}

fn waterline(uv: vec2<f32>) -> vec4<f32> {
    let band_top = 1.0 - settings.band_frac;
    if uv.y < band_top {
        return vec4<f32>(0.0);
    }

    // Gentle shimmer of the surface line.
    let shimmer = sin(globals.time * 0.9 + uv.x * 14.0) * 0.006;
    let surface = band_top + shimmer;

    // Depth 0 at the surface, 1 at the card bottom.
    let depth = clamp((uv.y - surface) / max(settings.band_frac, 0.001), 0.0, 1.0);

    // rgba(90,235,230,.16) → rgba(60,210,210,.28), tinted for warm variants.
    let shallow = vec3<f32>(0.353, 0.922, 0.902) * settings.tint.rgb;
    let deep = vec3<f32>(0.235, 0.824, 0.824) * settings.tint.rgb;
    var color = mix(shallow, deep, depth);
    var alpha = mix(0.16, 0.28, depth) * settings.tint.a;

    // 1px luminous top edge: rgba(150,255,250,.5).
    let edge_px = 1.5 / max(settings.element_height_px, 1.0);
    if abs(uv.y - surface) < edge_px {
        color = vec3<f32>(0.588, 1.0, 0.98) * settings.tint.rgb;
        alpha = 0.5 * settings.tint.a;
    }

    return vec4<f32>(color, alpha);
}

fn glow(uv: vec2<f32>) -> vec4<f32> {
    let d = length((uv - vec2<f32>(0.5, 0.5)) * 2.0);
    let falloff = pow(clamp(1.0 - d, 0.0, 1.0), 2.2);
    return vec4<f32>(settings.tint.rgb, settings.tint.a * falloff);
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = full_uv(in.uv);
    switch settings.mode {
        case 0u: {
            return background(uv);
        }
        case 1u: {
            return spine(uv);
        }
        case 2u: {
            return waterline(uv);
        }
        default: {
            return glow(uv);
        }
    }
}
