//! Core Triton Material 3 Futura widgets built 100% in safe Rust on [`triton_engine::ui`].
//!
//! Organized by component family — later phases add `progress` and `overlay`
//! modules alongside these:
//! - [`surface`]: [`panel`] & [`island`] (materials, top highlight, blur reporting,
//!   [`shadow_gutter`]), [`card`], [`scrim`], [`dialog`], [`divider`],
//!   [`empty_state`], [`list_group`] + [`list_row`], [`scroll_area`].
//! - [`button`]: [`button`], [`icon_button`], [`ButtonTone`].
//! - [`control`]: [`switch`], [`checkbox`], [`radio`], [`slider`] (`Animator`-driven).
//! - [`input`]: [`TextInputState`] + [`text_input`].
//! - [`chrome`]: [`pill`], [`icon_disc`].
//!
//! Depends only on `triton-design` and `triton-engine`: no D-Bus, tokio, or IPC.

#![forbid(unsafe_code)]

mod button;
mod chrome;
mod control;
mod input;
mod surface;

pub use button::{ButtonTone, button, icon_button};
pub use chrome::{icon_disc, pill};
pub use control::{
    CheckState, SliderMode, SliderOptions, SliderSurface, checkbox, radio, slider, switch,
};
pub use input::{TextInputAction, TextInputState, text_input};
pub use surface::{
    Dialog, DividerStyle, ListFill, Orientation, ScrollAreaOptions, card, concentric_inner, dialog,
    divider, empty_state, island, list_group, list_row, list_row_text, panel, scrim, scroll_area,
    shadow_gutter,
};
