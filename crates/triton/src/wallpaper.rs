//! Per-output wallpaper pipeline (#18).
//!
//! Owns wallpaper decoding (off the UI thread via [`spawn_decode`]), WCAG
//! mean-luminance detection (`bright_backdrop` for adaptive Mica), the decoded
//! image registry shared with #19 `material-colors`, per-output [`Crossfade`]
//! state, and the `Layer::Background` view body.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use thiserror::Error;
use tracing::warn;
use triton_design::color::{self, Color};
use triton_design::material::Material;
use triton_design::shape::RADII;
use triton_engine::{Element, EventSender, ImageData, ImageId, ImageStore, ImageStyle, Theme};
use triton_kit::panel;

use crate::state::{OutputKey, ShellEvent};

/// Wallpaper crossfade duration under normal motion (design spec: 200 ms).
pub const CROSSFADE_DURATION: Duration = Duration::from_millis(200);

/// Crossfade duration under `reduce_motion` (120 ms linear fade).
pub const CROSSFADE_DURATION_REDUCED: Duration = Duration::from_millis(120);

/// Mean relative luminance above which `bright_backdrop` engages adaptive
/// Mica alpha (`+0.08` dark / `+0.06` light).
pub const BRIGHT_BACKDROP_THRESHOLD: f32 = 0.6;

/// The crossfade duration to use for a transition starting now.
pub const fn crossfade_duration(reduce_motion: bool) -> Duration {
    if reduce_motion {
        CROSSFADE_DURATION_REDUCED
    } else {
        CROSSFADE_DURATION
    }
}

/// Failures decoding a wallpaper file into GPU-ready RGBA8 pixels.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum WallpaperError {
    /// Reading the wallpaper file failed.
    #[error("failed to read wallpaper {}: {source}", .path.display())]
    Io {
        /// Path that failed to read.
        path: PathBuf,
        /// Underlying I/O error.
        source: std::io::Error,
    },
    /// The file is corrupt or not a supported image format.
    #[error("failed to decode wallpaper {}: {source}", .path.display())]
    Decode {
        /// Path that failed to decode.
        path: PathBuf,
        /// Decoder error.
        source: image::ImageError,
    },
    /// Decoded pixels failed RGBA8 buffer validation.
    #[error("wallpaper {} decoded to invalid RGBA8 data", .path.display())]
    InvalidData {
        /// Path that produced invalid data.
        path: PathBuf,
    },
}

impl WallpaperError {
    /// The wallpaper path this error refers to.
    pub fn path(&self) -> &Path {
        match self {
            Self::Io { path, .. } | Self::Decode { path, .. } | Self::InvalidData { path } => path,
        }
    }
}

/// A decoded wallpaper: GPU-ready straight RGBA8 pixels plus derived metrics.
///
/// The raw pixels stay available on [`WallpaperImage::data`] so downstream
/// consumers (#19 `material-colors` extraction) can read `data.rgba` directly
/// without re-decoding.
#[derive(Debug, Clone, PartialEq)]
pub struct WallpaperImage {
    /// Source path the image was decoded from.
    pub path: PathBuf,
    /// Decoded straight RGBA8 pixels (`width * height * 4` bytes).
    pub data: ImageData,
    /// Mean relative luminance in `0.0..=1.0` (WCAG, alpha-weighted);
    /// values `> BRIGHT_BACKDROP_THRESHOLD` engage adaptive Mica alpha.
    pub mean_luminance: f32,
}

impl WallpaperImage {
    /// Build a [`WallpaperImage`] from already-decoded RGBA8 pixels, computing
    /// [`Self::mean_luminance`].
    pub fn from_data(path: PathBuf, data: ImageData) -> Self {
        let mean_luminance = mean_relative_luminance(&data);
        Self {
            path,
            data,
            mean_luminance,
        }
    }

    /// Decode a wallpaper file (PNG, JPEG, WebP, GIF) into RGBA8 pixels.
    ///
    /// Blocking file I/O and pixel decoding — call off the UI thread (see
    /// [`spawn_decode`]).
    pub fn decode(path: &Path) -> Result<Self, WallpaperError> {
        let path = path.to_path_buf();
        let reader = image::ImageReader::open(&path).map_err(|source| WallpaperError::Io {
            path: path.clone(),
            source,
        })?;
        let decoded = reader.decode().map_err(|source| WallpaperError::Decode {
            path: path.clone(),
            source,
        })?;
        let rgba = decoded.to_rgba8();
        let (width, height) = rgba.dimensions();
        let data = ImageData::from_rgba(width, height, rgba.into_raw())
            .ok_or(WallpaperError::InvalidData { path: path.clone() })?;
        Ok(Self::from_data(path, data))
    }

    /// `true` when this wallpaper is bright enough to engage adaptive Mica alpha.
    pub fn is_bright_backdrop(&self) -> bool {
        self.mean_luminance > BRIGHT_BACKDROP_THRESHOLD
    }
}

/// Compute the alpha-weighted mean relative luminance (WCAG) of straight
/// RGBA8 pixels: each pixel's linear-light luminance scaled by its alpha, so
/// transparent regions count as the black backdrop beneath them.
#[must_use]
pub fn mean_relative_luminance(data: &ImageData) -> f32 {
    let pixels = data.rgba.len() / 4;
    if pixels == 0 {
        return 0.0;
    }
    let mut sum = 0.0_f64;
    for px in data.rgba.as_chunks::<4>().0 {
        let lum = color::luminance(Color::from_rgb8(px[0], px[1], px[2]));
        sum += f64::from(lum) * (f64::from(px[3]) / 255.0);
    }
    (sum / pixels as f64) as f32
}

/// Decode `path` on a named worker thread and deliver the result into the
/// shell event loop as [`ShellEvent::WallpaperReady`] or
/// [`ShellEvent::WallpaperFailed`]. Returns the worker's join handle so tests
/// can block on decode completion deterministically.
pub fn spawn_decode(
    path: PathBuf,
    sender: EventSender<ShellEvent>,
) -> std::io::Result<JoinHandle<()>> {
    thread::Builder::new()
        .name("triton-wallpaper".to_owned())
        .spawn(move || match WallpaperImage::decode(&path) {
            Ok(image) => {
                if sender
                    .send(ShellEvent::WallpaperReady {
                        image: Arc::new(image),
                    })
                    .is_err()
                {
                    warn!(path = %path.display(), "wallpaper decoded but shell is gone");
                }
            }
            Err(err) => {
                let _ = sender.send(ShellEvent::WallpaperFailed {
                    path: err.path().to_path_buf(),
                    error: err.to_string(),
                });
            }
        })
}

/// A linear crossfade between two GPU-resident wallpaper images.
///
/// Deterministic and time-as-argument: `progress(now)` drives the blend and
/// `is_settled(now)` lets the engine stop frame callbacks once the transition
/// completes (0% idle CPU).
#[derive(Debug, Clone, PartialEq)]
pub struct Crossfade {
    /// Texture shown at `t = 0` (the outgoing wallpaper).
    pub from: ImageId,
    /// Texture shown at `t = 1` (the incoming wallpaper).
    pub to: ImageId,
    /// When the fade began.
    pub start: Instant,
    /// Total fade duration.
    pub duration: Duration,
}

impl Crossfade {
    /// Begin a fade `from -> to` at `start` with the duration appropriate for
    /// the current `reduce_motion` setting.
    pub fn new(from: ImageId, to: ImageId, start: Instant, reduce_motion: bool) -> Self {
        Self {
            from,
            to,
            start,
            duration: crossfade_duration(reduce_motion),
        }
    }

    /// Clamped linear progress `0.0..=1.0` at `now`.
    pub fn progress(&self, now: Instant) -> f32 {
        let elapsed = now.saturating_duration_since(self.start);
        (elapsed.as_secs_f32() / self.duration.as_secs_f32()).clamp(0.0, 1.0)
    }

    /// `true` once the fade completed at or before `now`.
    pub fn is_settled(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.start) >= self.duration
    }

    /// Time at which the fade completes.
    pub fn deadline(&self) -> Instant {
        self.start + self.duration
    }

    /// The [`ImageStyle`] blending `from` into `to` at `now`.
    pub fn style(&self, now: Instant) -> ImageStyle {
        ImageStyle {
            crossfade_to: Some(self.to),
            crossfade_t: self.progress(now),
            ..ImageStyle::default()
        }
    }
}

/// Decoded wallpaper entry in [`WallpaperSet`]: pixels kept cached for quick
/// re-display, with a GPU texture slot while any output references it.
#[derive(Debug)]
struct Loaded {
    image: Arc<WallpaperImage>,
    id: Option<ImageId>,
    /// Output reference count across `current` and `Crossfade::from` slots.
    refs: usize,
}

/// Decoded pixels queued for the next [`WallpaperSet::sync`] upload.
#[derive(Debug)]
struct PendingUpload {
    path: PathBuf,
    data: ImageData,
    /// When the decoded image became available; seeds the crossfade start.
    ready_at: Instant,
}

/// Per-output wallpaper display state.
#[derive(Debug, Clone, PartialEq)]
struct OutputEntry {
    /// Latest requested path (what this output should end up showing).
    desired: PathBuf,
    /// The texture currently displayed, or the fade target while transitioning.
    current: Option<ImageId>,
    /// In-progress transition, if any. Invariant: `fade.to == current`.
    fade: Option<Crossfade>,
}

/// Decoded-wallpaper registry and per-output display/crossfade state.
///
/// Queues decoded pixels for GPU upload in [`Self::sync`], tracks which
/// [`ImageId`] each output displays or fades between with reference counting
/// (a texture shared by two outputs survives until both release it), and
/// queues GPU deletions once a transition settles.
#[derive(Debug, Default)]
pub struct WallpaperSet {
    loaded: BTreeMap<PathBuf, Loaded>,
    outputs: BTreeMap<OutputKey, OutputEntry>,
    pending_uploads: Vec<PendingUpload>,
    pending_releases: Vec<ImageId>,
    pending_decodes: BTreeSet<PathBuf>,
    reduce_motion: bool,
}

impl WallpaperSet {
    /// Update the motion mode used for crossfades started from now on.
    pub fn set_reduce_motion(&mut self, reduce_motion: bool) {
        self.reduce_motion = reduce_motion;
    }

    /// Record that `output` should display `path`, beginning the transition
    /// immediately when the image is already GPU-resident.
    pub fn request(&mut self, output: OutputKey, path: &Path, now: Instant) {
        {
            let entry = self.outputs.entry(output).or_insert(OutputEntry {
                desired: path.to_path_buf(),
                current: None,
                fade: None,
            });
            if entry.desired != path {
                entry.desired = path.to_path_buf();
            }
        }

        let resident = self.loaded.get(path).and_then(|l| l.id);
        match resident {
            Some(id) => self.begin_transition(output, id, now),
            None => {
                // Cached pixels without a live texture: re-upload on next sync.
                if let Some(loaded) = self.loaded.get(path)
                    && !self.pending_uploads.iter().any(|u| u.path == path)
                {
                    self.pending_uploads.push(PendingUpload {
                        path: path.to_path_buf(),
                        data: loaded.image.data.clone(),
                        ready_at: now,
                    });
                }
            }
        }
    }

    /// Store a freshly decoded image and queue it for GPU upload.
    pub fn on_decoded(&mut self, image: Arc<WallpaperImage>, ready_at: Instant) {
        let path = image.path.clone();
        self.pending_decodes.remove(&path);
        match self.loaded.entry(path.clone()) {
            std::collections::btree_map::Entry::Occupied(mut e) => {
                e.get_mut().image = image;
            }
            std::collections::btree_map::Entry::Vacant(e) => {
                e.insert(Loaded {
                    image,
                    id: None,
                    refs: 0,
                });
            }
        }
        let data = self.loaded[&path].image.data.clone();
        self.pending_uploads.push(PendingUpload {
            path,
            data,
            ready_at,
        });
    }

    /// Mark `path` as having a decode worker in flight (dedups requests).
    pub fn mark_decoding(&mut self, path: &Path) {
        self.pending_decodes.insert(path.to_path_buf());
    }

    /// Clear the in-flight marker after a failed decode; outputs keep their
    /// previous wallpaper.
    pub fn on_decode_failed(&mut self, path: &Path) {
        self.pending_decodes.remove(path);
    }

    /// Desired output paths with neither decoded pixels nor an in-flight decode.
    pub fn undecoded_desires(&self) -> Vec<PathBuf> {
        let mut paths = BTreeSet::new();
        for entry in self.outputs.values() {
            if !self.loaded.contains_key(&entry.desired)
                && !self.pending_decodes.contains(&entry.desired)
            {
                paths.insert(entry.desired.clone());
            }
        }
        paths.into_iter().collect()
    }

    /// `true` if an output wants `path` while already displaying a texture,
    /// meaning its arrival will start a real crossfade.
    pub fn will_transition_to(&self, path: &Path) -> bool {
        self.outputs
            .values()
            .any(|e| e.desired == path && e.current.is_some())
    }

    /// `true` while any output has an unsettled crossfade.
    pub fn has_active_fades(&self) -> bool {
        self.outputs.values().any(|e| e.fade.is_some())
    }

    /// `true` while `output` is crossfading (drives `is_animating` and the
    /// 200 ms vsync redraw window).
    pub fn is_animating(&self, output: OutputKey, now: Instant) -> bool {
        self.outputs
            .get(&output)
            .and_then(|e| e.fade.as_ref())
            .is_some_and(|f| !f.is_settled(now))
    }

    /// The active [`Crossfade`] on `output`, if any.
    pub fn fade_for(&self, output: OutputKey) -> Option<Crossfade> {
        self.outputs.get(&output).and_then(|e| e.fade.clone())
    }

    /// The texture `output` currently shows (or is fading toward).
    pub fn current_for(&self, output: OutputKey) -> Option<ImageId> {
        self.outputs.get(&output).and_then(|e| e.current)
    }

    /// The wallpaper path `output` is committed to showing.
    pub fn desired_path(&self, output: OutputKey) -> Option<PathBuf> {
        self.outputs.get(&output).map(|e| e.desired.clone())
    }

    /// Decoded image for `path`, when resident or pixel-cached.
    pub fn image_for_path(&self, path: &Path) -> Option<Arc<WallpaperImage>> {
        self.loaded.get(path).map(|l| Arc::clone(&l.image))
    }

    /// The decoded image `output` currently displays (its `current` texture).
    pub fn displayed_image(&self, output: OutputKey) -> Option<Arc<WallpaperImage>> {
        let id = self.outputs.get(&output).and_then(|e| e.current)?;
        self.loaded
            .values()
            .find(|l| l.id == Some(id))
            .map(|l| Arc::clone(&l.image))
    }

    /// Drop display state for outputs no longer connected, releasing their
    /// texture references.
    pub fn retain_outputs(&mut self, keep: impl Fn(&OutputKey) -> bool) {
        let removed: Vec<OutputKey> = self.outputs.keys().filter(|k| !keep(k)).copied().collect();
        for key in removed {
            if let Some(entry) = self.outputs.remove(&key) {
                if let Some(id) = entry.current {
                    self.unref(id);
                }
                if let Some(fade) = entry.fade {
                    self.unref(fade.from);
                }
            }
        }
    }

    /// What the background surface for `output` should draw at `now`:
    /// `(base_image, style)` where a live fade reports `crossfade_to`/`t`.
    pub fn style_for(&self, output: OutputKey, now: Instant) -> Option<(ImageId, ImageStyle)> {
        let entry = self.outputs.get(&output)?;
        if let Some(fade) = entry.fade.as_ref().filter(|f| !f.is_settled(now)) {
            return Some((fade.from, fade.style(now)));
        }
        entry.current.map(|id| (id, ImageStyle::default()))
    }

    /// Drain queued uploads/releases into `store` and start crossfades whose
    /// pixels just became GPU-resident. Called by `ShellApp::sync_images`.
    pub fn sync(&mut self, store: &mut ImageStore) {
        for id in self.pending_releases.drain(..) {
            store.remove(id);
        }
        for upload in std::mem::take(&mut self.pending_uploads) {
            let Some(loaded) = self.loaded.get_mut(&upload.path) else {
                continue;
            };
            if loaded.id.is_some() {
                continue; // already GPU-resident
            }
            let id = store.insert(upload.data);
            loaded.id = Some(id);
            let waiting: Vec<OutputKey> = self
                .outputs
                .iter()
                .filter(|(_, e)| e.desired == upload.path && e.current != Some(id))
                .map(|(k, _)| *k)
                .collect();
            for key in waiting {
                self.begin_transition(key, id, upload.ready_at);
            }
        }
    }

    /// Release fade sources whose deadline passed by `now`. Returns the
    /// remaining duration of the earliest still-running fade so the caller
    /// can re-arm the settle timer.
    pub fn settle(&mut self, now: Instant) -> Option<Duration> {
        let mut earliest: Option<Duration> = None;
        let mut released = Vec::new();
        for entry in self.outputs.values_mut() {
            if let Some(fade) = entry.fade.take() {
                if fade.is_settled(now) {
                    released.push(fade.from);
                } else {
                    let remaining = fade.deadline().saturating_duration_since(now);
                    earliest = Some(earliest.map_or(remaining, |e| e.min(remaining)));
                    entry.fade = Some(fade);
                }
            }
        }
        for id in released {
            self.unref(id);
        }
        earliest
    }

    /// Move `output`'s display slot to `to`, starting a crossfade from the
    /// previously displayed texture (or showing `to` directly when nothing
    /// was displayed).
    fn begin_transition(&mut self, output: OutputKey, to: ImageId, start: Instant) {
        let Some(entry) = self.outputs.get(&output) else {
            return;
        };
        let already_there =
            entry.current == Some(to) && entry.fade.as_ref().is_none_or(|f| f.to == to);
        if already_there {
            return;
        }
        let from = entry.current;

        // Mid-fade retarget: the old fade source is no longer shown anywhere.
        if let Some(old_fade) = self.outputs.get_mut(&output).and_then(|e| e.fade.take()) {
            self.unref(old_fade.from);
        }

        let reduce_motion = self.reduce_motion;
        let entry = self.outputs.get_mut(&output).expect("output entry exists");
        entry.fade = from
            .filter(|&f| f != to)
            .map(|from_id| Crossfade::new(from_id, to, start, reduce_motion));
        if entry.current != Some(to) {
            entry.current = Some(to);
            self.ref_id(to);
        }
    }

    fn ref_id(&mut self, id: ImageId) {
        if let Some(loaded) = self.loaded.values_mut().find(|l| l.id == Some(id)) {
            loaded.refs += 1;
        }
    }

    /// Drop one output reference to `id`; queues GPU release when it hits zero.
    fn unref(&mut self, id: ImageId) {
        for (path, loaded) in &mut self.loaded {
            if loaded.id == Some(id) {
                loaded.refs = loaded.refs.saturating_sub(1);
                if loaded.refs == 0
                    && let Some(dropped) = loaded.id.take()
                {
                    // Pixels stay cached in `loaded` for fast re-display; only
                    // the GPU texture is released.
                    let _ = path;
                    self.pending_releases.push(dropped);
                }
                return;
            }
        }
    }
}

/// Build the per-output `Layer::Background` view: the active wallpaper texture
/// (blended through [`Crossfade`]) filling the surface, or the Solid fallback
/// panel while no wallpaper is decoded yet.
pub fn view_background<M>(
    wallpapers: &WallpaperSet,
    output: OutputKey,
    theme: &Theme,
    now: Instant,
) -> Element<M> {
    if let Some((id, style)) = wallpapers.style_for(output, now) {
        triton_engine::image(id, style).w_full().h_full()
    } else {
        panel(Material::Solid, RADII.tiled, theme).w_full().h_full()
    }
}
