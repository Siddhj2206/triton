//! Icons: Material Symbols Rounded at weight 500, outlined or filled.
//!
//! cosmic-text applies only the `wght` axis of a variable font, so the FILL axis is
//! baked into two fonts, "Triton Symbols" and "Triton Symbols Filled" (built by
//! `mise run fonts`). Draw an icon as one character, [`Icon::codepoint`], in
//! [`Icon::font`].

use iced_core::Font;
use iced_core::font::{Family, Weight};

include!(concat!(env!("OUT_DIR"), "/icons.rs"));

/// Outlined is the default. Filled marks selection or an active state
/// (an on toggle, the current page, a muted mic).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum IconStyle {
    #[default]
    Outlined,
    Filled,
}

impl IconStyle {
    pub const fn family(self) -> &'static str {
        match self {
            IconStyle::Outlined => "Triton Symbols",
            IconStyle::Filled => "Triton Symbols Filled",
        }
    }
}

impl Icon {
    /// The font to draw this icon in. Weight 500, as the design specifies.
    pub const fn font(style: IconStyle) -> Font {
        Font {
            family: Family::Name(style.family()),
            weight: Weight::Medium,
            ..Font::DEFAULT
        }
    }
}
