//! The four materials. `triton-engine` paints the tint; niri blurs behind it through
//! `ext-background-effect-v1` and layer/window rules generated from [`MaterialSpec`],
//! so the two can never disagree.
//!
//! Rules from the canvas: no glass on glass; if text drops under 4.5:1, go solid;
//! Mica falls back to Solid on battery saver.

use crate::color::{Color, Scheme, Variant, with_alpha};
use crate::tokens;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Material {
    /// Content: text, code, lists, media. Opaque.
    Solid,
    /// Chrome: header bars, sidebars, the pinned bar. Samples the wallpaper only.
    Mica,
    /// Floating things: bar islands, menus, quick settings, OSD, toasts.
    Glass,
    /// Big modal panels over busy content: launcher, switcher, clipboard.
    Thick,
}

/// How a translucent material is drawn: tint alpha in iced, backdrop in niri.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MaterialSpec {
    /// Opacity of the tint fill.
    pub alpha: f32,
    /// Blur radius behind the surface, in logical px.
    pub blur: f32,
    pub saturate: f32,
    pub brightness: f32,
    /// niri `xray`: `true` sees only the wallpaper, `false` sees the windows below.
    pub xray: Option<bool>,
}

impl Material {
    /// The backdrop recipe, or `None` for Solid.
    pub const fn spec(self, variant: Variant) -> Option<MaterialSpec> {
        match (self, variant) {
            (Material::Solid, _) => None,
            (Material::Mica, Variant::Dark) => Some(tokens::MICA_DARK),
            (Material::Glass, Variant::Dark) => Some(tokens::GLASS_DARK),
            (Material::Thick, Variant::Dark) => Some(tokens::THICK_DARK),
            (Material::Mica, Variant::Light) => Some(tokens::MICA_LIGHT),
            (Material::Glass, Variant::Light) => Some(tokens::GLASS_LIGHT),
            (Material::Thick, Variant::Light) => Some(tokens::THICK_LIGHT),
        }
    }

    /// The fill iced paints. Solid and Mica tint with `surface_container_low`,
    /// Glass and Thick with `surface_container_high`.
    pub fn fill(self, scheme: &Scheme, variant: Variant) -> Color {
        let base = match self {
            Material::Solid | Material::Mica => scheme.surface_container_low,
            Material::Glass | Material::Thick => scheme.surface_container_high,
        };
        match self.spec(variant) {
            Some(spec) => with_alpha(base, spec.alpha),
            None => base,
        }
    }

    /// Mica's fill while the wallpaper under it is bright (mean luminance over 0.6),
    /// so bar text stays above 4.5:1. Other materials are unchanged.
    pub fn fill_adaptive(self, scheme: &Scheme, variant: Variant, bright_backdrop: bool) -> Color {
        let fill = self.fill(scheme, variant);
        if self != Material::Mica || !bright_backdrop {
            return fill;
        }
        let extra = match variant {
            Variant::Dark => tokens::MICA_ADAPTIVE_DARK,
            Variant::Light => tokens::MICA_ADAPTIVE_LIGHT,
        };
        Color {
            a: (fill.a + extra).min(1.0),
            ..fill
        }
    }

    /// Whether it gets the 1 px top highlight and a floating shadow.
    pub const fn floats(self) -> bool {
        matches!(self, Material::Glass | Material::Thick)
    }

    /// The shadow under a floating material.
    pub const fn shadow(self) -> Option<ShadowToken> {
        match self {
            Material::Glass => Some(tokens::SHADOW_FLOATING),
            Material::Thick => Some(tokens::SHADOW_FLOATING_LARGE),
            Material::Solid | Material::Mica => None,
        }
    }
}

/// The 1 px inner highlight along the top edge of Glass and Thick.
pub const HIGHLIGHT: Color = Color::from_rgba(1.0, 1.0, 1.0, 0.14);

/// Fill for controls that sit on Glass, Thick or a scrim. A solid container there
/// would read as a hole in the material.
pub const fn veil(variant: Variant) -> Color {
    match variant {
        Variant::Dark => tokens::VEIL_DARK,
        Variant::Light => tokens::VEIL_LIGHT,
    }
}

/// The hovered or pressed veil.
pub const fn veil_hi(variant: Variant) -> Color {
    match variant {
        Variant::Dark => tokens::VEIL_HI_DARK,
        Variant::Light => tokens::VEIL_HI_LIGHT,
    }
}

/// A shadow from the tokens. `spread` is kept because the window outline is a
/// spread-only shadow.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShadowToken {
    pub x: f32,
    pub y: f32,
    pub blur: f32,
    pub spread: f32,
    pub color: Color,
}

impl ShadowToken {
    pub const FLOATING: ShadowToken = tokens::SHADOW_FLOATING;
    pub const FLOATING_LARGE: ShadowToken = tokens::SHADOW_FLOATING_LARGE;
    pub const RAISED: ShadowToken = tokens::SHADOW_RAISED;
    pub const WINDOW_OUTLINE: ShadowToken = tokens::WINDOW_OUTLINE;
}
