//! Corner radii, spacing and component sizes, in logical px.
//!
//! Tiled windows are square with no gaps. Radius, shadow and blur belong to things
//! that float.

use crate::tokens;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Radii {
    pub tiled: f32,
    pub xs: f32,
    pub sm: f32,
    pub md: f32,
    pub lg: f32,
    pub xl: f32,
    /// A pill: clamp to half the height when drawing.
    pub full: f32,
    pub floating_window: f32,
    /// Outer corners of the first and last row in a grouped list.
    pub list_outer: f32,
    /// Corners between rows in a grouped list.
    pub list_inner: f32,
    /// Outer ends of a connected button group.
    pub connected_end: f32,
    /// Inner joins of a connected button group.
    pub connected_inner: f32,
}

pub const RADII: Radii = tokens::RADII;

/// The spacing scale: 4, 8, 12, 16, 20, 24, 32, 40, 48, 56.
pub const SPACE: [f32; 10] = tokens::SPACE;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Size {
    pub control_s: f32,
    pub control_m: f32,
    pub control_l: f32,
    pub control_xl: f32,
    pub touch_min: f32,
    pub icon_s: f32,
    pub icon_m: f32,
    pub icon_l: f32,
    pub icon_xl: f32,
    /// Height of the bar and of each island.
    pub bar_height: f32,
    /// Switch track (width, height).
    pub switch: (f32, f32),
    pub slider_track: f32,
    /// Slider handle (width, height).
    pub slider_handle: (f32, f32),
    pub search_field: f32,
    pub list_row_min: f32,
}

pub const SIZE: Size = tokens::SIZE;

/// Per-corner radii for item `index` of `count` in a grouped list: outer corners on
/// the ends, inner corners between rows. Order: top-left, top-right, bottom-right,
/// bottom-left (iced's `border::Radius` order).
pub fn list_corners(index: usize, count: usize) -> [f32; 4] {
    let (o, i) = (RADII.list_outer, RADII.list_inner);
    let top = if index == 0 { o } else { i };
    let bottom = if index + 1 >= count { o } else { i };
    [top, top, bottom, bottom]
}

/// Per-corner radii for segment `index` of `count` in a horizontal connected group.
/// `end` is the outer radius (`RADII.connected_end`, or half the height for the
/// large variant). Order: top-left, top-right, bottom-right, bottom-left.
pub fn connected_corners(index: usize, count: usize, end: f32) -> [f32; 4] {
    let i = RADII.connected_inner;
    let left = if index == 0 { end } else { i };
    let right = if index + 1 >= count { end } else { i };
    [left, right, right, left]
}

/// The outer radius that makes a container's corner concentric with the shape it
/// wraps, `inset` px in: `inner + inset`. Both curves then share a center, so the
/// gap between them stays the same width all the way around the corner.
///
/// Use it where a container tightly wraps one control and the two corners are meant
/// to read as one (a toolbar panel around a segmented group, a track around its
/// thumb). It is not for loose layout: a card's corner stays its own token no matter
/// what sits 20 px inside it, as in Material.
pub fn concentric(inner: f32, inset: f32) -> f32 {
    inner + inset
}

/// The inverse of [`concentric`]: the radius for a shape `inset` px inside a corner of
/// radius `outer`, never below zero.
pub fn nested(outer: f32, inset: f32) -> f32 {
    (outer - inset).max(0.0)
}
