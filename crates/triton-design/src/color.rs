//! Color roles. The canvas derives them from the wallpaper (tonal-spot); these are
//! the canvas's reference scheme, used until the wallpaper pipeline produces one.

use crate::tokens;

/// RGBA color in straight (non-premultiplied) 0.0..=1.0 float channels.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const TRANSPARENT: Self = Self::from_rgba(0.0, 0.0, 0.0, 0.0);
    pub const WHITE: Self = Self::from_rgb(1.0, 1.0, 1.0);
    pub const BLACK: Self = Self::from_rgb(0.0, 0.0, 0.0);

    pub const fn from_rgb(r: f32, g: f32, b: f32) -> Self {
        Self { r, g, b, a: 1.0 }
    }

    pub const fn from_rgba(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    pub const fn from_rgb8(r: u8, g: u8, b: u8) -> Self {
        Self {
            r: r as f32 / 255.0,
            g: g as f32 / 255.0,
            b: b as f32 / 255.0,
            a: 1.0,
        }
    }

    pub const fn from_rgba8(r: u8, g: u8, b: u8, a: f32) -> Self {
        Self {
            r: r as f32 / 255.0,
            g: g as f32 / 255.0,
            b: b as f32 / 255.0,
            a,
        }
    }

    pub const fn to_array(self) -> [f32; 4] {
        [self.r, self.g, self.b, self.a]
    }
}

/// Dark or light. The light scheme never dims the backdrop, and in it primary is for
/// fills only, never text on a material.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Variant {
    #[default]
    Dark,
    Light,
}

/// The 28 color roles of one scheme. Field names match the keys in `tokens.json`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scheme {
    pub primary: Color,
    pub on_primary: Color,
    pub primary_container: Color,
    pub on_primary_container: Color,
    pub secondary: Color,
    pub on_secondary: Color,
    pub secondary_container: Color,
    pub on_secondary_container: Color,
    pub tertiary: Color,
    pub on_tertiary: Color,
    pub tertiary_container: Color,
    pub on_tertiary_container: Color,
    pub error: Color,
    pub on_error: Color,
    pub error_container: Color,
    pub on_error_container: Color,
    pub surface: Color,
    pub surface_container_lowest: Color,
    pub surface_container_low: Color,
    pub surface_container: Color,
    pub surface_container_high: Color,
    pub surface_container_highest: Color,
    pub on_surface: Color,
    pub on_surface_variant: Color,
    pub outline: Color,
    pub outline_variant: Color,
    pub signal_warn: Color,
    pub signal_info: Color,
}

impl Scheme {
    pub const DARK: Scheme = tokens::DARK;
    pub const LIGHT: Scheme = tokens::LIGHT;

    pub const fn reference(variant: Variant) -> Scheme {
        match variant {
            Variant::Dark => Self::DARK,
            Variant::Light => Self::LIGHT,
        }
    }
}

/// Opacity of the state layer drawn over a control, and of disabled parts.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StateLayer {
    pub hover: f32,
    pub focus: f32,
    pub press: f32,
    pub drag: f32,
    pub disabled_content: f32,
    pub disabled_container: f32,
}

pub const STATE_LAYER: StateLayer = tokens::STATE_LAYER;

/// `color` over `base` at `opacity`, as a state layer is composited.
pub fn over(base: Color, color: Color, opacity: f32) -> Color {
    let a = opacity.clamp(0.0, 1.0) * color.a;
    Color {
        r: base.r + (color.r - base.r) * a,
        g: base.g + (color.g - base.g) * a,
        b: base.b + (color.b - base.b) * a,
        a: base.a + (1.0 - base.a) * a,
    }
}

/// `color` with its alpha multiplied by `alpha`.
pub fn with_alpha(color: Color, alpha: f32) -> Color {
    Color {
        a: color.a * alpha,
        ..color
    }
}

/// WCAG relative luminance of an opaque color.
pub fn luminance(c: Color) -> f32 {
    fn lin(v: f32) -> f32 {
        if v <= 0.040_45 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    }
    0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b)
}

/// WCAG contrast ratio between two opaque colors (1 to 21).
pub fn contrast(a: Color, b: Color) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}
