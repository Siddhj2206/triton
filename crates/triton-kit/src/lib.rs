//! Core Triton Material 3 Futura widgets built 100% in safe Rust on [`triton_engine::ui`].
//!
//! Organized by component family — later phases add `overlay`
//! modules alongside these:
//! - [`surface`]: [`panel`] & [`island`] (materials, top highlight, blur reporting,
//!   [`shadow_gutter`]), [`card`], [`scrim`], [`dialog`], [`divider`],
//!   [`empty_state`], [`list_group`] + [`list_row`], [`scroll_area`].
//! - [`button`]: [`button`], [`icon_button`], [`ButtonTone`].
//! - [`control`]: [`slider`], [`SwitchState`] + [`switch`].
//! - [`input`]: [`TextInputState`] + [`text_input`].
//! - [`chrome`]: [`pill`], [`status_pill`], [`overflow_pill`], [`icon_disc`],
//!   [`wave_avatar`], [`badge`], [`keycap`], [`hint_bar`].
//! - [`progress`]: [`wavy_progress`], [`wavy_ring`], [`level_meter`].
//!
//! Depends only on `triton-design` and `triton-engine`: no D-Bus, tokio, or IPC.

#![forbid(unsafe_code)]

mod button;
mod chrome;
mod control;
mod input;
mod progress;
mod surface;

pub use button::{ButtonTone, button, icon_button};
pub use chrome::{
    Badge, KeycapStyle, PillStatus, badge, badged, hint_bar, icon_disc, icon_disc_styled, keycap,
    overflow_pill, pill, status_pill, wave_avatar, wave_avatar_image,
};
pub use control::{SwitchState, slider, switch};
pub use input::{TextInputAction, TextInputState, text_input};
pub use progress::{MeterMode, Progress, level_meter, wavy_progress, wavy_ring};
pub use surface::{
    Dialog, DividerStyle, ListFill, Orientation, ScrollAreaOptions, card, concentric_inner, dialog,
    divider, empty_state, island, list_group, list_row, list_row_text, panel, scrim, scroll_area,
    shadow_gutter,
};
