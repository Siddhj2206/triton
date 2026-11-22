//! Core geometry, color, typography, and material styling primitives.

use triton_design::color::{Color, Scheme, Variant, over, with_alpha};
use triton_design::material::{HIGHLIGHT, Material, ShadowToken};
use triton_design::typography::{Family, TypeStyle};

/// RGBA color in straight (non-premultiplied) 0.0..=1.0 float channels.
#[must_use]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Rgba {
    pub const TRANSPARENT: Self = Self::new(0.0, 0.0, 0.0, 0.0);
    pub const WHITE: Self = Self::new(1.0, 1.0, 1.0, 1.0);
    pub const BLACK: Self = Self::new(0.0, 0.0, 0.0, 1.0);

    pub const fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    pub const fn from_color(c: Color) -> Self {
        Self {
            r: c.r,
            g: c.g,
            b: c.b,
            a: c.a,
        }
    }

    pub const fn with_alpha(self, alpha: f32) -> Self {
        Self {
            a: self.a * alpha,
            ..self
        }
    }

    pub const fn to_array(self) -> [f32; 4] {
        [self.r, self.g, self.b, self.a]
    }

    pub fn lerp(self, other: Self, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        Self {
            r: self.r + (other.r - self.r) * t,
            g: self.g + (other.g - self.g) * t,
            b: self.b + (other.b - self.b) * t,
            a: self.a + (other.a - self.a) * t,
        }
    }

    /// Composite `overlay` over `self` at `opacity`.
    pub fn over(self, overlay: Self, opacity: f32) -> Self {
        let base = Color::from_rgba(self.r, self.g, self.b, self.a);
        let top = Color::from_rgba(overlay.r, overlay.g, overlay.b, overlay.a);
        Self::from_color(over(base, top, opacity))
    }
}

impl From<Color> for Rgba {
    fn from(c: Color) -> Self {
        Self::from_color(c)
    }
}

impl From<[f32; 4]> for Rgba {
    fn from([r, g, b, a]: [f32; 4]) -> Self {
        Self { r, g, b, a }
    }
}

/// Logical-pixel axis-aligned rectangle.
#[must_use]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    pub fn contains(self, px: f32, py: f32) -> bool {
        px >= self.x && px <= self.x + self.w && py >= self.y && py <= self.y + self.h
    }

    pub fn intersect(self, other: Self) -> Self {
        let x0 = self.x.max(other.x);
        let y0 = self.y.max(other.y);
        let x1 = (self.x + self.w).min(other.x + other.w);
        let y1 = (self.y + self.h).min(other.y + other.h);
        Self {
            x: x0,
            y: y0,
            w: (x1 - x0).max(0.0),
            h: (y1 - y0).max(0.0),
        }
    }

    pub fn is_empty(self) -> bool {
        self.w <= 0.0 || self.h <= 0.0
    }
}

/// Per-corner radii in logical px: `[top_left, top_right, bottom_right, bottom_left]`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Corners(pub [f32; 4]);

impl Corners {
    pub const ZERO: Self = Self([0.0; 4]);

    pub const fn all(r: f32) -> Self {
        Self([r, r, r, r])
    }

    pub const fn each(tl: f32, tr: f32, br: f32, bl: f32) -> Self {
        Self([tl, tr, br, bl])
    }

    /// Clamp radii to half the shortest side so pills (`RADII.full`) render cleanly.
    pub fn clamped(self, w: f32, h: f32) -> [f32; 4] {
        let max_r = (w.min(h) * 0.5).max(0.0);
        self.0.map(|r| r.clamp(0.0, max_r))
    }

    /// Representative single radius for Wayland `wl_region` blur/input banding.
    pub fn max_clamped(self, w: f32, h: f32) -> f32 {
        let c = self.clamped(w, h);
        c[0].max(c[1]).max(c[2]).max(c[3])
    }
}

impl From<f32> for Corners {
    fn from(r: f32) -> Self {
        Self::all(r)
    }
}

impl From<[f32; 4]> for Corners {
    fn from(r: [f32; 4]) -> Self {
        Self(r)
    }
}

/// Edge insets (padding / margin) in logical px.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Edges {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

impl Edges {
    pub const ZERO: Self = Self::all(0.0);

    pub const fn all(v: f32) -> Self {
        Self {
            top: v,
            right: v,
            bottom: v,
            left: v,
        }
    }

    pub const fn xy(horizontal: f32, vertical: f32) -> Self {
        Self {
            top: vertical,
            right: horizontal,
            bottom: vertical,
            left: horizontal,
        }
    }

    pub const fn new(top: f32, right: f32, bottom: f32, left: f32) -> Self {
        Self {
            top,
            right,
            bottom,
            left,
        }
    }
}

impl From<f32> for Edges {
    fn from(v: f32) -> Self {
        Self::all(v)
    }
}

/// Font styling for a text run.
#[must_use]
#[derive(Debug, Clone, PartialEq)]
pub struct FontSpec {
    pub family: &'static str,
    pub size: f32,
    pub line_height: f32,
    pub weight: u16,
    /// Extra letter spacing in logical px.
    pub letter_spacing: f32,
    pub uppercase: bool,
}

impl FontSpec {
    pub const fn ui(size: f32) -> Self {
        Self {
            family: Family::Ui.name(),
            size,
            line_height: size * 1.35,
            weight: 400,
            letter_spacing: 0.0,
            uppercase: false,
        }
    }

    pub const fn display(size: f32) -> Self {
        Self {
            family: Family::Display.name(),
            size,
            line_height: size * 1.15,
            weight: 600,
            letter_spacing: 0.0,
            uppercase: false,
        }
    }

    pub const fn mono(size: f32) -> Self {
        Self {
            family: Family::Mono.name(),
            size,
            line_height: size * 1.4,
            weight: 500,
            letter_spacing: 0.0,
            uppercase: false,
        }
    }

    pub const fn from_type_style(ts: TypeStyle) -> Self {
        Self {
            family: ts.family.name(),
            size: ts.size,
            line_height: ts.line_height,
            weight: ts.weight.numeric(),
            letter_spacing: ts.tracking * ts.size,
            uppercase: ts.uppercase,
        }
    }

    pub const fn weight(mut self, weight: u16) -> Self {
        self.weight = weight;
        self
    }

    pub const fn line_height(mut self, lh: f32) -> Self {
        self.line_height = lh;
        self
    }
}

impl From<TypeStyle> for FontSpec {
    fn from(ts: TypeStyle) -> Self {
        Self::from_type_style(ts)
    }
}

/// Drop shadow specification for SDF shadow quads.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DropShadow {
    pub offset_x: f32,
    pub offset_y: f32,
    pub blur: f32,
    pub spread: f32,
    pub color: Rgba,
}

impl DropShadow {
    /// Minimum transparent gutter (`Edges`) around a caster so the drop shadow
    /// reaches zero alpha cleanly without clipping against a layer surface edge.
    #[must_use]
    pub fn gutter(self) -> Edges {
        let pad = (self.blur * 2.0 + self.spread).max(0.0);
        Edges::new(
            (pad - self.offset_y).max(0.0).ceil(),
            (pad + self.offset_x).max(0.0).ceil(),
            (pad + self.offset_y).max(0.0).ceil(),
            (pad - self.offset_x).max(0.0).ceil(),
        )
    }
}

impl From<ShadowToken> for DropShadow {
    fn from(tok: ShadowToken) -> Self {
        Self {
            offset_x: tok.x,
            offset_y: tok.y,
            blur: tok.blur,
            spread: tok.spread,
            color: Rgba::from_color(tok.color),
        }
    }
}

/// Active theme bundle passed down during view construction.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Theme {
    pub scheme: Scheme,
    pub variant: Variant,
}

impl Default for Theme {
    fn default() -> Self {
        Self::dark()
    }
}

impl Theme {
    pub const fn dark() -> Self {
        Self {
            scheme: Scheme::DARK,
            variant: Variant::Dark,
        }
    }

    pub const fn light() -> Self {
        Self {
            scheme: Scheme::LIGHT,
            variant: Variant::Light,
        }
    }

    /// Fill color for a Triton material (`Solid`, `Mica`, `Glass`, `Thick`).
    pub fn material_fill(&self, material: Material) -> Rgba {
        let base = match material {
            Material::Solid | Material::Mica => self.scheme.surface_container_low,
            Material::Glass | Material::Thick => self.scheme.surface_container_high,
        };
        match material.spec(self.variant) {
            Some(spec) => Rgba::from_color(with_alpha(base, spec.alpha)),
            None => Rgba::from_color(base),
        }
    }

    /// 1px inner top highlight for `Glass` and `Thick` materials.
    pub fn material_highlight(&self, material: Material) -> Option<Rgba> {
        if material.floats() {
            let alpha = match self.variant {
                Variant::Dark => HIGHLIGHT.a,
                Variant::Light => 0.55,
            };
            Some(Rgba::new(1.0, 1.0, 1.0, alpha))
        } else {
            None
        }
    }

    /// Standard subtle outer/inner hairline outline for floating surfaces.
    pub fn material_border(&self, material: Material) -> Option<Rgba> {
        if material.floats() {
            let c = match self.variant {
                Variant::Dark => Rgba::new(1.0, 1.0, 1.0, 0.08),
                Variant::Light => Rgba::new(0.0, 0.0, 0.0, 0.08),
            };
            Some(c)
        } else {
            None
        }
    }

    /// Veil fill for controls sitting on Glass or Thick Glass.
    pub fn veil(&self) -> Rgba {
        Rgba::from_color(triton_design::material::veil(self.variant))
    }

    /// Emphasized veil fill (hover / pressed).
    pub fn veil_hi(&self) -> Rgba {
        Rgba::from_color(triton_design::material::veil_hi(self.variant))
    }
}

/// A 2D scale + translate + opacity transform applied to a subtree during paint.
#[must_use]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SubtreeTransform {
    pub translate_x: f32,
    pub translate_y: f32,
    pub scale: f32,
    pub opacity: f32,
}

impl Default for SubtreeTransform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl SubtreeTransform {
    pub const IDENTITY: Self = Self {
        translate_x: 0.0,
        translate_y: 0.0,
        scale: 1.0,
        opacity: 1.0,
    };

    pub fn is_identity(&self) -> bool {
        self.translate_x.abs() < 1e-4
            && self.translate_y.abs() < 1e-4
            && (self.scale - 1.0).abs() < 1e-4
            && (self.opacity - 1.0).abs() < 1e-4
    }

    /// Compose `child` inside `self` around the child's center `(cx, cy)`.
    pub fn compose_about(self, child: Self, cx: f32, cy: f32) -> Self {
        // Transform a point p:
        //   p' = center + (p - center) * child.scale + child.translate
        //      = p * child.scale + center * (1 - child.scale) + child.translate
        // Then apply self:
        //   p'' = p' * self.scale + self.translate
        let s = self.scale * child.scale;
        let tx = self.translate_x + self.scale * (cx * (1.0 - child.scale) + child.translate_x);
        let ty = self.translate_y + self.scale * (cy * (1.0 - child.scale) + child.translate_y);
        Self {
            translate_x: tx,
            translate_y: ty,
            scale: s,
            opacity: (self.opacity * child.opacity).clamp(0.0, 1.0),
        }
    }

    pub fn map_rect(self, r: Rect) -> Rect {
        Rect {
            x: self.translate_x + r.x * self.scale,
            y: self.translate_y + r.y * self.scale,
            w: r.w * self.scale,
            h: r.h * self.scale,
        }
    }
}
