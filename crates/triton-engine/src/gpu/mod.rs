//! GLES3 (`glow`) SDF uber-quad + instanced glyph renderer.
//!
//! All primitives render in physical (buffer) pixel space with premultiplied
//! alpha compositing and scissor-aware batching so clipped containers (`ScrollArea`,
//! overflow bounds) preserve exact draw order without extra render passes.

#![allow(unsafe_code)] // GLES3 calls via `glow::HasContext`

pub mod corner_lut;
pub mod layer_cache;

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::Hasher;

use glow::HasContext;
use layer_cache::LayerCache;

use crate::damage::DamageRect;
use crate::error::EngineError;
use crate::image::{ImageData, ImageId, ImageStore};
use crate::text::AtlasUpload;

const QUAD_VS: &str = r"#version 300 es
precision highp float;
layout(location = 0) in vec2 a_pos;
layout(location = 1) in vec4 i_dst;      // x, y, w, h (buffer px)
layout(location = 2) in vec4 i_radii;    // tl, tr, br, bl (buffer px)
layout(location = 3) in vec4 i_fill;     // straight RGBA
layout(location = 4) in vec4 i_border;   // straight RGBA (or highlight/shadow)
layout(location = 5) in vec4 i_misc;     // kind, smoothing, border_w_or_blur, opacity

uniform vec2 u_view;

out vec2 v_local;
out vec2 v_half;
out vec2 v_px;
flat out vec4 v_radii;
flat out vec4 v_fill;
flat out vec4 v_border;
flat out vec4 v_misc;

void main() {
    vec2 px = i_dst.xy + a_pos * i_dst.zw;
    v_local = a_pos * i_dst.zw;
    v_half = i_dst.zw * 0.5;
    v_px = px;
    v_radii = i_radii;
    v_fill = i_fill;
    v_border = i_border;
    v_misc = i_misc;
    gl_Position = vec4(
        px.x / u_view.x * 2.0 - 1.0,
        1.0 - px.y / u_view.y * 2.0,
        0.0,
        1.0
    );
}
";

const QUAD_FS: &str = r"#version 300 es
precision highp float;

uniform sampler2D u_corner_lut;
uniform vec4 u_fade_rect;
uniform vec4 u_fade_widths;

in vec2 v_local;
in vec2 v_half;
in vec2 v_px;
flat in vec4 v_radii;
flat in vec4 v_fill;
flat in vec4 v_border;
flat in vec4 v_misc;

out vec4 frag;

float edge_fade_alpha(vec2 px, vec4 rect, vec4 w) {
    if (w.x <= 0.0 && w.y <= 0.0 && w.z <= 0.0 && w.w <= 0.0) {
        return 1.0;
    }
    float a = 1.0;
    if (w.x > 0.0) {
        a *= smoothstep(0.0, w.x, px.x - rect.x);
    }
    if (w.y > 0.0) {
        a *= smoothstep(0.0, w.y, (rect.x + rect.z) - px.x);
    }
    if (w.z > 0.0) {
        a *= smoothstep(0.0, w.z, px.y - rect.y);
    }
    if (w.w > 0.0) {
        a *= smoothstep(0.0, w.w, (rect.y + rect.w) - px.y);
    }
    return a;
}

// Standard circular rounded-rect SDF.
float sd_round(vec2 p, vec2 b, vec4 r) {
    float rr = (p.y < 0.0) ? ((p.x < 0.0) ? r.x : r.y)
                           : ((p.x < 0.0) ? r.w : r.z);
    rr = clamp(rr, 0.0, min(b.x, b.y));
    vec2 q = abs(p) - b + rr;
    return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - rr;
}

// Exact Figma smooth-corner SDF sampled from the 256x256 corner LUT.
float sd_smooth(vec2 p, vec2 b, vec4 r) {
    float rr = (p.y < 0.0) ? ((p.x < 0.0) ? r.x : r.y)
                           : ((p.x < 0.0) ? r.w : r.z);
    rr = clamp(rr, 0.0, min(b.x, b.y));
    if (rr <= 0.001) {
        vec2 q0 = abs(p) - b;
        return length(max(q0, 0.0)) + min(max(q0.x, q0.y), 0.0);
    }
    float s_eff = clamp(min(b.x, b.y) / rr - 1.0, 0.0, 0.6);
    vec2 c = b - abs(p);
    const float LUT_EXTENT = 1.6;
    if (c.x < LUT_EXTENT * rr && c.y < LUT_EXTENT * rr) {
        vec2 uv = clamp(c / (LUT_EXTENT * rr), 0.0, 1.0);
        float d_lut = (texture(u_corner_lut, uv).r - 0.5) * 2.0 * rr;
        return mix(sd_round(p, b, r), d_lut, s_eff / 0.6);
    }
    return -min(c.x, c.y);
}

float sd_box(vec2 p, vec2 b, vec4 r, float smoothing) {
    float d_circ = sd_round(p, b, r);
    if (smoothing <= 0.001) {
        return d_circ;
    }
    float d_sm = sd_smooth(p, b, r);
    return mix(d_circ, d_sm, clamp(smoothing, 0.0, 1.0));
}

// Error-function approximation for soft Gaussian-like drop shadows.
float shadow_falloff(float d, float sigma) {
    float t = clamp(d / max(sigma, 0.5), -3.0, 3.0);
    // Smooth S-curve matching 0.5 * erfc(d / (sigma * sqrt(2)))
    float s = 1.0 / (1.0 + exp(1.7 * t));
    return s;
}

void main() {
    vec2 p = v_local - v_half;
    float kind = v_misc.x;
    float smoothing = v_misc.y;
    float param = v_misc.z;
    float opacity = v_misc.w;
    float fade = edge_fade_alpha(v_px, u_fade_rect, u_fade_widths);

    // Kind 4: Analytic round-capped line segment (capsule) SDF.
    // `i_dst` is the segment's bounding quad padded by radius + AA margin;
    // `i_radii` = (ax, ay, bx, by) in local quad coordinates (buffer px);
    // `v_misc.z` = stroke half-width (radius, buffer px).
    if (kind > 3.5) {
        vec2 a = v_radii.xy;
        vec2 b = v_radii.zw;
        vec2 pa = v_local - a;
        vec2 ba = b - a;
        float denom = dot(ba, ba);
        float h = (denom > 1e-6) ? clamp(dot(pa, ba) / denom, 0.0, 1.0) : 0.0;
        float d = length(pa - ba * h) - param;
        float aa = max(fwidth(d), 0.7);
        float cover = 1.0 - smoothstep(-aa, aa, d);
        float a_out = v_fill.a * cover * opacity * fade;
        frag = vec4(v_fill.rgb * a_out, a_out);
        return;
    }

    // Kind 3: Analytic 9-lobe Wave SDF (`i_radii` = phase, lobe_amp, lobes, 0).
    if (kind > 2.5) {
        float phase = v_radii.x;
        float amp = v_radii.y;
        float lobes = v_radii.z;
        float base_r = min(v_half.x, v_half.y) / (1.0 + amp);
        float ang = atan(p.y, p.x);
        float r_wave = base_r * (1.0 + amp * cos(lobes * ang + phase));
        float d = length(p) - r_wave;
        float aa = max(fwidth(d), 0.7);
        float cover = 1.0 - smoothstep(-aa, aa, d);
        vec4 col = v_fill;
        float border_w = param;
        if (border_w > 0.0) {
            float inner = -d - border_w;
            float bcov = cover * (1.0 - smoothstep(-aa, aa, inner));
            float ba = v_border.a * bcov;
            float out_a = ba + col.a * (1.0 - ba);
            vec3 out_rgb = (v_border.rgb * ba + col.rgb * col.a * (1.0 - ba)) / max(out_a, 1e-5);
            col = vec4(out_rgb, out_a);
        }
        float a = col.a * cover * opacity * fade;
        frag = vec4(col.rgb * a, a);
        return;
    }

    // Kind 2: Soft SDF drop shadow. `i_dst` is already expanded by `2 * blur` around
    // the shifted caster rect, so the shifted caster half-size is `v_half - 2 * blur`.
    // `v_border` carries `(offset_x, offset_y, spread, cutout)` in buffer px so we can
    // cleanly cut out the unshifted caster interior under translucent Glass/Thick fills.
    if (kind > 1.5) {
        float blur = max(param, 1.0);
        float pad = blur * 2.0;
        vec2 caster_half = max(v_half - vec2(pad), vec2(0.5));
        float d = sd_box(p, caster_half, v_radii, smoothing);
        float a = shadow_falloff(d, blur * 0.55) * v_fill.a * opacity;
        // Fade cleanly to zero near the quad boundary to prevent hard clip lines.
        vec2 edge = v_half - abs(p);
        float edge_fade = smoothstep(0.0, max(blur * 0.35, 1.0), min(edge.x, edge.y));
        a *= edge_fade;
        // Cut out the unshifted caster interior when requested so translucent Glass
        // panels never get darkened/muddied by their own drop shadow bleeding through.
        if (v_border.w > 0.5) {
            vec2 unshifted_p = p + v_border.xy;
            vec2 unshifted_half = max(caster_half - vec2(v_border.z), vec2(0.5));
            vec4 unshifted_r = max(v_radii - vec4(v_border.z), vec4(0.0));
            float d_caster = sd_box(unshifted_p, unshifted_half, unshifted_r, smoothing);
            float aa_c = max(fwidth(d_caster), 0.7);
            float outside = smoothstep(-aa_c, aa_c, d_caster);
            a *= outside;
        }
        a *= fade;
        frag = vec4(v_fill.rgb * a, a);
        return;
    }

    float d = sd_box(p, v_half, v_radii, smoothing);
    float aa = max(fwidth(d), 0.7);
    float cover = 1.0 - smoothstep(-aa, aa, d);

    // Base fill
    vec4 col = v_fill;

    // Inner border ring vs. Glass/Thick top highlight
    float border_w = param;
    if (kind > 0.5) {
        // Kind 1: Glass/Thick subtle perimeter hairline + 1px inner top specular highlight.
        float hl_w = max(border_w, 1.0);
        float rim_band = cover * (1.0 - smoothstep(-aa, aa, -d - hl_w));
        float rr_top = (p.x < 0.0) ? v_radii.x : v_radii.y;
        float top_mask = 1.0 - smoothstep(max(rr_top * 0.45, 1.0), max(rr_top * 1.15, 3.0), v_local.y);
        // Perimeter hairline (~40% of highlight alpha) smoothly rising to 100% along top edge.
        float rim_alpha = v_border.a * mix(0.40, 1.0, top_mask) * rim_band;
        float out_a = rim_alpha + col.a * (1.0 - rim_alpha);
        vec3 out_rgb = (v_border.rgb * rim_alpha + col.rgb * col.a * (1.0 - rim_alpha)) / max(out_a, 1e-5);
        col = vec4(out_rgb, out_a);
    } else if (border_w > 0.0) {
        float inner = -d - border_w;
        float bcov = cover * (1.0 - smoothstep(-aa, aa, inner));
        float ba = v_border.a * bcov;
        float out_a = ba + col.a * (1.0 - ba);
        vec3 out_rgb = (v_border.rgb * ba + col.rgb * col.a * (1.0 - ba)) / max(out_a, 1e-5);
        col = vec4(out_rgb, out_a);
    }

    float a = col.a * cover * opacity * fade;
    frag = vec4(col.rgb * a, a);
}
";

const GLYPH_VS: &str = r"#version 300 es
precision highp float;
layout(location = 0) in vec2 a_pos;
layout(location = 1) in vec4 i_dst;    // x, y, w, h (buffer px)
layout(location = 2) in vec4 i_uv;     // u0, v0, uw, vh (0..1)
layout(location = 3) in vec4 i_color;  // straight RGBA
layout(location = 4) in vec4 i_misc;   // kind (0 mask, 1 color), opacity, 0, 0

uniform vec2 u_view;

out vec2 v_uv;
out vec2 v_px;
flat out vec4 v_color;
flat out vec4 v_misc;

void main() {
    vec2 px = i_dst.xy + a_pos * i_dst.zw;
    v_uv = i_uv.xy + a_pos * i_uv.zw;
    v_px = px;
    v_color = i_color;
    v_misc = i_misc;
    gl_Position = vec4(
        px.x / u_view.x * 2.0 - 1.0,
        1.0 - px.y / u_view.y * 2.0,
        0.0,
        1.0
    );
}
";

const GLYPH_FS: &str = r"#version 300 es
precision highp float;

uniform sampler2D u_mask;
uniform sampler2D u_bitmap;
uniform vec4 u_fade_rect;
uniform vec4 u_fade_widths;

in vec2 v_uv;
in vec2 v_px;
flat in vec4 v_color;
flat in vec4 v_misc;

out vec4 frag;

float edge_fade_alpha(vec2 px, vec4 rect, vec4 w) {
    if (w.x <= 0.0 && w.y <= 0.0 && w.z <= 0.0 && w.w <= 0.0) {
        return 1.0;
    }
    float a = 1.0;
    if (w.x > 0.0) {
        a *= smoothstep(0.0, w.x, px.x - rect.x);
    }
    if (w.y > 0.0) {
        a *= smoothstep(0.0, w.y, (rect.x + rect.z) - px.x);
    }
    if (w.z > 0.0) {
        a *= smoothstep(0.0, w.z, px.y - rect.y);
    }
    if (w.w > 0.0) {
        a *= smoothstep(0.0, w.w, (rect.y + rect.w) - px.y);
    }
    return a;
}

void main() {
    float fade = edge_fade_alpha(v_px, u_fade_rect, u_fade_widths);
    if (v_misc.x > 0.5) {
        vec4 c = texture(u_bitmap, v_uv);
        float a = c.a * v_misc.y * fade;
        frag = vec4(c.rgb * a, a);
    } else {
        float m = texture(u_mask, v_uv).r;
        float a = v_color.a * m * v_misc.y * fade;
        frag = vec4(v_color.rgb * a, a);
    }
}
";

const IMAGE_VS: &str = r"#version 300 es
precision highp float;
layout(location = 0) in vec2 a_pos;
layout(location = 1) in vec4 i_dst;      // x, y, w, h (buffer px)
layout(location = 2) in vec4 i_radii;    // tl, tr, br, bl (buffer px) or wave (phase, amp, lobes, 0)
layout(location = 3) in vec4 i_uv0;      // u0, v0, uw, vh into u_tex0
layout(location = 4) in vec4 i_uv1;      // u0, v0, uw, vh into u_tex1 (crossfade target)
layout(location = 5) in vec4 i_border;   // straight RGBA border color
layout(location = 6) in vec4 i_misc;     // clip (0 rounded, 1 wave), smoothing, border_w, opacity
layout(location = 7) in vec4 i_fx;       // crossfade_t, blur, desaturate, premultiplied

uniform vec2 u_view;

out vec2 v_uv0;
out vec2 v_uv1;
out vec2 v_local;
out vec2 v_half;
out vec2 v_px;
flat out vec4 v_uvrect0;
flat out vec4 v_uvrect1;
flat out vec4 v_radii;
flat out vec4 v_border;
flat out vec4 v_misc;
flat out vec4 v_fx;

void main() {
    vec2 px = i_dst.xy + a_pos * i_dst.zw;
    v_uv0 = i_uv0.xy + a_pos * i_uv0.zw;
    v_uv1 = i_uv1.xy + a_pos * i_uv1.zw;
    v_local = a_pos * i_dst.zw;
    v_half = i_dst.zw * 0.5;
    v_px = px;
    v_uvrect0 = i_uv0;
    v_uvrect1 = i_uv1;
    v_radii = i_radii;
    v_border = i_border;
    v_misc = i_misc;
    v_fx = i_fx;
    gl_Position = vec4(
        px.x / u_view.x * 2.0 - 1.0,
        1.0 - px.y / u_view.y * 2.0,
        0.0,
        1.0
    );
}
";

const IMAGE_FS: &str = r"#version 300 es
precision highp float;

uniform sampler2D u_tex0;
uniform sampler2D u_tex1;
uniform sampler2D u_corner_lut;
uniform float u_has_tex1;
uniform vec4 u_fade_rect;
uniform vec4 u_fade_widths;

in vec2 v_uv0;
in vec2 v_uv1;
in vec2 v_local;
in vec2 v_half;
in vec2 v_px;
flat in vec4 v_uvrect0;
flat in vec4 v_uvrect1;
flat in vec4 v_radii;
flat in vec4 v_border;
flat in vec4 v_misc;
flat in vec4 v_fx;

out vec4 frag;

const float MAX_BLUR_TEXELS = 40.0;
const vec3 REC709 = vec3(0.2126, 0.7152, 0.0722);

float edge_fade_alpha(vec2 px, vec4 rect, vec4 w) {
    if (w.x <= 0.0 && w.y <= 0.0 && w.z <= 0.0 && w.w <= 0.0) {
        return 1.0;
    }
    float a = 1.0;
    if (w.x > 0.0) {
        a *= smoothstep(0.0, w.x, px.x - rect.x);
    }
    if (w.y > 0.0) {
        a *= smoothstep(0.0, w.y, (rect.x + rect.z) - px.x);
    }
    if (w.z > 0.0) {
        a *= smoothstep(0.0, w.z, px.y - rect.y);
    }
    if (w.w > 0.0) {
        a *= smoothstep(0.0, w.w, (rect.y + rect.w) - px.y);
    }
    return a;
}

// Standard circular rounded-rect SDF.
float sd_round(vec2 p, vec2 b, vec4 r) {
    float rr = (p.y < 0.0) ? ((p.x < 0.0) ? r.x : r.y)
                           : ((p.x < 0.0) ? r.w : r.z);
    rr = clamp(rr, 0.0, min(b.x, b.y));
    vec2 q = abs(p) - b + rr;
    return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - rr;
}

// Exact Figma smooth-corner SDF sampled from the 256x256 corner LUT.
float sd_smooth(vec2 p, vec2 b, vec4 r) {
    float rr = (p.y < 0.0) ? ((p.x < 0.0) ? r.x : r.y)
                           : ((p.x < 0.0) ? r.w : r.z);
    rr = clamp(rr, 0.0, min(b.x, b.y));
    if (rr <= 0.001) {
        vec2 q0 = abs(p) - b;
        return length(max(q0, 0.0)) + min(max(q0.x, q0.y), 0.0);
    }
    float s_eff = clamp(min(b.x, b.y) / rr - 1.0, 0.0, 0.6);
    vec2 c = b - abs(p);
    const float LUT_EXTENT = 1.6;
    if (c.x < LUT_EXTENT * rr && c.y < LUT_EXTENT * rr) {
        vec2 uv = clamp(c / (LUT_EXTENT * rr), 0.0, 1.0);
        float d_lut = (texture(u_corner_lut, uv).r - 0.5) * 2.0 * rr;
        return mix(sd_round(p, b, r), d_lut, s_eff / 0.6);
    }
    return -min(c.x, c.y);
}

float sd_box(vec2 p, vec2 b, vec4 r, float smoothing) {
    float d_circ = sd_round(p, b, r);
    if (smoothing <= 0.001) {
        return d_circ;
    }
    float d_sm = sd_smooth(p, b, r);
    return mix(d_circ, d_sm, clamp(smoothing, 0.0, 1.0));
}

// 13-tap disc kernel: all taps are clamped inside the cropped UV rect so blur
// never bleeds neighbouring texels or the cropped-away region of the source.
// `uvrect.zw` may be negative (e.g. Y-flipped FBO layer UVs), so the clamp
// bounds are computed component-wise instead of assuming positive extents.
vec4 blur_sample(sampler2D tex, vec2 uv, vec4 uvrect, float blur) {
    vec2 a = uvrect.xy;
    vec2 b = uvrect.xy + uvrect.zw;
    vec2 lo = min(a, b);
    vec2 hi = max(a, b);
    if (blur <= 0.001) {
        return texture(tex, clamp(uv, lo, hi));
    }
    vec2 r = blur * MAX_BLUR_TEXELS / vec2(textureSize(tex, 0));
    vec4 acc = texture(tex, clamp(uv, lo, hi)) * 4.0;
    vec2 r1 = r * 0.55;
    acc += texture(tex, clamp(uv + vec2( r1.x,  0.0), lo, hi)) * 1.5;
    acc += texture(tex, clamp(uv + vec2(-r1.x,  0.0), lo, hi)) * 1.5;
    acc += texture(tex, clamp(uv + vec2(  0.0,  r1.y), lo, hi)) * 1.5;
    acc += texture(tex, clamp(uv + vec2(  0.0, -r1.y), lo, hi)) * 1.5;
    acc += texture(tex, clamp(uv + vec2( r1.x,  r1.y), lo, hi)) * 1.5;
    acc += texture(tex, clamp(uv + vec2(-r1.x,  r1.y), lo, hi)) * 1.5;
    acc += texture(tex, clamp(uv + vec2( r1.x, -r1.y), lo, hi)) * 1.5;
    acc += texture(tex, clamp(uv + vec2(-r1.x, -r1.y), lo, hi)) * 1.5;
    acc += texture(tex, clamp(uv + vec2( r.x,  0.0), lo, hi));
    acc += texture(tex, clamp(uv + vec2(-r.x,  0.0), lo, hi));
    acc += texture(tex, clamp(uv + vec2( 0.0,  r.y), lo, hi));
    acc += texture(tex, clamp(uv + vec2( 0.0, -r.y), lo, hi));
    return acc / 20.0;
}

void main() {
    vec2 p = v_local - v_half;
    float clip_wave = v_misc.x;
    float smoothing = v_misc.y;
    float border_w = v_misc.z;
    float opacity = v_misc.w;
    float crossfade_t = clamp(v_fx.x, 0.0, 1.0);
    float blur = clamp(v_fx.y, 0.0, 1.0);
    float desaturate = clamp(v_fx.z, 0.0, 1.0);
    float premul = v_fx.w;
    float fade = edge_fade_alpha(v_px, u_fade_rect, u_fade_widths);

    vec4 col = blur_sample(u_tex0, v_uv0, v_uvrect0, blur);
    if (u_has_tex1 > 0.5 && crossfade_t > 0.0) {
        vec4 col1 = blur_sample(u_tex1, v_uv1, v_uvrect1, blur);
        col = mix(col, col1, crossfade_t);
    }
    if (desaturate > 0.0) {
        col.rgb = mix(col.rgb, vec3(dot(col.rgb, REC709)), desaturate);
    }

    // Clip: analytic 9-lobe wave SDF (`i_radii` = phase, amp, lobes, 0) or the
    // rounded-corner LUT box SDF shared with QUAD_FS.
    float d;
    if (clip_wave > 0.5) {
        float phase = v_radii.x;
        float amp = v_radii.y;
        float lobes = v_radii.z;
        float base_r = min(v_half.x, v_half.y) / (1.0 + amp);
        float ang = atan(p.y, p.x);
        float r_wave = base_r * (1.0 + amp * cos(lobes * ang + phase));
        d = length(p) - r_wave;
    } else {
        d = sd_box(p, v_half, v_radii, smoothing);
    }
    float aa = max(fwidth(d), 0.7);
    float cover = 1.0 - smoothstep(-aa, aa, d);

    // Inner border ring drawn over the image edge (same math as QUAD_FS).
    if (border_w > 0.0) {
        float inner = -d - border_w;
        float bcov = cover * (1.0 - smoothstep(-aa, aa, inner));
        float ba = v_border.a * bcov;
        float out_a = ba + col.a * (1.0 - ba);
        vec3 out_rgb = (v_border.rgb * ba + col.rgb * col.a * (1.0 - ba)) / max(out_a, 1e-5);
        col = vec4(out_rgb, out_a);
    }

    // FBO layer textures already hold premultiplied RGBA from the ONE,
    // ONE_MINUS_SRC_ALPHA blend into the offscreen pass, while CPU-uploaded
    // images carry straight alpha that still needs premultiplying here.
    if (premul > 0.5) {
        float scale_a = cover * opacity * fade;
        frag = col * scale_a;
    } else {
        float a = col.a * cover * opacity * fade;
        frag = vec4(col.rgb * a, a);
    }
}
";

/// Primitive kinds for [`QuadInstance::misc`]`[0]`.
pub const KIND_FILL: f32 = 0.0;
pub const KIND_TOP_HIGHLIGHT: f32 = 1.0;
pub const KIND_SHADOW: f32 = 2.0;
pub const KIND_WAVE: f32 = 3.0;
pub const KIND_CAPSULE: f32 = 4.0;

/// Highest per-instance vertex attribute location used by any pass (`IMAGE`
/// uses locations `1..=7`; `QUAD` `1..=5`; `GLYPH` `1..=4`).
const MAX_INSTANCE_ATTR: u32 = 7;

/// One SDF shape or shadow instance (20 floats, 80 bytes).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct QuadInstance {
    pub dst: [f32; 4],
    pub radii: [f32; 4],
    pub fill: [f32; 4],
    pub border: [f32; 4],
    /// `[kind, smoothing, border_w_or_blur, opacity]`.
    pub misc: [f32; 4],
}

/// One glyph instance (16 floats, 64 bytes).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct GlyphInstance {
    pub dst: [f32; 4],
    pub uv: [f32; 4],
    pub color: [f32; 4],
    /// `[kind (0 mask / 1 color), opacity, 0, 0]`.
    pub misc: [f32; 4],
}

/// Clip shape mode for [`ImageInstance::misc`]`[0]`.
pub const IMAGE_CLIP_ROUNDED: f32 = 0.0;
pub const IMAGE_CLIP_WAVE: f32 = 1.0;

/// One textured-quad image instance (28 floats, 112 bytes).
///
/// All geometry lives in physical buffer pixels; `uv0`/`uv1` are normalized
/// `[u0, v0, uw, vh]` rects into `u_tex0`/`u_tex1` so a wallpaper crossfade can
/// `Cover`-crop two differently-proportioned sources independently.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct ImageInstance {
    /// Destination rect `[x, y, w, h]` in buffer px.
    pub dst: [f32; 4],
    /// Per-corner radii `[tl, tr, br, bl]` in buffer px for
    /// [`IMAGE_CLIP_ROUNDED`], or `[phase, amp, lobes, 0]` for [`IMAGE_CLIP_WAVE`].
    pub radii: [f32; 4],
    /// UV rect `[u0, v0, uw, vh]` into the batch's primary texture (`u_tex0`).
    pub uv0: [f32; 4],
    /// UV rect `[u0, v0, uw, vh]` into the crossfade texture (`u_tex1`).
    pub uv1: [f32; 4],
    /// Straight RGBA inner border color (drawn over the image edge).
    pub border: [f32; 4],
    /// `[clip mode, smoothing, border_w (buffer px), opacity]`.
    pub misc: [f32; 4],
    /// `[crossfade_t, blur, desaturate, premultiplied]`: the first three are
    /// normalized `0.0..=1.0`; `premultiplied` is `1.0` when `u_tex0` already
    /// holds premultiplied RGBA (FBO layer textures) and `0.0` for straight
    /// alpha CPU uploads.
    pub fx: [f32; 4],
}

/// Scissor rectangle in physical buffer pixels, origin top-left.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScissorRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// Edge-fade mask in physical buffer pixels: `rect = [x, y, w, h]`,
/// `widths = [left, right, top, bottom]`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct EdgeFade {
    pub rect: [f32; 4],
    pub widths: [f32; 4],
}

/// A batch of primitives sharing one scissor clip rect and edge fade, preserving z-order.
#[derive(Debug, Clone)]
pub enum DrawBatch {
    Quads {
        scissor: Option<ScissorRect>,
        edge_fade: Option<EdgeFade>,
        instances: Vec<QuadInstance>,
    },
    Glyphs {
        scissor: Option<ScissorRect>,
        edge_fade: Option<EdgeFade>,
        instances: Vec<GlyphInstance>,
    },
    Images {
        image_id: ImageId,
        crossfade_to: Option<ImageId>,
        scissor: Option<ScissorRect>,
        edge_fade: Option<EdgeFade>,
        instances: Vec<ImageInstance>,
    },
    /// A subtree rasterized into an offscreen [`LayerCache`] FBO and composited
    /// as a single premultiplied textured quad at `dst`.
    CachedLayer {
        /// [`crate::ui::NodeKey`] identity; stable across frames for cache hits.
        key: u64,
        /// Offscreen texture width in physical (buffer) pixels.
        width: u32,
        /// Offscreen texture height in physical (buffer) pixels.
        height: u32,
        /// Deterministic hash of `content`; a hit composites without re-rasterizing.
        content_hash: u64,
        /// Composite destination rect `[x, y, w, h]` in physical buffer px.
        dst: [f32; 4],
        /// Opacity applied when compositing the cached texture.
        opacity: f32,
        scissor: Option<ScissorRect>,
        edge_fade: Option<EdgeFade>,
        /// The subtree's own draw list (rendered into the FBO on a cache miss).
        content: Box<DrawList>,
    },
}

/// Ordered command buffer of scissor-aware batches for a single surface frame.
#[derive(Debug, Default, Clone)]
pub struct DrawList {
    pub batches: Vec<DrawBatch>,
}

impl DrawList {
    pub fn clear(&mut self) {
        self.batches.clear();
    }

    pub fn push_quad(&mut self, scissor: Option<ScissorRect>, quad: QuadInstance) {
        self.push_quad_faded(scissor, None, quad);
    }

    pub fn push_quad_faded(
        &mut self,
        scissor: Option<ScissorRect>,
        edge_fade: Option<EdgeFade>,
        quad: QuadInstance,
    ) {
        if let Some(DrawBatch::Quads {
            scissor: last_sc,
            edge_fade: last_ef,
            instances,
        }) = self.batches.last_mut()
            && *last_sc == scissor
            && *last_ef == edge_fade
        {
            instances.push(quad);
            return;
        }
        self.batches.push(DrawBatch::Quads {
            scissor,
            edge_fade,
            instances: vec![quad],
        });
    }

    pub fn push_glyphs(&mut self, scissor: Option<ScissorRect>, glyphs: &[GlyphInstance]) {
        self.push_glyphs_faded(scissor, None, glyphs);
    }

    pub fn push_glyphs_faded(
        &mut self,
        scissor: Option<ScissorRect>,
        edge_fade: Option<EdgeFade>,
        glyphs: &[GlyphInstance],
    ) {
        if glyphs.is_empty() {
            return;
        }
        if let Some(DrawBatch::Glyphs {
            scissor: last_sc,
            edge_fade: last_ef,
            instances,
        }) = self.batches.last_mut()
            && *last_sc == scissor
            && *last_ef == edge_fade
        {
            instances.extend_from_slice(glyphs);
            return;
        }
        self.batches.push(DrawBatch::Glyphs {
            scissor,
            edge_fade,
            instances: glyphs.to_vec(),
        });
    }

    pub fn push_image(
        &mut self,
        image_id: ImageId,
        scissor: Option<ScissorRect>,
        image: ImageInstance,
    ) {
        self.push_image_faded(image_id, None, scissor, None, image);
    }

    pub fn push_image_faded(
        &mut self,
        image_id: ImageId,
        crossfade_to: Option<ImageId>,
        scissor: Option<ScissorRect>,
        edge_fade: Option<EdgeFade>,
        image: ImageInstance,
    ) {
        if let Some(DrawBatch::Images {
            image_id: last_id,
            crossfade_to: last_xf,
            scissor: last_sc,
            edge_fade: last_ef,
            instances,
        }) = self.batches.last_mut()
            && *last_id == image_id
            && *last_xf == crossfade_to
            && *last_sc == scissor
            && *last_ef == edge_fade
        {
            instances.push(image);
            return;
        }
        self.batches.push(DrawBatch::Images {
            image_id,
            crossfade_to,
            scissor,
            edge_fade,
            instances: vec![image],
        });
    }

    pub fn quad_count(&self) -> usize {
        self.batches
            .iter()
            .map(|b| match b {
                DrawBatch::Quads { instances, .. } => instances.len(),
                DrawBatch::Glyphs { .. }
                | DrawBatch::Images { .. }
                | DrawBatch::CachedLayer { .. } => 0,
            })
            .sum()
    }

    pub fn glyph_count(&self) -> usize {
        self.batches
            .iter()
            .map(|b| match b {
                DrawBatch::Glyphs { instances, .. } => instances.len(),
                DrawBatch::Quads { .. }
                | DrawBatch::Images { .. }
                | DrawBatch::CachedLayer { .. } => 0,
            })
            .sum()
    }

    pub fn image_count(&self) -> usize {
        self.batches
            .iter()
            .map(|b| match b {
                DrawBatch::Images { instances, .. } => instances.len(),
                DrawBatch::Quads { .. }
                | DrawBatch::Glyphs { .. }
                | DrawBatch::CachedLayer { .. } => 0,
            })
            .sum()
    }

    /// Append a cached-layer composite batch for an `Element::layer(key)` subtree.
    #[allow(clippy::too_many_arguments)]
    pub fn push_cached_layer(
        &mut self,
        key: u64,
        width: u32,
        height: u32,
        content_hash: u64,
        dst: [f32; 4],
        opacity: f32,
        scissor: Option<ScissorRect>,
        edge_fade: Option<EdgeFade>,
        content: DrawList,
    ) {
        self.batches.push(DrawBatch::CachedLayer {
            key,
            width,
            height,
            content_hash,
            dst,
            opacity,
            scissor,
            edge_fade,
            content: Box::new(content),
        });
    }

    /// Number of [`DrawBatch::CachedLayer`] composite batches in this frame.
    pub fn layer_count(&self) -> usize {
        self.batches
            .iter()
            .filter(|b| matches!(b, DrawBatch::CachedLayer { .. }))
            .count()
    }

    /// Deterministic hash over every batch in this draw list.
    ///
    /// All floats are hashed by bit pattern (`f32::to_bits`), and nested
    /// `CachedLayer` contents fold in recursively, so two frames produce the
    /// same hash iff their visible content is bit-identical. Layer subtree
    /// `SubtreeTransform`s intentionally live *outside* the hashed `content`,
    /// so translate/scale/opacity morphs reuse the cached FBO.
    pub fn content_hash(&self) -> u64 {
        let mut h = DefaultHasher::new();
        for batch in &self.batches {
            hash_batch(&mut h, batch);
        }
        h.finish()
    }

    /// Damage bounding rect (top-left physical buffer px, padded 2px for SDF
    /// `fwidth` anti-aliasing fringe) covering everything that changed between
    /// `prev` and `self`, or `None` when the frames are identical.
    ///
    /// Matching batch kinds (equal `scissor`/`edge_fade`/image ids) are diffed
    /// instance-by-instance; any structural difference unions the bounding
    /// rects of the differing batches from both frames.
    pub fn diff_bounds(&self, prev: &DrawList) -> Option<DamageRect> {
        if self.batches.len() == prev.batches.len()
            && self
                .batches
                .iter()
                .zip(&prev.batches)
                .all(|(a, b)| batch_bit_eq(a, b))
        {
            return None;
        }
        let mut acc: Option<DamageRect> = None;
        let n = self.batches.len().max(prev.batches.len());
        for i in 0..n {
            let d = match (self.batches.get(i), prev.batches.get(i)) {
                (Some(a), Some(b)) if batches_compatible(a, b) => diff_within(a, b),
                (a, b) => union_opt(batch_bounds(a), batch_bounds(b)),
            };
            acc = union_opt(acc, d);
        }
        acc
    }
}

/// AA fringe padding (buffer px) applied around every changed primitive.
const DIFF_PAD: f32 = 2.0;

fn union_opt(a: Option<DamageRect>, b: Option<DamageRect>) -> Option<DamageRect> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.union(b)),
        (Some(_), None) => a,
        (None, Some(_)) => b,
        (None, None) => None,
    }
}

/// Bit-exact `[f32; 4]` comparison (`-0.0 != 0.0`, `NaN == NaN` by pattern).
fn f4_eq(a: &[f32; 4], b: &[f32; 4]) -> bool {
    a.iter()
        .zip(b.iter())
        .all(|(x, y)| x.to_bits() == y.to_bits())
}

fn edge_fade_eq(a: Option<EdgeFade>, b: Option<EdgeFade>) -> bool {
    match (a, b) {
        (Some(x), Some(y)) => f4_eq(&x.rect, &y.rect) && f4_eq(&x.widths, &y.widths),
        (None, None) => true,
        _ => false,
    }
}

fn quad_eq(a: &QuadInstance, b: &QuadInstance) -> bool {
    f4_eq(&a.dst, &b.dst)
        && f4_eq(&a.radii, &b.radii)
        && f4_eq(&a.fill, &b.fill)
        && f4_eq(&a.border, &b.border)
        && f4_eq(&a.misc, &b.misc)
}

fn glyph_eq(a: &GlyphInstance, b: &GlyphInstance) -> bool {
    f4_eq(&a.dst, &b.dst)
        && f4_eq(&a.uv, &b.uv)
        && f4_eq(&a.color, &b.color)
        && f4_eq(&a.misc, &b.misc)
}

fn image_eq(a: &ImageInstance, b: &ImageInstance) -> bool {
    f4_eq(&a.dst, &b.dst)
        && f4_eq(&a.radii, &b.radii)
        && f4_eq(&a.uv0, &b.uv0)
        && f4_eq(&a.uv1, &b.uv1)
        && f4_eq(&a.border, &b.border)
        && f4_eq(&a.misc, &b.misc)
        && f4_eq(&a.fx, &b.fx)
}

fn slice_eq<T>(a: &[T], b: &[T], eq: impl Fn(&T, &T) -> bool) -> bool {
    a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| eq(x, y))
}

/// Exact float-bit equality between two batches (all fields, all instances).
fn batch_bit_eq(a: &DrawBatch, b: &DrawBatch) -> bool {
    match (a, b) {
        (
            DrawBatch::Quads {
                scissor: sa,
                edge_fade: ea,
                instances: ia,
            },
            DrawBatch::Quads {
                scissor: sb,
                edge_fade: eb,
                instances: ib,
            },
        ) => sa == sb && edge_fade_eq(*ea, *eb) && slice_eq(ia, ib, quad_eq),
        (
            DrawBatch::Glyphs {
                scissor: sa,
                edge_fade: ea,
                instances: ia,
            },
            DrawBatch::Glyphs {
                scissor: sb,
                edge_fade: eb,
                instances: ib,
            },
        ) => sa == sb && edge_fade_eq(*ea, *eb) && slice_eq(ia, ib, glyph_eq),
        (
            DrawBatch::Images {
                image_id: ida,
                crossfade_to: ca,
                scissor: sa,
                edge_fade: ea,
                instances: ia,
            },
            DrawBatch::Images {
                image_id: idb,
                crossfade_to: cb,
                scissor: sb,
                edge_fade: eb,
                instances: ib,
            },
        ) => {
            ida == idb
                && ca == cb
                && sa == sb
                && edge_fade_eq(*ea, *eb)
                && slice_eq(ia, ib, image_eq)
        }
        (
            DrawBatch::CachedLayer {
                key: ka,
                width: wa,
                height: ha,
                content_hash: cha,
                dst: da,
                opacity: oa,
                scissor: sa,
                edge_fade: ea,
                ..
            },
            DrawBatch::CachedLayer {
                key: kb,
                width: wb,
                height: hb,
                content_hash: chb,
                dst: db,
                opacity: ob,
                scissor: sb,
                edge_fade: eb,
                ..
            },
        ) => {
            ka == kb
                && wa == wb
                && ha == hb
                && cha == chb
                && f4_eq(da, db)
                && oa.to_bits() == ob.to_bits()
                && sa == sb
                && edge_fade_eq(*ea, *eb)
        }
        _ => false,
    }
}

/// `true` when two batches can be diffed element-by-element: same kind with
/// equal `scissor`/`edge_fade`/identity fields.
fn batches_compatible(a: &DrawBatch, b: &DrawBatch) -> bool {
    match (a, b) {
        (
            DrawBatch::Quads {
                scissor: sa,
                edge_fade: ea,
                ..
            },
            DrawBatch::Quads {
                scissor: sb,
                edge_fade: eb,
                ..
            },
        )
        | (
            DrawBatch::Glyphs {
                scissor: sa,
                edge_fade: ea,
                ..
            },
            DrawBatch::Glyphs {
                scissor: sb,
                edge_fade: eb,
                ..
            },
        ) => sa == sb && edge_fade_eq(*ea, *eb),
        (
            DrawBatch::Images {
                image_id: ida,
                crossfade_to: ca,
                scissor: sa,
                edge_fade: ea,
                ..
            },
            DrawBatch::Images {
                image_id: idb,
                crossfade_to: cb,
                scissor: sb,
                edge_fade: eb,
                ..
            },
        ) => ida == idb && ca == cb && sa == sb && edge_fade_eq(*ea, *eb),
        (
            DrawBatch::CachedLayer {
                key: ka,
                width: wa,
                height: ha,
                scissor: sa,
                edge_fade: ea,
                ..
            },
            DrawBatch::CachedLayer {
                key: kb,
                width: wb,
                height: hb,
                scissor: sb,
                edge_fade: eb,
                ..
            },
        ) => ka == kb && wa == wb && ha == hb && sa == sb && edge_fade_eq(*ea, *eb),
        _ => false,
    }
}

/// Damage rect covering one primitive `dst` clipped to `scissor`, padded 2px
/// for SDF anti-aliasing fringe.
fn prim_damage(dst: [f32; 4], scissor: Option<ScissorRect>) -> DamageRect {
    let x = (dst[0] - DIFF_PAD).floor() as i32;
    let y = (dst[1] - DIFF_PAD).floor() as i32;
    let x1 = (dst[0] + dst[2] + DIFF_PAD).ceil() as i32;
    let y1 = (dst[1] + dst[3] + DIFF_PAD).ceil() as i32;
    let r = DamageRect {
        x,
        y,
        w: (x1 - x).max(0),
        h: (y1 - y).max(0),
    };
    match scissor {
        Some(sc) => r.intersect(DamageRect {
            x: sc.x,
            y: sc.y,
            w: sc.w,
            h: sc.h,
        }),
        None => r,
    }
}

/// Union of every primitive's padded damage rect inside one batch.
fn batch_bounds(batch: Option<&DrawBatch>) -> Option<DamageRect> {
    let batch = batch?;
    let mut acc: Option<DamageRect> = None;
    let mut add = |dst: [f32; 4], scissor: Option<ScissorRect>| {
        acc = union_opt(acc, Some(prim_damage(dst, scissor)));
    };
    match batch {
        DrawBatch::Quads {
            scissor, instances, ..
        } => {
            for q in instances {
                add(q.dst, *scissor);
            }
        }
        DrawBatch::Glyphs {
            scissor, instances, ..
        } => {
            for g in instances {
                add(g.dst, *scissor);
            }
        }
        DrawBatch::Images {
            scissor, instances, ..
        } => {
            for i in instances {
                add(i.dst, *scissor);
            }
        }
        DrawBatch::CachedLayer { dst, scissor, .. } => add(*dst, *scissor),
    }
    acc
}

/// Instance-level diff inside two compatible batches (union of each side's
/// padded rects for changed/added/removed instances).
fn diff_instances<T>(
    scissor: Option<ScissorRect>,
    a: &[T],
    b: &[T],
    dst: impl Fn(&T) -> [f32; 4],
    eq: impl Fn(&T, &T) -> bool,
) -> Option<DamageRect> {
    let mut acc: Option<DamageRect> = None;
    for j in 0..a.len().max(b.len()) {
        match (a.get(j), b.get(j)) {
            (Some(x), Some(y)) if eq(x, y) => {}
            (x, y) => {
                if let Some(i) = x {
                    acc = union_opt(acc, Some(prim_damage(dst(i), scissor)));
                }
                if let Some(i) = y {
                    acc = union_opt(acc, Some(prim_damage(dst(i), scissor)));
                }
            }
        }
    }
    acc
}

fn diff_within(a: &DrawBatch, b: &DrawBatch) -> Option<DamageRect> {
    match (a, b) {
        (
            DrawBatch::Quads {
                scissor,
                instances: ia,
                ..
            },
            DrawBatch::Quads { instances: ib, .. },
        ) => diff_instances(*scissor, ia, ib, |q| q.dst, quad_eq),
        (
            DrawBatch::Glyphs {
                scissor,
                instances: ia,
                ..
            },
            DrawBatch::Glyphs { instances: ib, .. },
        ) => diff_instances(*scissor, ia, ib, |g| g.dst, glyph_eq),
        (
            DrawBatch::Images {
                scissor,
                instances: ia,
                ..
            },
            DrawBatch::Images { instances: ib, .. },
        ) => diff_instances(*scissor, ia, ib, |i| i.dst, image_eq),
        (
            DrawBatch::CachedLayer {
                content_hash: ha,
                dst: da,
                opacity: oa,
                scissor: sa,
                ..
            },
            DrawBatch::CachedLayer {
                content_hash: hb,
                dst: db,
                opacity: ob,
                scissor: sb,
                ..
            },
        ) => {
            if ha == hb && f4_eq(da, db) && oa.to_bits() == ob.to_bits() {
                None
            } else {
                union_opt(Some(prim_damage(*da, *sa)), Some(prim_damage(*db, *sb)))
            }
        }
        _ => None,
    }
}

fn hash_f4<H: Hasher>(h: &mut H, v: &[f32; 4]) {
    for x in v {
        h.write_u32(x.to_bits());
    }
}

fn hash_scissor<H: Hasher>(h: &mut H, scissor: Option<ScissorRect>) {
    match scissor {
        Some(sc) => {
            h.write_u8(1);
            h.write_i32(sc.x);
            h.write_i32(sc.y);
            h.write_i32(sc.w);
            h.write_i32(sc.h);
        }
        None => h.write_u8(0),
    }
}

fn hash_edge_fade<H: Hasher>(h: &mut H, edge_fade: Option<EdgeFade>) {
    match edge_fade {
        Some(ef) => {
            h.write_u8(1);
            hash_f4(h, &ef.rect);
            hash_f4(h, &ef.widths);
        }
        None => h.write_u8(0),
    }
}

fn hash_batch<H: Hasher>(h: &mut H, batch: &DrawBatch) {
    match batch {
        DrawBatch::Quads {
            scissor,
            edge_fade,
            instances,
        } => {
            h.write_u8(0);
            hash_scissor(h, *scissor);
            hash_edge_fade(h, *edge_fade);
            for q in instances {
                hash_f4(h, &q.dst);
                hash_f4(h, &q.radii);
                hash_f4(h, &q.fill);
                hash_f4(h, &q.border);
                hash_f4(h, &q.misc);
            }
        }
        DrawBatch::Glyphs {
            scissor,
            edge_fade,
            instances,
        } => {
            h.write_u8(1);
            hash_scissor(h, *scissor);
            hash_edge_fade(h, *edge_fade);
            for g in instances {
                hash_f4(h, &g.dst);
                hash_f4(h, &g.uv);
                hash_f4(h, &g.color);
                hash_f4(h, &g.misc);
            }
        }
        DrawBatch::Images {
            image_id,
            crossfade_to,
            scissor,
            edge_fade,
            instances,
        } => {
            h.write_u8(2);
            h.write_u64(image_id.0);
            h.write_u64(crossfade_to.map_or(0, |id| id.0));
            hash_scissor(h, *scissor);
            hash_edge_fade(h, *edge_fade);
            for i in instances {
                hash_f4(h, &i.dst);
                hash_f4(h, &i.radii);
                hash_f4(h, &i.uv0);
                hash_f4(h, &i.uv1);
                hash_f4(h, &i.border);
                hash_f4(h, &i.misc);
                hash_f4(h, &i.fx);
            }
        }
        DrawBatch::CachedLayer {
            key,
            width,
            height,
            content_hash,
            dst,
            opacity,
            scissor,
            edge_fade,
            content,
        } => {
            h.write_u8(3);
            h.write_u64(*key);
            h.write_u32(*width);
            h.write_u32(*height);
            h.write_u64(*content_hash);
            hash_f4(h, dst);
            h.write_u32(opacity.to_bits());
            hash_scissor(h, *scissor);
            hash_edge_fade(h, *edge_fade);
            h.write_u64(content.content_hash());
        }
    }
}

/// Uploaded `RGBA8` texture backing one [`ImageId`].
#[derive(Debug)]
struct ImageTexture {
    tex: glow::Texture,
    width: u32,
    height: u32,
}

/// GLES3 shader programs and streaming instance buffers.
#[derive(Debug)]
pub struct Gpu {
    gl: glow::Context,
    quad_prog: glow::Program,
    glyph_prog: glow::Program,
    image_prog: glow::Program,
    vao: glow::VertexArray,
    unit_vbo: glow::Buffer,
    quad_bo: glow::Buffer,
    glyph_bo: glow::Buffer,
    image_bo: glow::Buffer,
    corner_lut_tex: glow::Texture,
    image_textures: HashMap<ImageId, ImageTexture>,
    layer_cache: LayerCache,
    view_loc_quad: Option<glow::UniformLocation>,
    corner_lut_loc: Option<glow::UniformLocation>,
    fade_rect_loc_quad: Option<glow::UniformLocation>,
    fade_widths_loc_quad: Option<glow::UniformLocation>,
    view_loc_glyph: Option<glow::UniformLocation>,
    mask_loc: Option<glow::UniformLocation>,
    bitmap_loc: Option<glow::UniformLocation>,
    fade_rect_loc_glyph: Option<glow::UniformLocation>,
    fade_widths_loc_glyph: Option<glow::UniformLocation>,
    view_loc_image: Option<glow::UniformLocation>,
    tex0_loc: Option<glow::UniformLocation>,
    tex1_loc: Option<glow::UniformLocation>,
    has_tex1_loc: Option<glow::UniformLocation>,
    image_lut_loc: Option<glow::UniformLocation>,
    fade_rect_loc_image: Option<glow::UniformLocation>,
    fade_widths_loc_image: Option<glow::UniformLocation>,
}

impl Gpu {
    /// Compile GLES3 shaders and allocate unit quad + instance VBOs.
    /// Requires a current GLES3 context on the calling thread.
    pub fn new(gl: glow::Context) -> Result<Self, EngineError> {
        // SAFETY: Valid GLES3 context is current; all objects are checked for errors.
        unsafe {
            let quad_prog = compile_program(&gl, QUAD_VS, QUAD_FS)?;
            let glyph_prog = compile_program(&gl, GLYPH_VS, GLYPH_FS)?;
            let image_prog = compile_program(&gl, IMAGE_VS, IMAGE_FS)?;
            let vao = gl
                .create_vertex_array()
                .map_err(|e| EngineError::Gpu(format!("create_vertex_array: {e}")))?;
            let quad_bo = gl
                .create_buffer()
                .map_err(|e| EngineError::Gpu(format!("create quad_bo: {e}")))?;
            let glyph_bo = gl
                .create_buffer()
                .map_err(|e| EngineError::Gpu(format!("create glyph_bo: {e}")))?;
            let image_bo = gl
                .create_buffer()
                .map_err(|e| EngineError::Gpu(format!("create image_bo: {e}")))?;
            let unit_vbo = gl
                .create_buffer()
                .map_err(|e| EngineError::Gpu(format!("create unit_vbo: {e}")))?;

            let lut_data =
                corner_lut::bake_smooth_corner_lut(triton_design::shape::CORNER_SMOOTHING);
            let corner_lut_tex = gl
                .create_texture()
                .map_err(|e| EngineError::Gpu(format!("create corner_lut_tex: {e}")))?;
            gl.bind_texture(glow::TEXTURE_2D, Some(corner_lut_tex));
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MIN_FILTER,
                glow::LINEAR.cast_signed(),
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MAG_FILTER,
                glow::LINEAR.cast_signed(),
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_S,
                glow::CLAMP_TO_EDGE.cast_signed(),
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_T,
                glow::CLAMP_TO_EDGE.cast_signed(),
            );
            let lut_dim = i32::try_from(corner_lut::LUT_SIZE).unwrap_or(256);
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::R8.cast_signed(),
                lut_dim,
                lut_dim,
                0,
                glow::RED,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(&lut_data)),
            );
            gl.bind_texture(glow::TEXTURE_2D, None);

            gl.bind_vertex_array(Some(vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(unit_vbo));
            let verts: [f32; 8] = [0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 1.0];
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, as_bytes(&verts), glow::STATIC_DRAW);
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, 8, 0);
            gl.vertex_attrib_divisor(0, 0);
            gl.bind_vertex_array(None);

            Ok(Self {
                view_loc_quad: gl.get_uniform_location(quad_prog, "u_view"),
                corner_lut_loc: gl.get_uniform_location(quad_prog, "u_corner_lut"),
                fade_rect_loc_quad: gl.get_uniform_location(quad_prog, "u_fade_rect"),
                fade_widths_loc_quad: gl.get_uniform_location(quad_prog, "u_fade_widths"),
                view_loc_glyph: gl.get_uniform_location(glyph_prog, "u_view"),
                mask_loc: gl.get_uniform_location(glyph_prog, "u_mask"),
                bitmap_loc: gl.get_uniform_location(glyph_prog, "u_bitmap"),
                fade_rect_loc_glyph: gl.get_uniform_location(glyph_prog, "u_fade_rect"),
                fade_widths_loc_glyph: gl.get_uniform_location(glyph_prog, "u_fade_widths"),
                view_loc_image: gl.get_uniform_location(image_prog, "u_view"),
                tex0_loc: gl.get_uniform_location(image_prog, "u_tex0"),
                tex1_loc: gl.get_uniform_location(image_prog, "u_tex1"),
                has_tex1_loc: gl.get_uniform_location(image_prog, "u_has_tex1"),
                image_lut_loc: gl.get_uniform_location(image_prog, "u_corner_lut"),
                fade_rect_loc_image: gl.get_uniform_location(image_prog, "u_fade_rect"),
                fade_widths_loc_image: gl.get_uniform_location(image_prog, "u_fade_widths"),
                gl,
                quad_prog,
                glyph_prog,
                image_prog,
                vao,
                unit_vbo,
                quad_bo,
                glyph_bo,
                image_bo,
                corner_lut_tex,
                image_textures: HashMap::new(),
                layer_cache: LayerCache::new(),
            })
        }
    }

    /// Drain the [`ImageStore`]'s pending GPU deletions and uploads into
    /// `image_textures`. Same-size updates reuse the existing texture via
    /// `glTexSubImage2D` so live screencopy frames avoid reallocation.
    ///
    /// Requires a current GLES3 context on the calling thread.
    pub fn sync_images(&mut self, store: &mut ImageStore) {
        // SAFETY: Caller guarantees a current GLES3 context (same as `render`).
        unsafe {
            for id in store.drain_deletions() {
                if let Some(entry) = self.image_textures.remove(&id) {
                    self.gl.delete_texture(entry.tex);
                }
            }
            for (id, data) in store.drain_uploads() {
                self.upload_image(id, &data);
            }
        }
    }

    unsafe fn upload_image(&mut self, id: ImageId, data: &ImageData) {
        let gl = &self.gl;
        let w = data.width.max(1);
        let h = data.height.max(1);
        // SAFETY: Caller holds a current GLES3 context.
        unsafe {
            if let Some(&ImageTexture { tex, width, height }) = self.image_textures.get(&id) {
                if width == w && height == h {
                    gl.bind_texture(glow::TEXTURE_2D, Some(tex));
                    gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
                    gl.tex_sub_image_2d(
                        glow::TEXTURE_2D,
                        0,
                        0,
                        0,
                        i32::try_from(w).unwrap_or(1),
                        i32::try_from(h).unwrap_or(1),
                        glow::RGBA,
                        glow::UNSIGNED_BYTE,
                        glow::PixelUnpackData::Slice(Some(&data.rgba)),
                    );
                    gl.bind_texture(glow::TEXTURE_2D, None);
                    return;
                }
                gl.delete_texture(tex);
                self.image_textures.remove(&id);
            }
            let tex = match gl.create_texture() {
                Ok(tex) => tex,
                Err(e) => {
                    tracing::error!(%e, "create image texture failed");
                    return;
                }
            };
            gl.bind_texture(glow::TEXTURE_2D, Some(tex));
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MIN_FILTER,
                glow::LINEAR.cast_signed(),
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MAG_FILTER,
                glow::LINEAR.cast_signed(),
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_S,
                glow::CLAMP_TO_EDGE.cast_signed(),
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_T,
                glow::CLAMP_TO_EDGE.cast_signed(),
            );
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA8.cast_signed(),
                i32::try_from(w).unwrap_or(1),
                i32::try_from(h).unwrap_or(1),
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(&data.rgba)),
            );
            gl.bind_texture(glow::TEXTURE_2D, None);
            self.image_textures.insert(
                id,
                ImageTexture {
                    tex,
                    width: w,
                    height: h,
                },
            );
        }
    }

    /// Execute a full frame's [`DrawList`] into the currently bound EGL surface,
    /// first draining pending [`ImageStore`] uploads/deletions to the GPU.
    ///
    /// `damage` is the buffer-age effective dirty rect in top-left physical px:
    /// `None` performs a full repaint, `Some` non-empty rect scissored-clips
    /// both `glClear` and every batch so undamaged backbuffer pixels survive,
    /// and `Some` empty rect skips clearing and drawing entirely.
    pub fn render(
        &mut self,
        w: u32,
        h: u32,
        draw_list: &DrawList,
        atlas: &mut AtlasUpload<'_>,
        images: &mut ImageStore,
        damage: Option<DamageRect>,
    ) {
        self.sync_images(images);
        self.layer_cache.begin_frame();
        let wi = i32::try_from(w.max(1)).unwrap_or(1);
        let hi = i32::try_from(h.max(1)).unwrap_or(1);
        let dmg = damage.map(|d| d.clamp_to_surface(wi, hi));
        if dmg.is_some_and(DamageRect::is_empty) {
            // Nothing to repaint: the backbuffer already holds this frame.
            // SAFETY: GLES3 context is current for this surface.
            unsafe { self.layer_cache.evict_stale(&self.gl) };
            return;
        }
        // Damage covering the whole surface degrades to a full repaint.
        let clip_damage = dmg
            .filter(|d| !(d.x <= 0 && d.y <= 0 && d.x + d.w >= wi && d.y + d.h >= hi))
            .map(|d| ScissorRect {
                x: d.x,
                y: d.y,
                w: d.w,
                h: d.h,
            });
        // SAFETY: Valid GLES3 context is current for this surface.
        unsafe {
            let gl = &self.gl;
            gl.viewport(0, 0, wi, hi);
            gl.disable(glow::DEPTH_TEST);
            gl.disable(glow::SCISSOR_TEST);
            gl.enable(glow::BLEND);
            gl.blend_func(glow::ONE, glow::ONE_MINUS_SRC_ALPHA);
            gl.clear_color(0.0, 0.0, 0.0, 0.0);
            if clip_damage.is_some() {
                // With buffer-age partial damage the backbuffer still holds
                // pixels from `age` frames ago; only clear the damaged region
                // so undamaged content survives the swap.
                self.apply_scissor(hi, clip_damage);
                gl.clear(glow::COLOR_BUFFER_BIT);
                gl.disable(glow::SCISSOR_TEST);
            } else {
                gl.clear(glow::COLOR_BUFFER_BIT);
            }

            // Upload any pending glyph updates once before executing batches.
            atlas.upload(gl);
        }

        self.render_batches(None, w, h, clip_damage, draw_list, atlas);

        // SAFETY: Valid GLES3 context is current for this surface.
        unsafe {
            let gl = &self.gl;
            gl.disable(glow::SCISSOR_TEST);
            self.layer_cache.evict_stale(gl);
        }
    }

    /// Execute `draw_list`'s batches into `target_fbo` (`None` = the default
    /// framebuffer of the current EGL surface) with viewport `(w, h)`.
    ///
    /// `clip_damage` intersects every batch's scissor for partial-damage
    /// repaints; offscreen FBO sub-renders always pass `None` (they repaint the
    /// full cached layer texture on a cache miss).
    #[allow(clippy::too_many_lines)]
    fn render_batches(
        &mut self,
        target_fbo: Option<glow::Framebuffer>,
        w: u32,
        h: u32,
        clip_damage: Option<ScissorRect>,
        draw_list: &DrawList,
        atlas: &mut AtlasUpload<'_>,
    ) {
        let wi = i32::try_from(w.max(1)).unwrap_or(1);
        let hi = i32::try_from(h.max(1)).unwrap_or(1);
        // SAFETY: Valid GLES3 context is current and `target_fbo` is bound.
        unsafe {
            for batch in &draw_list.batches {
                match batch {
                    DrawBatch::Quads {
                        scissor,
                        edge_fade,
                        instances,
                    } if !instances.is_empty() => {
                        self.apply_scissor(hi, intersect_scissor(*scissor, clip_damage));
                        self.draw_quads_inner(w, h, *edge_fade, instances);
                    }
                    DrawBatch::Glyphs {
                        scissor,
                        edge_fade,
                        instances,
                    } if !instances.is_empty() => {
                        self.apply_scissor(hi, intersect_scissor(*scissor, clip_damage));
                        self.draw_glyphs_inner(w, h, *edge_fade, atlas, instances);
                    }
                    DrawBatch::Images {
                        image_id,
                        crossfade_to,
                        scissor,
                        edge_fade,
                        instances,
                    } if !instances.is_empty() => {
                        // Skip batches whose texture hasn't been uploaded yet
                        // (e.g. a view referencing an ImageId the app never
                        // inserted) without panicking or raising GL errors.
                        if let Some(entry) = self.image_textures.get(image_id) {
                            let tex0 = entry.tex;
                            let tex1_entry =
                                crossfade_to.and_then(|to| self.image_textures.get(&to));
                            let (tex1, has_tex1) = match tex1_entry {
                                Some(e) => (e.tex, true),
                                None => (tex0, false),
                            };
                            self.apply_scissor(hi, intersect_scissor(*scissor, clip_damage));
                            self.draw_images_inner(
                                w, h, *edge_fade, tex0, tex1, has_tex1, instances,
                            );
                        }
                    }
                    DrawBatch::CachedLayer {
                        key,
                        width,
                        height,
                        content_hash,
                        dst,
                        opacity,
                        scissor,
                        edge_fade,
                        content,
                    } => {
                        if !self
                            .layer_cache
                            .is_valid(*key, *width, *height, *content_hash)
                        {
                            // SAFETY: `self.gl` is the active GLES3 context.
                            match self.layer_cache.acquire(
                                &self.gl,
                                *key,
                                *width,
                                *height,
                                *content_hash,
                            ) {
                                Ok(entry) => {
                                    // Grab the FBO handle before recursing so
                                    // the `&LayerEntry` borrow ends here.
                                    let fbo = entry.fbo;
                                    let fw = *width;
                                    let fh = *height;
                                    {
                                        let gl = &self.gl;
                                        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
                                        gl.viewport(
                                            0,
                                            0,
                                            i32::try_from(fw.max(1)).unwrap_or(1),
                                            i32::try_from(fh.max(1)).unwrap_or(1),
                                        );
                                        gl.disable(glow::SCISSOR_TEST);
                                        gl.clear(glow::COLOR_BUFFER_BIT);
                                    }
                                    self.render_batches(Some(fbo), fw, fh, None, content, atlas);
                                    // Restore the outer target + viewport.
                                    let gl = &self.gl;
                                    gl.bind_framebuffer(glow::FRAMEBUFFER, target_fbo);
                                    gl.viewport(0, 0, wi, hi);
                                }
                                Err(e) => {
                                    tracing::error!(key, %e, "layer acquire failed");
                                }
                            }
                        }
                        if let Some(entry) = self.layer_cache.get(*key) {
                            let tex = entry.tex;
                            self.apply_scissor(hi, intersect_scissor(*scissor, clip_damage));
                            // Composite the premultiplied FBO texture. uv0's
                            // negative vh flips Y: OpenGL FBO row 0 (v=0) is
                            // the logical bottom of our top-left-origin frame.
                            let inst = ImageInstance {
                                dst: *dst,
                                radii: [0.0; 4],
                                uv0: [0.0, 1.0, 1.0, -1.0],
                                uv1: [0.0, 0.0, 1.0, 1.0],
                                border: [0.0; 4],
                                misc: [IMAGE_CLIP_ROUNDED, 0.0, 0.0, *opacity],
                                fx: [0.0, 0.0, 0.0, 1.0],
                            };
                            self.draw_images_inner(
                                w,
                                h,
                                *edge_fade,
                                tex,
                                tex,
                                false,
                                std::slice::from_ref(&inst),
                            );
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    unsafe fn apply_scissor(&self, viewport_h: i32, scissor: Option<ScissorRect>) {
        let gl = &self.gl;
        // SAFETY: Caller holds a current GLES3 context.
        unsafe {
            if let Some(sc) = scissor {
                gl.enable(glow::SCISSOR_TEST);
                // Convert top-left origin to GL bottom-left origin.
                let gl_y = viewport_h - (sc.y + sc.h);
                gl.scissor(sc.x, gl_y, sc.w.max(0), sc.h.max(0));
            } else {
                gl.disable(glow::SCISSOR_TEST);
            }
        }
    }

    unsafe fn draw_quads_inner(
        &self,
        w: u32,
        h: u32,
        edge_fade: Option<EdgeFade>,
        quads: &[QuadInstance],
    ) {
        let gl = &self.gl;
        let ef = edge_fade.unwrap_or_default();
        // SAFETY: Caller holds a current GLES3 context and `quads` is non-empty.
        unsafe {
            gl.use_program(Some(self.quad_prog));
            if let Some(loc) = &self.view_loc_quad {
                gl.uniform_2_f32(Some(loc), w as f32, h as f32);
            }
            if let Some(loc) = &self.corner_lut_loc {
                gl.uniform_1_i32(Some(loc), 2);
            }
            if let Some(loc) = &self.fade_rect_loc_quad {
                gl.uniform_4_f32(Some(loc), ef.rect[0], ef.rect[1], ef.rect[2], ef.rect[3]);
            }
            if let Some(loc) = &self.fade_widths_loc_quad {
                gl.uniform_4_f32(
                    Some(loc),
                    ef.widths[0],
                    ef.widths[1],
                    ef.widths[2],
                    ef.widths[3],
                );
            }
            gl.active_texture(glow::TEXTURE2);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.corner_lut_tex));
            gl.bind_vertex_array(Some(self.vao));
            self.bind_instance_attribs(
                self.quad_bo,
                as_bytes(quads),
                std::mem::size_of::<QuadInstance>(),
                1,
                5,
            );
            gl.draw_arrays_instanced(
                glow::TRIANGLE_STRIP,
                0,
                4,
                i32::try_from(quads.len()).unwrap_or(0),
            );
            gl.bind_vertex_array(None);
        }
    }

    unsafe fn draw_glyphs_inner(
        &self,
        w: u32,
        h: u32,
        edge_fade: Option<EdgeFade>,
        atlas: &AtlasUpload<'_>,
        glyphs: &[GlyphInstance],
    ) {
        let gl = &self.gl;
        let ef = edge_fade.unwrap_or_default();
        // SAFETY: Caller holds a current GLES3 context and `glyphs` is non-empty.
        unsafe {
            gl.use_program(Some(self.glyph_prog));
            if let Some(loc) = &self.view_loc_glyph {
                gl.uniform_2_f32(Some(loc), w as f32, h as f32);
            }
            if let Some(loc) = &self.mask_loc {
                gl.uniform_1_i32(Some(loc), 0);
            }
            if let Some(loc) = &self.bitmap_loc {
                gl.uniform_1_i32(Some(loc), 1);
            }
            if let Some(loc) = &self.fade_rect_loc_glyph {
                gl.uniform_4_f32(Some(loc), ef.rect[0], ef.rect[1], ef.rect[2], ef.rect[3]);
            }
            if let Some(loc) = &self.fade_widths_loc_glyph {
                gl.uniform_4_f32(
                    Some(loc),
                    ef.widths[0],
                    ef.widths[1],
                    ef.widths[2],
                    ef.widths[3],
                );
            }
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, atlas.mask_tex());
            gl.active_texture(glow::TEXTURE1);
            gl.bind_texture(glow::TEXTURE_2D, atlas.bitmap_tex());
            gl.bind_vertex_array(Some(self.vao));
            self.bind_instance_attribs(
                self.glyph_bo,
                as_bytes(glyphs),
                std::mem::size_of::<GlyphInstance>(),
                1,
                4,
            );
            gl.draw_arrays_instanced(
                glow::TRIANGLE_STRIP,
                0,
                4,
                i32::try_from(glyphs.len()).unwrap_or(0),
            );
            gl.bind_vertex_array(None);
        }
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn draw_images_inner(
        &self,
        w: u32,
        h: u32,
        edge_fade: Option<EdgeFade>,
        tex0: glow::Texture,
        tex1: glow::Texture,
        has_tex1: bool,
        images: &[ImageInstance],
    ) {
        let gl = &self.gl;
        let ef = edge_fade.unwrap_or_default();
        // SAFETY: Caller holds a current GLES3 context and `images` is non-empty.
        unsafe {
            gl.use_program(Some(self.image_prog));
            if let Some(loc) = &self.view_loc_image {
                gl.uniform_2_f32(Some(loc), w as f32, h as f32);
            }
            if let Some(loc) = &self.tex0_loc {
                gl.uniform_1_i32(Some(loc), 0);
            }
            if let Some(loc) = &self.tex1_loc {
                gl.uniform_1_i32(Some(loc), 1);
            }
            if let Some(loc) = &self.has_tex1_loc {
                gl.uniform_1_f32(Some(loc), if has_tex1 { 1.0 } else { 0.0 });
            }
            if let Some(loc) = &self.image_lut_loc {
                gl.uniform_1_i32(Some(loc), 2);
            }
            if let Some(loc) = &self.fade_rect_loc_image {
                gl.uniform_4_f32(Some(loc), ef.rect[0], ef.rect[1], ef.rect[2], ef.rect[3]);
            }
            if let Some(loc) = &self.fade_widths_loc_image {
                gl.uniform_4_f32(
                    Some(loc),
                    ef.widths[0],
                    ef.widths[1],
                    ef.widths[2],
                    ef.widths[3],
                );
            }
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(tex0));
            gl.active_texture(glow::TEXTURE1);
            gl.bind_texture(glow::TEXTURE_2D, Some(tex1));
            gl.active_texture(glow::TEXTURE2);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.corner_lut_tex));
            gl.bind_vertex_array(Some(self.vao));
            self.bind_instance_attribs(
                self.image_bo,
                as_bytes(images),
                std::mem::size_of::<ImageInstance>(),
                1,
                7,
            );
            gl.draw_arrays_instanced(
                glow::TRIANGLE_STRIP,
                0,
                4,
                i32::try_from(images.len()).unwrap_or(0),
            );
            gl.bind_vertex_array(None);
        }
    }

    unsafe fn bind_instance_attribs(
        &self,
        bo: glow::Buffer,
        data: &[u8],
        stride: usize,
        first_attr: u32,
        count: u32,
    ) {
        let gl = &self.gl;
        let stride_i32 = i32::try_from(stride).unwrap_or(0);
        // SAFETY: `self.vao` is bound; `stride` and `count` match the instance
        // struct (`QuadInstance` / `GlyphInstance` / `ImageInstance`).
        unsafe {
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(bo));
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, data, glow::STREAM_DRAW);
            for i in 0..count {
                let attr = first_attr + i;
                let offset = i.cast_signed() * 16;
                gl.enable_vertex_attrib_array(attr);
                gl.vertex_attrib_pointer_f32(attr, 4, glow::FLOAT, false, stride_i32, offset);
                gl.vertex_attrib_divisor(attr, 1);
            }
            // Disable any higher instance attribs left enabled by a previous
            // program (e.g. IMAGE's 6..=7) so stale buffer references can never
            // trip draw-time validation or leak into another pass's VAO state.
            for attr in (first_attr + count)..=MAX_INSTANCE_ATTR {
                gl.disable_vertex_attrib_array(attr);
            }
        }
    }
}

/// Intersect two optional scissor rects (top-left origin physical px).
fn intersect_scissor(a: Option<ScissorRect>, b: Option<ScissorRect>) -> Option<ScissorRect> {
    match (a, b) {
        (Some(a), Some(b)) => {
            let x0 = a.x.max(b.x);
            let y0 = a.y.max(b.y);
            let x1 = a.x.saturating_add(a.w).min(b.x.saturating_add(b.w));
            let y1 = a.y.saturating_add(a.h).min(b.y.saturating_add(b.h));
            Some(ScissorRect {
                x: x0,
                y: y0,
                w: (x1 - x0).max(0),
                h: (y1 - y0).max(0),
            })
        }
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

impl Drop for Gpu {
    fn drop(&mut self) {
        // SAFETY: Best-effort cleanup of GL objects when the engine shuts down.
        unsafe {
            self.layer_cache.clear(&self.gl);
            for (_, entry) in self.image_textures.drain() {
                self.gl.delete_texture(entry.tex);
            }
            self.gl.delete_texture(self.corner_lut_tex);
            self.gl.delete_buffer(self.unit_vbo);
            self.gl.delete_buffer(self.quad_bo);
            self.gl.delete_buffer(self.glyph_bo);
            self.gl.delete_buffer(self.image_bo);
            self.gl.delete_vertex_array(self.vao);
            self.gl.delete_program(self.quad_prog);
            self.gl.delete_program(self.glyph_prog);
            self.gl.delete_program(self.image_prog);
        }
    }
}

unsafe fn compile_program(
    gl: &glow::Context,
    vs_src: &str,
    fs_src: &str,
) -> Result<glow::Program, EngineError> {
    // SAFETY: Caller holds a current GLES3 context.
    unsafe {
        let compile = |kind, src| -> Result<glow::Shader, EngineError> {
            let shader = gl
                .create_shader(kind)
                .map_err(|e| EngineError::Gpu(format!("create_shader: {e}")))?;
            gl.shader_source(shader, src);
            gl.compile_shader(shader);
            if !gl.get_shader_compile_status(shader) {
                let log = gl.get_shader_info_log(shader);
                gl.delete_shader(shader);
                return Err(EngineError::Gpu(format!("shader compile failed: {log}")));
            }
            Ok(shader)
        };
        let vs = compile(glow::VERTEX_SHADER, vs_src)?;
        let fs = compile(glow::FRAGMENT_SHADER, fs_src)?;
        let prog = gl
            .create_program()
            .map_err(|e| EngineError::Gpu(format!("create_program: {e}")))?;
        gl.attach_shader(prog, vs);
        gl.attach_shader(prog, fs);
        gl.link_program(prog);
        let linked = gl.get_program_link_status(prog);
        gl.delete_shader(vs);
        gl.delete_shader(fs);
        if !linked {
            let log = gl.get_program_info_log(prog);
            gl.delete_program(prog);
            return Err(EngineError::Gpu(format!("program link failed: {log}")));
        }
        Ok(prog)
    }
}

fn as_bytes<T>(slice: &[T]) -> &[u8] {
    // SAFETY: Used strictly for `#[repr(C)]` plain-old-data f32 structs/arrays with no padding.
    unsafe { std::slice::from_raw_parts(slice.as_ptr().cast::<u8>(), std::mem::size_of_val(slice)) }
}
