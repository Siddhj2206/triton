//! Offscreen FBO layer cache for `Element::layer(key)` subtrees.
//!
//! Heavy animated subtrees (Launcher, Quick Settings, Notifications, Wallpaper
//! crossfade) can be rasterized once into an `RGBA8` offscreen framebuffer and
//! composited as a textured quad on subsequent frames while their
//! `content_hash` and dimensions remain unchanged.

#![allow(unsafe_code)] // GLES3 FBO and texture lifecycle calls via `glow::HasContext`

use std::collections::HashMap;

use glow::HasContext;

use crate::error::EngineError;

/// Default number of idle frames before an unused cached layer is evicted.
const DEFAULT_MAX_IDLE_FRAMES: u64 = 60;

/// A single cached offscreen framebuffer and its backing `RGBA8` color texture.
#[derive(Debug)]
pub struct LayerEntry {
    /// GLES3 framebuffer object with `tex` attached at `COLOR_ATTACHMENT0`.
    pub fbo: glow::Framebuffer,
    /// Backing `RGBA8` color texture (`LINEAR`, `CLAMP_TO_EDGE`).
    pub tex: glow::Texture,
    /// Texture width in physical (buffer) pixels.
    pub width: u32,
    /// Texture height in physical (buffer) pixels.
    pub height: u32,
    /// Hash of the cached subtree's draw commands / content.
    pub content_hash: u64,
    /// Frame generation counter when this entry was last accessed.
    pub last_used_frame: u64,
}

/// Cache of offscreen FBO layers keyed by `NodeKey.0`.
#[derive(Debug)]
pub struct LayerCache {
    entries: HashMap<u64, LayerEntry>,
    frame_gen: u64,
    max_idle_frames: u64,
}

impl Default for LayerCache {
    fn default() -> Self {
        Self::new()
    }
}

impl LayerCache {
    /// Creates an empty layer cache with the default idle eviction threshold
    /// (60 frames).
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            frame_gen: 0,
            max_idle_frames: DEFAULT_MAX_IDLE_FRAMES,
        }
    }

    /// Advances the internal frame generation counter. Call once at the start of
    /// each rendered frame.
    pub fn begin_frame(&mut self) {
        self.frame_gen = self.frame_gen.saturating_add(1);
    }

    /// Looks up a cached layer by `key` and marks it as used on the current
    /// frame.
    pub fn get(&mut self, key: u64) -> Option<&LayerEntry> {
        let entry = self.entries.get_mut(&key)?;
        entry.last_used_frame = self.frame_gen;
        Some(entry)
    }

    /// Returns `true` if a cached layer exists for `key` with matching
    /// `width`, `height`, and `content_hash`.
    pub fn is_valid(&self, key: u64, width: u32, height: u32, content_hash: u64) -> bool {
        self.entries.get(&key).is_some_and(|entry| {
            entry.width == width && entry.height == height && entry.content_hash == content_hash
        })
    }

    /// Acquires an offscreen FBO layer for `key` with the requested physical
    /// `width` and `height`, allocating or resizing the backing `RGBA8` texture
    /// as needed.
    ///
    /// On success, the entry's `content_hash` and `last_used_frame` are updated
    /// and a reference to the `LayerEntry` is returned. The default framebuffer
    /// (`None`) is left bound on return.
    ///
    /// # Safety
    ///
    /// The caller must ensure `gl` refers to the active GLES3 context that owns
    /// (or will own) the cached framebuffers and textures.
    pub unsafe fn acquire(
        &mut self,
        gl: &glow::Context,
        key: u64,
        width: u32,
        height: u32,
        content_hash: u64,
    ) -> Result<&LayerEntry, EngineError> {
        let w = width.max(1);
        let h = height.max(1);

        if let Some(entry) = self.entries.get_mut(&key) {
            if entry.width == w && entry.height == h {
                entry.content_hash = content_hash;
                entry.last_used_frame = self.frame_gen;
                return Ok(entry);
            }

            // Dimensions changed: allocate a new texture and re-attach to the
            // existing FBO, then delete the old texture.
            // SAFETY: caller guarantees `gl` is the active GLES3 context.
            let new_tex = unsafe { create_rgba8_texture(gl, w, h) }?;
            // SAFETY: `entry.fbo` and `new_tex` belong to the active `gl` context.
            if let Err(err) = unsafe { attach_and_check_fbo(gl, entry.fbo, new_tex) } {
                // SAFETY: `new_tex` was just created in `gl` and is not stored.
                unsafe { gl.delete_texture(new_tex) };
                return Err(err);
            }
            let old_tex = entry.tex;
            // SAFETY: `old_tex` is no longer attached to `entry.fbo`.
            unsafe { gl.delete_texture(old_tex) };

            entry.tex = new_tex;
            entry.width = w;
            entry.height = h;
            entry.content_hash = content_hash;
            entry.last_used_frame = self.frame_gen;
            return Ok(entry);
        }

        // SAFETY: caller guarantees `gl` is the active GLES3 context.
        let tex = unsafe { create_rgba8_texture(gl, w, h) }?;
        // SAFETY: caller guarantees `gl` is the active GLES3 context.
        let fbo = match unsafe { gl.create_framebuffer() } {
            Ok(fbo) => fbo,
            Err(e) => {
                // SAFETY: `tex` was just created in `gl` and is not stored.
                unsafe { gl.delete_texture(tex) };
                return Err(EngineError::Gpu(format!("create_framebuffer: {e}")));
            }
        };

        // SAFETY: `fbo` and `tex` were just created in the active `gl` context.
        if let Err(err) = unsafe { attach_and_check_fbo(gl, fbo, tex) } {
            // SAFETY: clean up newly created GL objects on failure.
            unsafe {
                gl.delete_framebuffer(fbo);
                gl.delete_texture(tex);
            }
            return Err(err);
        }

        let entry = LayerEntry {
            fbo,
            tex,
            width: w,
            height: h,
            content_hash,
            last_used_frame: self.frame_gen,
        };
        Ok(self.entries.entry(key).or_insert(entry))
    }

    /// Evicts any cached layer that has not been used for more than
    /// `max_idle_frames` frames, deleting its GL framebuffer and texture.
    ///
    /// Returns the number of evicted entries.
    ///
    /// # Safety
    ///
    /// The caller must ensure `gl` is the active GLES3 context that owns the
    /// cached framebuffers and textures.
    pub unsafe fn evict_stale(&mut self, gl: &glow::Context) -> usize {
        let frame_gen = self.frame_gen;
        let max_idle = self.max_idle_frames;
        let mut evicted = 0_usize;

        self.entries.retain(|_, entry| {
            if frame_gen.saturating_sub(entry.last_used_frame) > max_idle {
                // SAFETY: caller guarantees `gl` is the active GLES3 context
                // owning `entry.fbo` and `entry.tex`.
                unsafe {
                    gl.delete_framebuffer(entry.fbo);
                    gl.delete_texture(entry.tex);
                }
                evicted += 1;
                false
            } else {
                true
            }
        });

        evicted
    }

    /// Deletes all cached framebuffers and textures and empties the cache.
    ///
    /// # Safety
    ///
    /// The caller must ensure `gl` is the active GLES3 context that owns the
    /// cached framebuffers and textures.
    pub unsafe fn clear(&mut self, gl: &glow::Context) {
        for (_, entry) in self.entries.drain() {
            // SAFETY: caller guarantees `gl` is the active GLES3 context
            // owning `entry.fbo` and `entry.tex`.
            unsafe {
                gl.delete_framebuffer(entry.fbo);
                gl.delete_texture(entry.tex);
            }
        }
    }

    /// Returns the number of cached layers.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns `true` if no layers are currently cached.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Allocates an uninitialized `RGBA8` 2D texture with `LINEAR` filtering and
/// `CLAMP_TO_EDGE` wrapping.
///
/// # Safety
///
/// `gl` must be an active GLES3 context.
unsafe fn create_rgba8_texture(
    gl: &glow::Context,
    width: u32,
    height: u32,
) -> Result<glow::Texture, EngineError> {
    let gl_w = i32::try_from(width.max(1)).unwrap_or(1);
    let gl_h = i32::try_from(height.max(1)).unwrap_or(1);

    // SAFETY: caller guarantees `gl` is an active GLES3 context.
    unsafe {
        let tex = gl
            .create_texture()
            .map_err(|e| EngineError::Gpu(format!("create layer texture: {e}")))?;
        gl.bind_texture(glow::TEXTURE_2D, Some(tex));
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MIN_FILTER,
            glow::LINEAR as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MAG_FILTER,
            glow::LINEAR as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_WRAP_S,
            glow::CLAMP_TO_EDGE as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_WRAP_T,
            glow::CLAMP_TO_EDGE as i32,
        );
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGBA8 as i32,
            gl_w,
            gl_h,
            0,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(None),
        );
        gl.bind_texture(glow::TEXTURE_2D, None);
        Ok(tex)
    }
}

/// Attaches `tex` to `fbo` at `COLOR_ATTACHMENT0`, verifies framebuffer
/// completeness, and restores the default framebuffer binding (`None`).
///
/// # Safety
///
/// `gl` must be an active GLES3 context owning `fbo` and `tex`.
unsafe fn attach_and_check_fbo(
    gl: &glow::Context,
    fbo: glow::Framebuffer,
    tex: glow::Texture,
) -> Result<(), EngineError> {
    // SAFETY: caller guarantees `gl` is an active GLES3 context owning `fbo` and `tex`.
    unsafe {
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
        gl.framebuffer_texture_2d(
            glow::FRAMEBUFFER,
            glow::COLOR_ATTACHMENT0,
            glow::TEXTURE_2D,
            Some(tex),
            0,
        );
        let status = gl.check_framebuffer_status(glow::FRAMEBUFFER);
        gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        if status != glow::FRAMEBUFFER_COMPLETE {
            return Err(EngineError::Gpu(format!(
                "layer framebuffer incomplete: status 0x{status:x}"
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_cache_defaults() {
        let mut cache = LayerCache::new();
        assert!(cache.is_empty());
        assert_eq!(cache.len(), 0);
        assert!(!cache.is_valid(1, 100, 100, 42));
        assert!(cache.get(1).is_none());

        cache.begin_frame();
        assert_eq!(cache.frame_gen, 1);
    }

    #[test]
    fn default_matches_new() {
        let cache = LayerCache::default();
        assert!(cache.is_empty());
        assert_eq!(cache.max_idle_frames, DEFAULT_MAX_IDLE_FRAMES);
    }
}
