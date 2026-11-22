//! `triton-engine`: Triton's owned Wayland + GLES3 UI toolkit.
//!
//! Architecture:
//! - [`platform`]: SCTK 0.21 + calloop + dynamic EGL/`wl_egl` multi-surface layer-shell host
//!   (`unsafe` quarantined strictly inside `platform/wl_egl.rs` and `platform/egl.rs`).
//! - [`gpu`]: GLES3 (`glow`) SDF uber-quad renderer (smooth corners, per-corner radii,
//!   inner borders, Glass 1px top highlight, soft drop shadows, scissor clipping, premultiplied alpha)
//!   plus instanced glyph rendering (`unsafe` quarantined in `gpu/mod.rs`).
//! - [`text`]: `cosmic-text` + `swash` shaping and resettable/LRU shelf glyph atlas
//!   (`unsafe` quarantined in `text/mod.rs` for texture upload).
//! - [`ui`]: 100% safe declarative element builder API (`col`, `row`, `box_`, `text`, `icon`)
//!   backed by Taffy flexbox layout, automatic hit-testing, hover/press/drag/focus/scroll,
//!   blur-region reporting, and subtree transform stack.
//! - [`widgets`]: 100% safe Triton Material 3 Futura components (`panel`, `island`, `button`,
//!   `icon_button`, `pill`, `text_input`, `slider`, `switch`, `scroll_area`).
//!
//! ## Subtree transform design
//! Animated panels apply a hierarchical [`style::SubtreeTransform`] (translate + scale + opacity)
//! directly during quad/glyph emission, with glyph rasterization scale quantized to `0.125` steps
//! and positions computed at true floating-point scale. This keeps SDF corners, borders, and text
//! razor-sharp at arbitrary fractional display scales (e.g. 1.75×) without allocating per-panel
//! offscreen FBOs or thrashing the glyph atlas, while the resettable shelf atlas guarantees bounded
//! VRAM across long-running sessions.

pub mod error;
pub mod gpu;
pub mod platform;
pub mod style;
pub mod text;
pub mod ui;
pub mod widgets;

pub use error::EngineError;
pub use platform::{
    AppCommand, ShellApp, SurfaceAnchor, SurfaceKeyboard, SurfaceLayer, SurfaceMetrics,
    SurfaceSize, SurfaceSpec, ViewContext, run,
};
pub use style::{Corners, DropShadow, Edges, FontSpec, Rect, Rgba, SubtreeTransform, Theme};
pub use ui::{
    BlurRect, BuiltFrame, Element, InputEvent, InteractionState, Key, NodeKey, TextAlign, box_,
    col, icon, row, text,
};
pub use widgets::{
    ButtonTone, SwitchState, TextInputAction, TextInputState, button, concentric_inner,
    icon_button, island, panel, pill, scroll_area, slider, switch, text_input,
};
