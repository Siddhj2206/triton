#![forbid(unsafe_code)]

//! Safe CPU-side image and texture management for `triton-engine`.
//!
//! Tracks image metadata (`ImageId` -> dimensions) and queues pending GPU
//! uploads/updates/deletions so `ui/` can reference wallpaper crossfades,
//! album art, user avatars, and live screencopy window previews without ever
//! touching OpenGL.

use std::collections::HashMap;

/// Opaque handle to a managed image/texture in [`ImageStore`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ImageId(pub u64);

/// How an image is fitted into its destination rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ImageFit {
    /// Scale uniformly to fill the destination, center-cropping overflow.
    #[default]
    Cover,
    /// Scale uniformly to fit inside the destination, letterboxing if needed.
    Contain,
    /// Stretch to fill the destination without preserving aspect ratio.
    Fill,
}

/// Visual presentation parameters for rendering an image.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImageStyle {
    /// Aspect-ratio fitting mode.
    pub fit: ImageFit,
    /// Opacity multiplier in `0.0..=1.0`.
    pub opacity: f32,
    /// Normalized blur strength in `0.0..=1.0` (e.g. private previews / lock backdrop).
    pub blur: f32,
    /// Desaturation amount in `0.0..=1.0`.
    pub desaturate: f32,
    /// Optional second image for wallpaper crossfade.
    pub crossfade_to: Option<ImageId>,
    /// Crossfade blend factor in `0.0..=1.0`.
    pub crossfade_t: f32,
}

impl Default for ImageStyle {
    fn default() -> Self {
        Self {
            fit: ImageFit::default(),
            opacity: 1.0,
            blur: 0.0,
            desaturate: 0.0,
            crossfade_to: None,
            crossfade_t: 0.0,
        }
    }
}

/// Validated RGBA8 pixel buffer ready for GPU upload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageData {
    /// Image width in pixels (always `> 0` when constructed via [`ImageData::from_rgba`]).
    pub width: u32,
    /// Image height in pixels (always `> 0` when constructed via [`ImageData::from_rgba`]).
    pub height: u32,
    /// Straight or premultiplied RGBA8 pixels (`width * height * 4` bytes).
    pub rgba: Vec<u8>,
}

impl ImageData {
    /// Construct [`ImageData`] after validating non-zero dimensions and exact byte length.
    pub fn from_rgba(width: u32, height: u32, rgba: Vec<u8>) -> Option<Self> {
        if width == 0 || height == 0 {
            return None;
        }
        let expected_len = (width as usize)
            .checked_mul(height as usize)?
            .checked_mul(4)?;
        if rgba.len() != expected_len {
            return None;
        }
        Some(Self {
            width,
            height,
            rgba,
        })
    }
}

/// CPU-side image registry and GPU upload/deletion queue.
#[derive(Debug)]
pub struct ImageStore {
    next_id: u64,
    dimensions: HashMap<ImageId, (u32, u32)>,
    pending_uploads: Vec<(ImageId, ImageData)>,
    pending_deletions: Vec<ImageId>,
}

impl Default for ImageStore {
    fn default() -> Self {
        Self::new()
    }
}

impl ImageStore {
    /// Create an empty image store.
    pub fn new() -> Self {
        Self {
            next_id: 1,
            dimensions: HashMap::new(),
            pending_uploads: Vec::new(),
            pending_deletions: Vec::new(),
        }
    }

    /// Insert a new image, allocating a fresh [`ImageId`] and queuing a GPU upload.
    pub fn insert(&mut self, data: ImageData) -> ImageId {
        let id = ImageId(self.next_id);
        self.next_id = self.next_id.wrapping_add(1).max(1);
        self.dimensions.insert(id, (data.width, data.height));
        self.pending_uploads.push((id, data));
        id
    }

    /// Replace pixel data for an existing image without reallocating its [`ImageId`].
    ///
    /// Intended for live screencopy frames. Returns `false` if `id` is unknown.
    pub fn update(&mut self, id: ImageId, data: ImageData) -> bool {
        let Some(dims) = self.dimensions.get_mut(&id) else {
            return false;
        };
        *dims = (data.width, data.height);
        if let Some((_, queued)) = self
            .pending_uploads
            .iter_mut()
            .find(|(queued_id, _)| *queued_id == id)
        {
            *queued = data;
        } else {
            self.pending_uploads.push((id, data));
        }
        true
    }

    /// Remove an image and queue its GPU texture for deletion.
    ///
    /// Returns `false` if `id` is unknown.
    pub fn remove(&mut self, id: ImageId) -> bool {
        if self.dimensions.remove(&id).is_none() {
            return false;
        }
        self.pending_uploads
            .retain(|(queued_id, _)| *queued_id != id);
        self.pending_deletions.push(id);
        true
    }

    /// Query the pixel dimensions `(width, height)` of a stored image.
    pub fn dimensions(&self, id: ImageId) -> Option<(u32, u32)> {
        self.dimensions.get(&id).copied()
    }

    /// Drain all queued GPU texture uploads/re-uploads.
    pub fn drain_uploads(&mut self) -> Vec<(ImageId, ImageData)> {
        std::mem::take(&mut self.pending_uploads)
    }

    /// Drain all queued GPU texture deletions.
    pub fn drain_deletions(&mut self) -> Vec<ImageId> {
        std::mem::take(&mut self.pending_deletions)
    }
}

/// Compute normalized `[u0, v0, uw, vh]` texture coordinates in `0.0..=1.0`.
///
/// - [`ImageFit::Cover`]: center-crops the source to match the destination aspect ratio.
/// - [`ImageFit::Contain`]: returns `[0.0, 0.0, 1.0, 1.0]`; pair with [`compute_contain_dst`]
///   for the letterboxed destination rectangle.
/// - [`ImageFit::Fill`]: returns `[0.0, 0.0, 1.0, 1.0]`.
pub fn compute_uv_rect(src_w: f32, src_h: f32, dst_w: f32, dst_h: f32, fit: ImageFit) -> [f32; 4] {
    match fit {
        ImageFit::Contain | ImageFit::Fill => [0.0, 0.0, 1.0, 1.0],
        ImageFit::Cover => {
            if src_w <= 0.0 || src_h <= 0.0 || dst_w <= 0.0 || dst_h <= 0.0 {
                return [0.0, 0.0, 1.0, 1.0];
            }
            let src_aspect = src_w / src_h;
            let dst_aspect = dst_w / dst_h;
            if src_aspect > dst_aspect {
                let uw = dst_aspect / src_aspect;
                let u0 = (1.0 - uw) * 0.5;
                [u0, 0.0, uw, 1.0]
            } else {
                let vh = src_aspect / dst_aspect;
                let v0 = (1.0 - vh) * 0.5;
                [0.0, v0, 1.0, vh]
            }
        }
    }
}

/// Compute the letterboxed destination rectangle `[x, y, w, h]` for [`ImageFit::Contain`].
pub fn compute_contain_dst(src_w: f32, src_h: f32, dst: [f32; 4]) -> [f32; 4] {
    let [dx, dy, dw, dh] = dst;
    if src_w <= 0.0 || src_h <= 0.0 || dw <= 0.0 || dh <= 0.0 {
        return dst;
    }
    let src_aspect = src_w / src_h;
    let dst_aspect = dw / dh;
    if src_aspect > dst_aspect {
        let h = dw / src_aspect;
        let y = dy + (dh - h) * 0.5;
        [dx, y, dw, h]
    } else {
        let w = dh * src_aspect;
        let x = dx + (dw - w) * 0.5;
        [x, dy, w, dh]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx_eq(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    fn approx_rect(a: [f32; 4], b: [f32; 4]) -> bool {
        a.iter().zip(b.iter()).all(|(&x, &y)| approx_eq(x, y))
    }

    #[test]
    fn from_rgba_validates_dimensions_and_length() {
        assert!(ImageData::from_rgba(0, 4, vec![]).is_none());
        assert!(ImageData::from_rgba(4, 0, vec![]).is_none());
        assert!(ImageData::from_rgba(2, 2, vec![0; 15]).is_none());
        assert!(ImageData::from_rgba(2, 2, vec![0; 17]).is_none());

        let valid = ImageData::from_rgba(2, 3, vec![255; 2 * 3 * 4]).unwrap();
        assert_eq!(valid.width, 2);
        assert_eq!(valid.height, 3);
        assert_eq!(valid.rgba.len(), 24);
    }

    #[test]
    fn store_insert_update_remove_and_drain() {
        let mut store = ImageStore::new();
        let frame1 = ImageData::from_rgba(4, 4, vec![1; 64]).unwrap();
        let id = store.insert(frame1);
        assert_eq!(store.dimensions(id), Some((4, 4)));

        let uploads = store.drain_uploads();
        assert_eq!(uploads.len(), 1);
        assert_eq!(uploads[0].0, id);
        assert_eq!(uploads[0].1.rgba[0], 1);
        assert_eq!(store.drain_uploads().len(), 0);

        // Live screencopy re-upload without reallocating ImageId.
        let frame2 = ImageData::from_rgba(8, 6, vec![2; 8 * 6 * 4]).unwrap();
        assert!(store.update(id, frame2));
        assert_eq!(store.dimensions(id), Some((8, 6)));

        // Coalesced second update before GPU drains.
        let frame3 = ImageData::from_rgba(8, 6, vec![3; 8 * 6 * 4]).unwrap();
        assert!(store.update(id, frame3));

        let uploads = store.drain_uploads();
        assert_eq!(uploads.len(), 1);
        assert_eq!(uploads[0].0, id);
        assert_eq!(uploads[0].1.rgba[0], 3);

        // Updating an unknown id returns false.
        let bogus = ImageId(999);
        let dummy = ImageData::from_rgba(1, 1, vec![0; 4]).unwrap();
        assert!(!store.update(bogus, dummy));
        assert_eq!(store.drain_uploads().len(), 0);

        // Remove queues deletion and clears metadata.
        assert!(store.remove(id));
        assert!(!store.remove(id));
        assert_eq!(store.dimensions(id), None);

        let deletions = store.drain_deletions();
        assert_eq!(deletions, vec![id]);
        assert_eq!(store.drain_deletions().len(), 0);
    }

    #[test]
    fn compute_uv_rect_cover_and_fill() {
        // Wide 200x100 image into 100x100 square: crop left/right 25%.
        let uv_wide = compute_uv_rect(200.0, 100.0, 100.0, 100.0, ImageFit::Cover);
        assert!(approx_rect(uv_wide, [0.25, 0.0, 0.5, 1.0]));

        // Tall 100x200 image into 200x100 wide dst: src_aspect=0.5, dst_aspect=2.0 -> vh=0.25.
        let uv_tall = compute_uv_rect(100.0, 200.0, 200.0, 100.0, ImageFit::Cover);
        assert!(approx_rect(uv_tall, [0.0, 0.375, 1.0, 0.25]));

        // Fill always uses full texture UVs.
        let uv_fill = compute_uv_rect(200.0, 100.0, 100.0, 300.0, ImageFit::Fill);
        assert!(approx_rect(uv_fill, [0.0, 0.0, 1.0, 1.0]));

        // Contain returns full UVs; destination letterboxing is handled by compute_contain_dst.
        let uv_contain = compute_uv_rect(200.0, 100.0, 100.0, 100.0, ImageFit::Contain);
        assert!(approx_rect(uv_contain, [0.0, 0.0, 1.0, 1.0]));
    }

    #[test]
    fn compute_contain_dst_letterboxes_correctly() {
        // Wide 200x100 into [10, 20, 100, 100] -> width-bound: 100x50 centered vertically.
        let dst_wide = compute_contain_dst(200.0, 100.0, [10.0, 20.0, 100.0, 100.0]);
        assert!(approx_rect(dst_wide, [10.0, 45.0, 100.0, 50.0]));

        // Tall 100x200 into [10, 20, 200, 100] -> height-bound: 50x100 centered horizontally.
        let dst_tall = compute_contain_dst(100.0, 200.0, [10.0, 20.0, 200.0, 100.0]);
        assert!(approx_rect(dst_tall, [85.0, 20.0, 50.0, 100.0]));
    }
}
