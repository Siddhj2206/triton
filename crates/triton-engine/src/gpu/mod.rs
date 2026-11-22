//! GLES3 (`glow`) SDF uber-quad + instanced glyph renderer.
//!
//! All primitives render in physical (buffer) pixel space with premultiplied
//! alpha compositing and scissor-aware batching so clipped containers (`ScrollArea`,
//! overflow bounds) preserve exact draw order without extra render passes.

#![allow(unsafe_code)] // GLES3 calls via `glow::HasContext`

use glow::HasContext;

use crate::error::EngineError;
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
flat out vec4 v_radii;
flat out vec4 v_fill;
flat out vec4 v_border;
flat out vec4 v_misc;

void main() {
    vec2 px = i_dst.xy + a_pos * i_dst.zw;
    v_local = a_pos * i_dst.zw;
    v_half = i_dst.zw * 0.5;
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

in vec2 v_local;
in vec2 v_half;
flat in vec4 v_radii;
flat in vec4 v_fill;
flat in vec4 v_border;
flat in vec4 v_misc;

out vec4 frag;

// Standard circular rounded-rect SDF.
float sd_round(vec2 p, vec2 b, vec4 r) {
    float rr = (p.y < 0.0) ? ((p.x < 0.0) ? r.x : r.y)
                           : ((p.x < 0.0) ? r.w : r.z);
    rr = clamp(rr, 0.0, min(b.x, b.y));
    vec2 q = abs(p) - b + rr;
    return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - rr;
}

// Continuous smooth-corner (superellipse n = 4) SDF with exact straight edges.
float sd_smooth(vec2 p, vec2 b, vec4 r) {
    float rr = (p.y < 0.0) ? ((p.x < 0.0) ? r.x : r.y)
                           : ((p.x < 0.0) ? r.w : r.z);
    rr = clamp(rr, 0.0, min(b.x, b.y));
    if (rr <= 0.001) {
        vec2 q0 = abs(p) - b;
        return length(max(q0, 0.0)) + min(max(q0.x, q0.y), 0.0);
    }
    vec2 q = abs(p) - b + rr;
    if (q.x > 0.0 && q.y > 0.0) {
        vec2 e = q / rr;
        vec2 e2 = e * e;
        float norm4 = pow(dot(e2, e2), 0.25);
        return (norm4 - 1.0) * rr;
    }
    return max(q.x, q.y) - rr;
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

    // Kind 2: Soft SDF drop shadow. `i_dst` is already expanded by 2 * blur,
    // so the inner caster half-size is `v_half - param`.
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

    float a = col.a * cover * opacity;
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
flat out vec4 v_color;
flat out vec4 v_misc;

void main() {
    vec2 px = i_dst.xy + a_pos * i_dst.zw;
    v_uv = i_uv.xy + a_pos * i_uv.zw;
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

in vec2 v_uv;
flat in vec4 v_color;
flat in vec4 v_misc;

out vec4 frag;

void main() {
    if (v_misc.x > 0.5) {
        vec4 c = texture(u_bitmap, v_uv);
        float a = c.a * v_misc.y;
        frag = vec4(c.rgb * a, a);
    } else {
        float m = texture(u_mask, v_uv).r;
        float a = v_color.a * m * v_misc.y;
        frag = vec4(v_color.rgb * a, a);
    }
}
";

/// Primitive kinds for [`QuadInstance::misc`]`[0]`.
pub const KIND_FILL: f32 = 0.0;
pub const KIND_TOP_HIGHLIGHT: f32 = 1.0;
pub const KIND_SHADOW: f32 = 2.0;

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

/// Scissor rectangle in physical buffer pixels, origin top-left.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScissorRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// A batch of primitives sharing one scissor clip rect, preserving z-order.
#[derive(Debug, Clone)]
pub enum DrawBatch {
    Quads {
        scissor: Option<ScissorRect>,
        instances: Vec<QuadInstance>,
    },
    Glyphs {
        scissor: Option<ScissorRect>,
        instances: Vec<GlyphInstance>,
    },
}

/// Ordered command buffer of scissor-aware batches for a single surface frame.
#[derive(Debug, Default)]
pub struct DrawList {
    pub batches: Vec<DrawBatch>,
}

impl DrawList {
    pub fn clear(&mut self) {
        self.batches.clear();
    }

    pub fn push_quad(&mut self, scissor: Option<ScissorRect>, quad: QuadInstance) {
        if let Some(DrawBatch::Quads {
            scissor: last_sc,
            instances,
        }) = self.batches.last_mut()
            && *last_sc == scissor
        {
            instances.push(quad);
            return;
        }
        self.batches.push(DrawBatch::Quads {
            scissor,
            instances: vec![quad],
        });
    }

    pub fn push_glyphs(&mut self, scissor: Option<ScissorRect>, glyphs: &[GlyphInstance]) {
        if glyphs.is_empty() {
            return;
        }
        if let Some(DrawBatch::Glyphs {
            scissor: last_sc,
            instances,
        }) = self.batches.last_mut()
            && *last_sc == scissor
        {
            instances.extend_from_slice(glyphs);
            return;
        }
        self.batches.push(DrawBatch::Glyphs {
            scissor,
            instances: glyphs.to_vec(),
        });
    }

    pub fn quad_count(&self) -> usize {
        self.batches
            .iter()
            .map(|b| match b {
                DrawBatch::Quads { instances, .. } => instances.len(),
                DrawBatch::Glyphs { .. } => 0,
            })
            .sum()
    }

    pub fn glyph_count(&self) -> usize {
        self.batches
            .iter()
            .map(|b| match b {
                DrawBatch::Quads { .. } => 0,
                DrawBatch::Glyphs { instances, .. } => instances.len(),
            })
            .sum()
    }
}

/// GLES3 shader programs and streaming instance buffers.
#[derive(Debug)]
pub struct Gpu {
    gl: glow::Context,
    quad_prog: glow::Program,
    glyph_prog: glow::Program,
    vao: glow::VertexArray,
    unit_vbo: glow::Buffer,
    quad_bo: glow::Buffer,
    glyph_bo: glow::Buffer,
    view_loc_quad: Option<glow::UniformLocation>,
    view_loc_glyph: Option<glow::UniformLocation>,
    mask_loc: Option<glow::UniformLocation>,
    bitmap_loc: Option<glow::UniformLocation>,
}

impl Gpu {
    /// Compile GLES3 shaders and allocate unit quad + instance VBOs.
    /// Requires a current GLES3 context on the calling thread.
    pub fn new(gl: glow::Context) -> Result<Self, EngineError> {
        // SAFETY: Valid GLES3 context is current; all objects are checked for errors.
        unsafe {
            let quad_prog = compile_program(&gl, QUAD_VS, QUAD_FS)?;
            let glyph_prog = compile_program(&gl, GLYPH_VS, GLYPH_FS)?;
            let vao = gl
                .create_vertex_array()
                .map_err(|e| EngineError::Gpu(format!("create_vertex_array: {e}")))?;
            let quad_bo = gl
                .create_buffer()
                .map_err(|e| EngineError::Gpu(format!("create quad_bo: {e}")))?;
            let glyph_bo = gl
                .create_buffer()
                .map_err(|e| EngineError::Gpu(format!("create glyph_bo: {e}")))?;
            let unit_vbo = gl
                .create_buffer()
                .map_err(|e| EngineError::Gpu(format!("create unit_vbo: {e}")))?;

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
                view_loc_glyph: gl.get_uniform_location(glyph_prog, "u_view"),
                mask_loc: gl.get_uniform_location(glyph_prog, "u_mask"),
                bitmap_loc: gl.get_uniform_location(glyph_prog, "u_bitmap"),
                gl,
                quad_prog,
                glyph_prog,
                vao,
                unit_vbo,
                quad_bo,
                glyph_bo,
            })
        }
    }

    /// Execute a full frame's [`DrawList`] into the currently bound EGL surface.
    pub fn render(&self, w: u32, h: u32, draw_list: &DrawList, atlas: &mut AtlasUpload<'_>) {
        let wi = i32::try_from(w.max(1)).unwrap_or(1);
        let hi = i32::try_from(h.max(1)).unwrap_or(1);
        // SAFETY: Valid GLES3 context is current for this surface.
        unsafe {
            let gl = &self.gl;
            gl.viewport(0, 0, wi, hi);
            gl.disable(glow::DEPTH_TEST);
            gl.disable(glow::SCISSOR_TEST);
            gl.enable(glow::BLEND);
            gl.blend_func(glow::ONE, glow::ONE_MINUS_SRC_ALPHA);
            gl.clear_color(0.0, 0.0, 0.0, 0.0);
            gl.clear(glow::COLOR_BUFFER_BIT);

            // Upload any pending glyph updates once before executing batches.
            atlas.upload(gl);

            for batch in &draw_list.batches {
                match batch {
                    DrawBatch::Quads { scissor, instances } if !instances.is_empty() => {
                        self.apply_scissor(hi, *scissor);
                        self.draw_quads_inner(w, h, instances);
                    }
                    DrawBatch::Glyphs { scissor, instances } if !instances.is_empty() => {
                        self.apply_scissor(hi, *scissor);
                        self.draw_glyphs_inner(w, h, atlas, instances);
                    }
                    _ => {}
                }
            }
            gl.disable(glow::SCISSOR_TEST);
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

    unsafe fn draw_quads_inner(&self, w: u32, h: u32, quads: &[QuadInstance]) {
        let gl = &self.gl;
        // SAFETY: Caller holds a current GLES3 context and `quads` is non-empty.
        unsafe {
            gl.use_program(Some(self.quad_prog));
            if let Some(loc) = &self.view_loc_quad {
                gl.uniform_2_f32(Some(loc), w as f32, h as f32);
            }
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
        atlas: &AtlasUpload<'_>,
        glyphs: &[GlyphInstance],
    ) {
        let gl = &self.gl;
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
        // SAFETY: `self.vao` is bound; `stride` and `count` match `QuadInstance` / `GlyphInstance`.
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
        }
    }
}

impl Drop for Gpu {
    fn drop(&mut self) {
        // SAFETY: Best-effort cleanup of GL objects when the engine shuts down.
        unsafe {
            self.gl.delete_buffer(self.unit_vbo);
            self.gl.delete_buffer(self.quad_bo);
            self.gl.delete_buffer(self.glyph_bo);
            self.gl.delete_vertex_array(self.vao);
            self.gl.delete_program(self.quad_prog);
            self.gl.delete_program(self.glyph_prog);
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
