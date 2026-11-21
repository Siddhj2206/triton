//! Corner radii, spacing and component sizes, in logical px.
//!
//! Tiled windows are square with no gaps. Radius, shadow and blur belong to things
//! that float.

use iced_core::Point;

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

/// How much in-shell corners ease into their curve (Figma's "corner smoothing",
/// Apple's continuous corners). 0 is a plain quarter circle.
pub const CORNER_SMOOTHING: f32 = tokens::CORNER_SMOOTHING;

/// One piece of an [`Outline`], continuing from the previous point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Segment {
    Line(Point),
    /// Cubic Bézier: two control points, then the end point.
    Cubic(Point, Point, Point),
    /// A circular arc, clockwise on screen (y down), from `start` to `end` radians.
    Arc {
        center: Point,
        radius: f32,
        start: f32,
        end: f32,
    },
}

/// A closed rounded-rectangle outline, clockwise from `start` (on the top edge).
#[derive(Debug, Clone, PartialEq)]
pub struct Outline {
    pub start: Point,
    pub segments: Vec<Segment>,
}

/// The measurements of one smoothed corner (the figma-squircle construction): the
/// corner takes `p` of each edge; a cubic eases from the edge into a shorter arc,
/// and a mirrored cubic eases out.
#[derive(Debug, Clone, Copy)]
struct Corner {
    r: f32,
    p: f32,
    a: f32,
    b: f32,
    c: f32,
    d: f32,
    arc_len: f32,
    sweep: f32,
}

#[expect(
    clippy::many_single_char_names,
    reason = "the figma-squircle construction names its lengths p, a, b, c, d and r"
)]
impl Corner {
    /// `budget` is the most of each edge one corner may use (half the short side).
    /// Smoothing needs edge to borrow; when there is none (a pill) it falls back to
    /// a plain arc.
    fn new(r: f32, smoothing: f32, budget: f32) -> Corner {
        use std::f32::consts::{FRAC_PI_2, FRAC_PI_4, SQRT_2};
        let r = r.clamp(0.0, budget.max(0.0));
        let s = if r > 0.0 {
            smoothing.min(budget / r - 1.0).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let p = ((1.0 + s) * r).min(budget);
        let sweep = FRAC_PI_2 * (1.0 - s);
        let arc_len = (sweep / 2.0).sin() * r * SQRT_2;
        let alpha = (FRAC_PI_2 - sweep) / 2.0;
        let beta = FRAC_PI_4 * s;
        let c = r * (alpha / 2.0).tan() * beta.cos();
        let d = c * beta.tan();
        let b = (p - arc_len - c - d) / 3.0;
        Corner {
            r,
            p,
            a: 2.0 * b,
            b,
            c,
            d,
            arc_len,
            sweep,
        }
    }

    /// Appends this corner, drawn at `k` with incoming edge direction `u` and
    /// outgoing `v`, starting from its point on the incoming edge.
    fn push(&self, k: Point, u: (f32, f32), v: (f32, f32), out: &mut Vec<Segment>) {
        let at = |x: f32, y: f32| Point::new(k.x + x * u.0 + y * v.0, k.y + x * u.1 + y * v.1);
        if self.r <= 0.0 {
            out.push(Segment::Line(k));
            return;
        }
        let Corner {
            p, a, b, c, d, r, ..
        } = *self;
        let (x1, y1) = (-p + a + b + c, d);
        if self.sweep < std::f32::consts::FRAC_PI_2 {
            out.push(Segment::Cubic(
                at(-p + a, 0.0),
                at(-p + a + b, 0.0),
                at(x1, y1),
            ));
        }
        let center = at(-r, r);
        let p1 = at(x1, y1);
        let start = (p1.y - center.y).atan2(p1.x - center.x);
        out.push(Segment::Arc {
            center,
            radius: r,
            start,
            end: start + self.sweep,
        });
        if self.sweep < std::f32::consts::FRAC_PI_2 {
            let (x2, y2) = (x1 + self.arc_len, y1 + self.arc_len);
            out.push(Segment::Cubic(
                at(x2 + d, y2 + c),
                at(x2 + d, y2 + b + c),
                at(0.0, p),
            ));
        }
    }
}

/// A corner: radius index, the corner point, incoming and outgoing directions.
type CornerFrame = (usize, Point, (f32, f32), (f32, f32));

/// The four corners of `rect` in drawing order (top-right, bottom-right,
/// bottom-left, top-left) with their incoming and outgoing directions, paired with
/// the radius index in `[top_left, top_right, bottom_right, bottom_left]`.
fn corners(x: f32, y: f32, w: f32, h: f32) -> [CornerFrame; 4] {
    [
        (1, Point::new(x + w, y), (1.0, 0.0), (0.0, 1.0)),
        (2, Point::new(x + w, y + h), (0.0, 1.0), (-1.0, 0.0)),
        (3, Point::new(x, y + h), (-1.0, 0.0), (0.0, -1.0)),
        (0, Point::new(x, y), (0.0, -1.0), (1.0, 0.0)),
    ]
}

/// A rounded rectangle at (`x`, `y`), `w` × `h`, with per-corner `radii` (top-left,
/// top-right, bottom-right, bottom-left) and corner `smoothing` (0 to 1).
#[expect(
    clippy::many_single_char_names,
    reason = "x, y, w, h is the box; c a corner"
)]
pub fn outline(x: f32, y: f32, w: f32, h: f32, radii: [f32; 4], smoothing: f32) -> Outline {
    let budget = w.min(h) / 2.0;
    let corner = |i: usize| Corner::new(radii[i], smoothing, budget);
    let mut segments = Vec::with_capacity(16);
    for (i, k, u, v) in corners(x, y, w, h) {
        let c = corner(i);
        segments.push(Segment::Line(Point::new(k.x - c.p * u.0, k.y - c.p * u.1)));
        c.push(k, u, v, &mut segments);
    }
    Outline {
        start: Point::new(x + corner(0).p, y),
        segments,
    }
}

/// The top of an outline, from where the top-left corner leaves the left edge to
/// where the top-right corner meets the right edge, as points about `step` px apart.
/// The Glass highlight is the band between this and itself moved down 1 px.
pub fn top_edge(w: f32, h: f32, radii: [f32; 4], smoothing: f32, step: f32) -> Vec<Point> {
    let budget = w.min(h) / 2.0;
    let tl = Corner::new(radii[0], smoothing, budget);
    let tr = Corner::new(radii[1], smoothing, budget);
    let mut segments = Vec::with_capacity(8);
    tl.push(Point::ORIGIN, (0.0, -1.0), (1.0, 0.0), &mut segments);
    segments.push(Segment::Line(Point::new(w - tr.p, 0.0)));
    tr.push(Point::new(w, 0.0), (1.0, 0.0), (0.0, 1.0), &mut segments);
    flatten(Point::new(0.0, tl.p), &segments, step)
}

/// Samples segments into points about `step` px apart.
#[expect(
    clippy::many_single_char_names,
    reason = "Bezier terms: t, its complement m, weights w, sample count n"
)]
pub fn flatten(start: Point, segments: &[Segment], step: f32) -> Vec<Point> {
    let step = step.max(0.25);
    let mut points = vec![start];
    let mut at = start;
    for seg in segments {
        match *seg {
            Segment::Line(p) => {
                points.push(p);
                at = p;
            }
            Segment::Cubic(c1, c2, p) => {
                let len = at.distance(c1) + c1.distance(c2) + c2.distance(p);
                let n = (len / step).ceil().max(1.0) as usize;
                for i in 1..=n {
                    let t = i as f32 / n as f32;
                    let m = 1.0 - t;
                    let w = [m * m * m, 3.0 * m * m * t, 3.0 * m * t * t, t * t * t];
                    points.push(Point::new(
                        w[0] * at.x + w[1] * c1.x + w[2] * c2.x + w[3] * p.x,
                        w[0] * at.y + w[1] * c1.y + w[2] * c2.y + w[3] * p.y,
                    ));
                }
                at = p;
            }
            Segment::Arc {
                center,
                radius,
                start,
                end,
            } => {
                let n = ((end - start).abs() * radius / step).ceil().max(1.0) as usize;
                for i in 1..=n {
                    let a = start + (end - start) * i as f32 / n as f32;
                    points.push(Point::new(
                        center.x + radius * a.cos(),
                        center.y + radius * a.sin(),
                    ));
                }
                at = *points.last().unwrap_or(&at);
            }
        }
    }
    points
}
