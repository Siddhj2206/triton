//! `triton-engine`: Triton's owned Wayland + GLES3 UI toolkit.
//!
//! Architecture:
//! - [`platform`]: SCTK 0.21 + calloop + dynamic EGL/`wl_egl` multi-surface layer-shell host
//!   (`unsafe` quarantined strictly inside `platform/wl_egl.rs` and `platform/egl.rs`).
//! - [`gpu`]: GLES3 (`glow`) SDF uber-quad renderer (smooth corners, per-corner radii,
//!   inner borders, Glass 1px top highlight, soft drop shadows, scissor clipping, premultiplied alpha)
//!   plus instanced glyph rendering, an offscreen FBO [`gpu::layer_cache::LayerCache`], and
//!   `EGL_BUFFER_AGE_KHR` partial damage (`unsafe` quarantined in `gpu/mod.rs`).
//! - [`text`]: `cosmic-text` + `swash` shaping and resettable/LRU shelf glyph atlas
//!   (`unsafe` quarantined in `text/mod.rs` for texture upload).
//! - [`ui`]: 100% safe declarative element builder API (`col`, `row`, `box_`, `text`, `icon`, `image`)
//!   backed by Taffy flexbox layout, automatic hit-testing, hover/press/drag/focus/scroll,
//!   blur-region reporting, and subtree transform stack.
//!
//! Styled Material 3 Futura components live in `triton-kit`, keeping `triton-engine`
//! strictly shell-agnostic.
//!
//! ## Subtree transform design
//! Animated panels apply a hierarchical [`style::SubtreeTransform`] (translate + scale + opacity)
//! directly during quad/glyph emission, with glyph rasterization scale quantized to `0.125` steps
//! and positions computed at true floating-point scale. This keeps SDF corners, borders, and text
//! razor-sharp at arbitrary fractional display scales (e.g. 1.75×) without allocating per-panel
//! offscreen FBOs or thrashing the glyph atlas, while the resettable shelf atlas guarantees bounded
//! VRAM across long-running sessions.

pub mod anim;
pub mod damage;
pub mod error;
pub mod gpu;
pub mod image;
pub mod platform;
pub mod style;
pub mod text;
pub mod timer;
pub mod ui;

pub use anim::{Animator, Presence};
pub use damage::{DamageRect, DamageTracker};
pub use error::EngineError;
pub use image::{ImageData, ImageFit, ImageId, ImageStore, ImageStyle};
pub use platform::{
    AppCommand, ShellApp, SurfaceAnchor, SurfaceKeyboard, SurfaceLayer, SurfaceMetrics,
    SurfaceSize, SurfaceSpec, ViewContext, run,
};
pub use style::{Corners, DropShadow, Edges, FontSpec, Rect, Rgba, SubtreeTransform, Theme};
pub use timer::TimerScheduler;
pub use triton_design::motion::Spring;
pub use ui::{
    BlurRect, BuiltFrame, Element, InputEvent, InteractionState, Key, NodeKey, ScrollbarStyle,
    TextAlign, box_, col, icon, image, row, text,
};
