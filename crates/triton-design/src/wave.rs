//! The wave: Triton's identity shape, and the wavy progress indicators.
//!
//! The wave is identity only: the launcher button, avatars, album art, swatches and
//! loading. App and file icons sit in circles. Geometry is returned as points in
//! logical px; a canvas widget strokes or fills them.

use std::f32::consts::{PI, TAU};
use std::sync::OnceLock;

use iced_core::Point;

use crate::tokens;

/// Lobes on the wave outline.
pub const LOBES: u32 = tokens::WAVE_LOBES;

/// Wavy linear progress: amplitude and wavelength in logical px.
pub const LINEAR_AMPLITUDE: f32 = 3.0;
pub const LINEAR_WAVELENGTH: f32 = 24.0;
/// Wavy circular progress: amplitude at a 34 px diameter. Scales with the diameter.
pub const CIRCULAR_AMPLITUDE_AT_34: f32 = 1.1;

/// The wave outline in its 100 × 100 design box, parsed once from the tokens.
fn unit_outline() -> &'static [Point] {
    static OUTLINE: OnceLock<Vec<Point>> = OnceLock::new();
    OUTLINE.get_or_init(|| parse_polyline(tokens::WAVE_PATH))
}

/// Parses the `M x,y L x,y ... Z` polyline the tokens use.
fn parse_polyline(path: &str) -> Vec<Point> {
    path.split(['M', 'L', 'Z'])
        .filter_map(|seg| {
            let (x, y) = seg.trim().split_once(',')?;
            Some(Point::new(x.trim().parse().ok()?, y.trim().parse().ok()?))
        })
        .collect()
}

/// The closed wave outline scaled to a `size` × `size` box at `origin`.
pub fn outline(origin: Point, size: f32) -> Vec<Point> {
    let k = size / 100.0;
    unit_outline()
        .iter()
        .map(|p| Point::new(origin.x + p.x * k, origin.y + p.y * k))
        .collect()
}

/// Wavy linear progress from `start`, `width` long, centered on `start.y`. The
/// played part (`0.0..=progress` of the width) is wavy; callers draw the rest as a
/// flat track. `phase` (radians) animates the wave while working.
pub fn wavy_line(start: Point, width: f32, progress: f32, phase: f32) -> Vec<Point> {
    let end = width * progress.clamp(0.0, 1.0);
    // Two samples per px is smooth at any scale the shell uses.
    let steps = (end * 2.0).ceil().max(1.0) as usize;
    (0..=steps)
        .map(|i| {
            let x = end * i as f32 / steps as f32;
            let y = LINEAR_AMPLITUDE * (TAU * x / LINEAR_WAVELENGTH + phase).sin();
            Point::new(start.x + x, start.y + y)
        })
        .collect()
}

/// Wavy circular progress around `center`, starting at 12 o'clock and going
/// clockwise for `progress` of a turn, with [`LOBES`] lobes per full turn.
pub fn wavy_ring(center: Point, diameter: f32, progress: f32, phase: f32) -> Vec<Point> {
    let r = diameter / 2.0;
    let amp = CIRCULAR_AMPLITUDE_AT_34 * diameter / 34.0;
    let sweep = TAU * progress.clamp(0.0, 1.0);
    let steps = (sweep * r).ceil().max(1.0) as usize;
    (0..=steps)
        .map(|i| {
            let a = sweep * i as f32 / steps as f32;
            let rr = r + amp * (LOBES as f32 * a + phase).sin();
            let theta = a - PI / 2.0;
            Point::new(center.x + rr * theta.cos(), center.y + rr * theta.sin())
        })
        .collect()
}
