//! Text shaping (`cosmic-text`) and rasterized glyph atlas (`swash`).
//!
//! Features generation-based LRU eviction / atlas reset so long-running desktop
//! sessions and animated panels never exhaust glyph atlas space, plus a bounded
//! shaped-run cache and cursor-offset measurement for `TextInput`.

#![allow(unsafe_code)] // GL texture creation/upload via `glow::HasContext`

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use cosmic_text::{
    Attrs, Buffer, CacheKey, Family, FontSystem, Metrics, Shaping, SwashCache, SwashContent, Weight,
};
use etagere::{AllocId, AtlasAllocator, size2};
use glow::HasContext;

use crate::gpu::GlyphInstance;
use crate::style::{FontSpec, Rgba};

const MASK_ATLAS_SIZE: u32 = 1024;
const BITMAP_ATLAS_SIZE: u32 = 512;
const ATLAS_PAD: u32 = 1;
const MAX_CACHED_RUNS: usize = 512;

/// Metrics returned for a shaped text run.
#[derive(Debug, Clone, Copy, Default)]
pub struct TextMetrics {
    pub width: f32,
    pub height: f32,
}

struct CachedRun {
    buffer: Buffer,
    width: f32,
    height: f32,
    last_frame: u64,
}

impl std::fmt::Debug for CachedRun {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CachedRun")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("last_frame", &self.last_frame)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy)]
struct GlyphSlot {
    alloc_id: AllocId,
    texel: [u32; 4],
    left: i32,
    top: i32,
    is_color: bool,
    last_frame: u64,
}

struct GlyphAtlas {
    size: u32,
    bpp: usize,
    tex: Option<glow::Texture>,
    pixels: Vec<u8>,
    allocator: AtlasAllocator,
    slots: HashMap<CacheKey, GlyphSlot>,
    dirty: Vec<[u32; 4]>,
    full_upload_needed: bool,
    /// Number of times this atlas reset after filling up (useful for telemetry/tests).
    resets: u64,
}

impl std::fmt::Debug for GlyphAtlas {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GlyphAtlas")
            .field("size", &self.size)
            .field("bpp", &self.bpp)
            .field("tex", &self.tex)
            .field("slots", &self.slots.len())
            .field("dirty", &self.dirty.len())
            .field("full_upload_needed", &self.full_upload_needed)
            .field("resets", &self.resets)
            .finish_non_exhaustive()
    }
}

impl GlyphAtlas {
    fn new(size: u32, bpp: usize) -> Self {
        let size_i32 = size.cast_signed();
        Self {
            size,
            bpp,
            tex: None,
            pixels: vec![0; (size * size) as usize * bpp],
            allocator: AtlasAllocator::new(size2(size_i32, size_i32)),
            slots: HashMap::new(),
            dirty: Vec::new(),
            full_upload_needed: true,
            resets: 0,
        }
    }

    fn alloc(&mut self, w: u32, h: u32) -> Option<(AllocId, [u32; 4])> {
        if w == 0 || h == 0 || w + 2 * ATLAS_PAD > self.size || h + 2 * ATLAS_PAD > self.size {
            return None;
        }
        let alloc_w = (w + 2 * ATLAS_PAD).cast_signed();
        let alloc_h = (h + 2 * ATLAS_PAD).cast_signed();
        let alloc = self.allocator.allocate(size2(alloc_w, alloc_h))?;
        let rect = [
            alloc.rectangle.min.x.cast_unsigned() + ATLAS_PAD,
            alloc.rectangle.min.y.cast_unsigned() + ATLAS_PAD,
            w,
            h,
        ];
        self.dirty.push(rect);
        Some((alloc.id, rect))
    }

    /// Allocate space for a glyph, trying targeted LRU eviction of cold slots
    /// (`last_frame < current_frame`) in batches before falling back to a full
    /// atlas reset.
    fn alloc_with_eviction(
        &mut self,
        w: u32,
        h: u32,
        current_frame: u64,
        is_color: bool,
    ) -> Option<(AllocId, [u32; 4])> {
        if let Some(res) = self.alloc(w, h) {
            return Some(res);
        }

        // Targeted LRU eviction: collect cold slots sorted oldest-first and evict
        // in 25% batches until allocation succeeds or no cold slots remain.
        let mut cold: Vec<(CacheKey, u64)> = self
            .slots
            .iter()
            .filter(|(_, slot)| slot.last_frame < current_frame)
            .map(|(&key, slot)| (key, slot.last_frame))
            .collect();
        cold.sort_unstable_by_key(|&(_, last_frame)| last_frame);

        if !cold.is_empty() {
            let batch_size = (cold.len() / 4).max(1);
            let mut evicted = 0usize;
            for chunk in cold.chunks(batch_size) {
                for &(key, _) in chunk {
                    if let Some(slot) = self.slots.remove(&key) {
                        self.allocator.deallocate(slot.alloc_id);
                        evicted += 1;
                    }
                }
                if let Some(res) = self.alloc(w, h) {
                    tracing::debug!(
                        is_color,
                        evicted,
                        remaining = self.slots.len(),
                        "evicted cold glyphs from atlas"
                    );
                    return Some(res);
                }
            }
        }

        // Fallback: fragmentation or every slot was used in `current_frame`.
        tracing::debug!(
            is_color,
            resets = self.resets + 1,
            "glyph atlas full; resetting allocator"
        );
        self.reset();
        self.alloc(w, h)
    }

    /// Clear all slots and reset the atlas allocator. Keeps the GL texture
    /// allocation intact and marks it for a clean re-upload.
    fn reset(&mut self) {
        self.pixels.fill(0);
        self.allocator.clear();
        self.slots.clear();
        self.dirty.clear();
        self.full_upload_needed = true;
        self.resets += 1;
    }

    fn write_pixels(&mut self, rect: [u32; 4], data: &[u8], is_color: bool) {
        let [rx, ry, rw, rh] = rect;
        let aw = self.size as usize;
        if is_color {
            for row in 0..rh {
                for col in 0..rw {
                    let si = ((row * rw + col) * 4) as usize;
                    let di = (((ry + row) as usize) * aw + (rx + col) as usize) * 4;
                    // swash Color content is BGRA -> convert to RGBA
                    self.pixels[di] = data[si + 2];
                    self.pixels[di + 1] = data[si + 1];
                    self.pixels[di + 2] = data[si];
                    self.pixels[di + 3] = data[si + 3];
                }
            }
        } else {
            for row in 0..rh {
                let si = (row * rw) as usize;
                let di = ((ry + row) as usize) * aw + rx as usize;
                self.pixels[di..di + rw as usize].copy_from_slice(&data[si..si + rw as usize]);
            }
        }
    }

    fn upload(&mut self, gl: &glow::Context, unit: u32) {
        let (internal, format) = if self.bpp == 1 {
            (glow::R8.cast_signed(), glow::RED)
        } else {
            (glow::RGBA8.cast_signed(), glow::RGBA)
        };
        let size_i32 = self.size.cast_signed();
        // SAFETY: Valid GLES3 context is current on the calling thread.
        unsafe {
            let tex = *self.tex.get_or_insert_with(|| {
                let t = gl.create_texture().expect("create glyph atlas texture");
                gl.active_texture(unit);
                gl.bind_texture(glow::TEXTURE_2D, Some(t));
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
                gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
                gl.tex_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    internal,
                    size_i32,
                    size_i32,
                    0,
                    format,
                    glow::UNSIGNED_BYTE,
                    glow::PixelUnpackData::Slice(Some(&self.pixels)),
                );
                self.full_upload_needed = false;
                t
            });

            gl.active_texture(unit);
            gl.bind_texture(glow::TEXTURE_2D, Some(tex));
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);

            if self.full_upload_needed {
                gl.tex_sub_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    0,
                    0,
                    size_i32,
                    size_i32,
                    format,
                    glow::UNSIGNED_BYTE,
                    glow::PixelUnpackData::Slice(Some(&self.pixels)),
                );
                self.full_upload_needed = false;
                self.dirty.clear();
                return;
            }

            for [x, y, w, h] in self.dirty.drain(..) {
                let bpp = self.bpp;
                let row_start = ((y * self.size + x) as usize) * bpp;
                for row in 0..h {
                    let off = row_start + (row * self.size) as usize * bpp;
                    let end = off + (w as usize) * bpp;
                    gl.tex_sub_image_2d(
                        glow::TEXTURE_2D,
                        0,
                        x.cast_signed(),
                        (y + row).cast_signed(),
                        w.cast_signed(),
                        1,
                        format,
                        glow::UNSIGNED_BYTE,
                        glow::PixelUnpackData::Slice(Some(&self.pixels[off..end])),
                    );
                }
            }
        }
    }
}

/// Handle handed to [`crate::gpu::Gpu::render`] for uploading dirty glyph tiles.
pub struct AtlasUpload<'a> {
    text: &'a mut TextEngine,
}

impl std::fmt::Debug for AtlasUpload<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.pad("AtlasUpload { .. }")
    }
}

impl AtlasUpload<'_> {
    pub fn mask_tex(&self) -> Option<glow::Texture> {
        self.text.mask.tex
    }

    pub fn bitmap_tex(&self) -> Option<glow::Texture> {
        self.text.bitmap.tex
    }

    pub fn upload(&mut self, gl: &glow::Context) {
        self.text.mask.upload(gl, glow::TEXTURE0);
        self.text.bitmap.upload(gl, glow::TEXTURE1);
    }
}

/// Text shaping, font database, and LRU/resettable glyph atlas.
#[derive(Debug)]
pub struct TextEngine {
    font_system: FontSystem,
    swash: SwashCache,
    runs: HashMap<u64, CachedRun>,
    mask: GlyphAtlas,
    bitmap: GlyphAtlas,
    frame_gen: u64,
}

impl Default for TextEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl TextEngine {
    /// Create a text engine preloaded with Triton's embedded fonts (Jost,
    /// Google Sans Flex, JetBrains Mono, Triton Symbols Outlined & Filled).
    pub fn new() -> Self {
        let mut font_system = FontSystem::new();
        for data in triton_design::fonts::ALL {
            font_system.db_mut().load_font_data(data.to_vec());
        }
        Self {
            font_system,
            swash: SwashCache::new(),
            runs: HashMap::new(),
            mask: GlyphAtlas::new(MASK_ATLAS_SIZE, 1),
            bitmap: GlyphAtlas::new(BITMAP_ATLAS_SIZE, 4),
            frame_gen: 1,
        }
    }

    /// Advance the frame generation counter and trim cold shaped-run entries.
    pub fn begin_frame(&mut self) {
        self.frame_gen = self.frame_gen.wrapping_add(1).max(1);
        if self.runs.len() > MAX_CACHED_RUNS {
            let cutoff = self.frame_gen.saturating_sub(120);
            self.runs.retain(|_, r| r.last_frame >= cutoff);
            if self.runs.len() > MAX_CACHED_RUNS {
                self.runs.clear();
            }
        }
    }

    /// Borrow an [`AtlasUpload`] view for the GPU renderer.
    pub fn atlas_upload(&mut self) -> AtlasUpload<'_> {
        AtlasUpload { text: self }
    }

    /// Number of times the mask glyph atlas compacted/reset.
    pub fn mask_atlas_resets(&self) -> u64 {
        self.mask.resets
    }

    /// Number of cached glyphs in the mask atlas.
    pub fn mask_glyph_count(&self) -> usize {
        self.mask.slots.len()
    }

    /// Measure `text` at `spec` within optional `max_width` (logical px).
    pub fn measure(&mut self, text: &str, spec: &FontSpec, max_width: Option<f32>) -> TextMetrics {
        let prepared;
        let s = if spec.uppercase {
            prepared = text.to_uppercase();
            prepared.as_str()
        } else {
            text
        };
        let key = self.ensure_shaped(s, spec, max_width);
        self.runs
            .get(&key)
            .map_or(TextMetrics::default(), |r| TextMetrics {
                width: r.width,
                height: r.height,
            })
    }

    /// Measure the x-advance of the prefix `text[..byte_index]` in logical px.
    /// Used by `TextInput` to place the text caret accurately.
    pub fn measure_caret_x(&mut self, text: &str, byte_index: usize, spec: &FontSpec) -> f32 {
        let clamped = byte_index.min(text.len());
        let mut idx = clamped;
        while idx > 0 && !text.is_char_boundary(idx) {
            idx -= 1;
        }
        if idx == 0 {
            return 0.0;
        }
        self.measure(&text[..idx], spec, None).width
    }

    /// Emit [`GlyphInstance`]s for `text` at logical `(ox, oy)` (top-left of the
    /// text box), scaled to buffer pixels by `scale` and `opacity`.
    ///
    /// Returns `true` if every glyph fit in the atlas, or `false` if an atlas
    /// reset occurred mid-frame (in which case callers should re-emit the frame
    /// cleanly — though because we reset+retry before drawing, the current run
    /// still gets valid slots).
    #[allow(clippy::too_many_arguments)]
    pub fn layout_glyphs(
        &mut self,
        text: &str,
        spec: &FontSpec,
        max_width: Option<f32>,
        ox: f32,
        oy: f32,
        scale: f32,
        color: Rgba,
        opacity: f32,
        out: &mut Vec<GlyphInstance>,
    ) -> TextMetrics {
        if text.is_empty() {
            return TextMetrics::default();
        }
        let prepared;
        let s = if spec.uppercase {
            prepared = text.to_uppercase();
            prepared.as_str()
        } else {
            text
        };
        let key = self.ensure_shaped(s, spec, max_width);
        let Some(run) = self.runs.get(&key) else {
            return TextMetrics::default();
        };
        let metrics = TextMetrics {
            width: run.width,
            height: run.height,
        };

        // Quantize the rasterization scale slightly (0.125 steps) during animated
        // scale transitions so springs don't thrash the glyph atlas, while exact
        // glyph positions still use true `scale`.
        let raster_scale = ((scale * 8.0).round() / 8.0).max(0.5);
        let pos_ratio = scale / raster_scale;

        let mut physical = Vec::new();
        for layout_run in run.buffer.layout_runs() {
            for g in layout_run.glyphs {
                let p = g.physical((0.0, 0.0), raster_scale);
                physical.push((p, layout_run.line_y, g.color_opt));
            }
        }

        let color_arr = color.to_array();
        for (p, line_y, color_opt) in physical {
            let Some(slot) = self.acquire_slot(p.cache_key) else {
                continue;
            };
            let uv_scale = if slot.is_color {
                1.0 / BITMAP_ATLAS_SIZE as f32
            } else {
                1.0 / MASK_ATLAS_SIZE as f32
            };
            let [tx, ty, tw, th] = slot.texel;
            let gx = (ox * scale + (p.x as f32 + slot.left as f32) * pos_ratio).round();
            let gy =
                (oy * scale + line_y * scale + (p.y as f32 - slot.top as f32) * pos_ratio).round();
            out.push(GlyphInstance {
                dst: [gx, gy, tw as f32 * pos_ratio, th as f32 * pos_ratio],
                uv: [
                    tx as f32 * uv_scale,
                    ty as f32 * uv_scale,
                    tw as f32 * uv_scale,
                    th as f32 * uv_scale,
                ],
                color: color_opt.map_or(color_arr, |c| {
                    [
                        f32::from(c.r()) / 255.0,
                        f32::from(c.g()) / 255.0,
                        f32::from(c.b()) / 255.0,
                        f32::from(c.a()) / 255.0 * color_arr[3],
                    ]
                }),
                misc: [f32::from(u8::from(slot.is_color)), opacity, 0.0, 0.0],
            });
        }
        metrics
    }

    fn ensure_shaped(&mut self, text: &str, spec: &FontSpec, max_width: Option<f32>) -> u64 {
        let key = run_hash(text, spec, max_width);
        if let Some(run) = self.runs.get_mut(&key) {
            run.last_frame = self.frame_gen;
            return key;
        }
        let metrics = Metrics::new(spec.size.max(1.0), spec.line_height.max(1.0));
        let mut buffer = Buffer::new(&mut self.font_system, metrics);
        buffer.set_size(&mut self.font_system, max_width, None);
        let mut attrs = Attrs::new()
            .family(Family::Name(spec.family))
            .weight(Weight(spec.weight));
        if spec.letter_spacing.abs() > 0.01 {
            attrs = attrs.letter_spacing(spec.letter_spacing / spec.size.max(1.0));
        }
        buffer.set_text(&mut self.font_system, text, &attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut self.font_system, false);

        let mut width = 0.0f32;
        let mut height = 0.0f32;
        let mut line_count = 0usize;
        for run in buffer.layout_runs() {
            width = width.max(run.line_w);
            height += run.line_height;
            line_count += 1;
        }
        if line_count == 0 {
            height = spec.line_height;
        }

        self.runs.insert(
            key,
            CachedRun {
                buffer,
                width,
                height,
                last_frame: self.frame_gen,
            },
        );
        key
    }

    fn acquire_slot(&mut self, key: CacheKey) -> Option<GlyphSlot> {
        if let Some(slot) = self.mask.slots.get_mut(&key) {
            slot.last_frame = self.frame_gen;
            return Some(*slot);
        }
        if let Some(slot) = self.bitmap.slots.get_mut(&key) {
            slot.last_frame = self.frame_gen;
            return Some(*slot);
        }

        let image = self.swash.get_image_uncached(&mut self.font_system, key)?;
        let w = image.placement.width;
        let h = image.placement.height;
        if w == 0 || h == 0 {
            return None;
        }
        let is_color = match image.content {
            SwashContent::Color => true,
            SwashContent::Mask => false,
            SwashContent::SubpixelMask => return None,
        };
        let atlas = if is_color {
            &mut self.bitmap
        } else {
            &mut self.mask
        };

        let (alloc_id, texel) = atlas.alloc_with_eviction(w, h, self.frame_gen, is_color)?;

        atlas.write_pixels(texel, &image.data, is_color);
        let slot = GlyphSlot {
            alloc_id,
            texel,
            left: image.placement.left,
            top: image.placement.top,
            is_color,
            last_frame: self.frame_gen,
        };
        atlas.slots.insert(key, slot);
        Some(slot)
    }
}

fn run_hash(text: &str, spec: &FontSpec, max_width: Option<f32>) -> u64 {
    let mut h = DefaultHasher::new();
    text.hash(&mut h);
    spec.family.hash(&mut h);
    spec.size.to_bits().hash(&mut h);
    spec.line_height.to_bits().hash(&mut h);
    spec.weight.hash(&mut h);
    spec.letter_spacing.to_bits().hash(&mut h);
    spec.uppercase.hash(&mut h);
    max_width.map(f32::to_bits).hash(&mut h);
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic_key(id: u16) -> CacheKey {
        CacheKey::new(
            cosmic_text::fontdb::ID::dummy(),
            id,
            16.0,
            (0.0, 0.0),
            Weight::NORMAL,
            cosmic_text::CacheKeyFlags::empty(),
        )
        .0
    }

    #[test]
    fn lru_eviction_frees_cold_glyphs_without_reset() {
        let mut atlas = GlyphAtlas::new(64, 1);

        // Fill the atlas across frames 1..=N so all existing glyphs are cold in frame 100.
        let mut initial_count = 0u16;
        while let Some((alloc_id, texel)) = atlas.alloc(16, 16) {
            let key = synthetic_key(initial_count);
            atlas.slots.insert(
                key,
                GlyphSlot {
                    alloc_id,
                    texel,
                    left: 0,
                    top: 0,
                    is_color: false,
                    last_frame: u64::from(initial_count) + 1,
                },
            );
            initial_count += 1;
        }
        assert!(initial_count > 0, "atlas should fit at least one glyph");

        // Advance to a new frame: all existing glyphs are cold (last_frame < 100).
        // Allocating new glyphs should evict cold slots without triggering a full reset.
        let current_frame = 100;
        let (alloc_id, texel) = atlas
            .alloc_with_eviction(16, 16, current_frame, false)
            .expect("eviction should free space for a new glyph");
        let new_key = synthetic_key(1000);
        atlas.slots.insert(
            new_key,
            GlyphSlot {
                alloc_id,
                texel,
                left: 0,
                top: 0,
                is_color: false,
                last_frame: current_frame,
            },
        );

        assert_eq!(atlas.resets, 0, "cold eviction must not reset the atlas");
        // The oldest glyph (frame 1) must have been evicted.
        assert!(!atlas.slots.contains_key(&synthetic_key(0)));
        // The newly allocated glyph must be present.
        assert!(atlas.slots.contains_key(&new_key));
    }

    #[test]
    fn hot_atlas_falls_back_to_reset() {
        let mut atlas = GlyphAtlas::new(64, 1);
        let current_frame = 10;

        // Fill the atlas with glyphs all hot in `current_frame`.
        let mut count = 0u16;
        while let Some((alloc_id, texel)) = atlas.alloc(16, 16) {
            let key = synthetic_key(count);
            atlas.slots.insert(
                key,
                GlyphSlot {
                    alloc_id,
                    texel,
                    left: 0,
                    top: 0,
                    is_color: false,
                    last_frame: current_frame,
                },
            );
            count += 1;
        }
        assert!(count > 0);
        assert_eq!(atlas.resets, 0);

        // Requesting another glyph in the same frame has no cold slots to evict,
        // so it must fall back cleanly to `reset()`.
        let (alloc_id, texel) = atlas
            .alloc_with_eviction(16, 16, current_frame, false)
            .expect("reset fallback should allocate the glyph");
        atlas.slots.insert(
            synthetic_key(999),
            GlyphSlot {
                alloc_id,
                texel,
                left: 0,
                top: 0,
                is_color: false,
                last_frame: current_frame,
            },
        );
        assert!(atlas.resets >= 1);
        assert_eq!(atlas.slots.len(), 1);
    }

    #[test]
    fn measure_and_caret_advance() {
        let mut text = TextEngine::new();
        let spec = FontSpec::ui(16.0);
        let full = text.measure("Triton", &spec, None);
        assert!(full.width > 10.0);
        let mid = text.measure_caret_x("Triton", 3, &spec);
        assert!(mid > 0.0 && mid < full.width);
        assert_eq!(text.measure_caret_x("Triton", 0, &spec), 0.0);
    }
}
