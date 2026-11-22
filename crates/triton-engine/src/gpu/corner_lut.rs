//! Exact Figma smooth-corner signed distance field (SDF) lookup table.
//!
//! Bakes a 2D `R8` texture over `[0, p] × [0, p]` in normalized corner-radius
//! units (`r = 1.0`, `p = 1.0 + smoothing`), where `(0, 0)` is the outer box
//! corner and `(+x, +y)` extends into the box interior.

#![forbid(unsafe_code)]

use std::f32::consts::{FRAC_PI_2, FRAC_PI_4, SQRT_2};

/// Width and height of the square `R8` corner LUT texture in texels.
pub const LUT_SIZE: u32 = 256;

/// Signed distance range encoded into `[0, 255]`, covering `[-LUT_DIST_RANGE, +LUT_DIST_RANGE]`
/// in normalized corner-radius units (`r = 1.0`).
pub const LUT_DIST_RANGE: f32 = 1.0;

/// Number of polyline subdivisions per curve piece (cubic 1, arc, cubic 2).
const SEGMENTS_PER_PIECE: usize = 64;

/// Extent `p = 1.0 + smoothing` (in units of corner radius `r`) along each edge
/// covered by the LUT for the given `smoothing` factor in `[0.0, 1.0]`.
#[must_use]
pub fn lut_extent(smoothing: f32) -> f32 {
    1.0 + smoothing.clamp(0.0, 1.0)
}

/// Decodes an `R8` LUT texel back into a signed distance in normalized corner-radius
/// units (`r = 1.0`), negative inside the shape and positive outside.
#[must_use]
pub fn decode_lut_distance(texel: u8) -> f32 {
    ((f32::from(texel) / 255.0) - 0.5) * 2.0 * LUT_DIST_RANGE
}

/// Encodes a signed distance in normalized corner-radius units into an `R8` texel.
fn encode_lut_distance(dist: f32) -> u8 {
    let norm = ((dist / LUT_DIST_RANGE) * 0.5 + 0.5).clamp(0.0, 1.0);
    (norm * 255.0).round() as u8
}

/// Bakes a `LUT_SIZE × LUT_SIZE` row-major `R8` signed distance field for a
/// Figma smooth corner with the given `smoothing` in `[0.0, 1.0]`.
///
/// Row `y`, column `x` (`0..LUT_SIZE`) corresponds to normalized coordinates
/// `(x / (LUT_SIZE - 1) * p, y / (LUT_SIZE - 1) * p)` where `p = lut_extent(smoothing)`.
#[must_use]
pub fn bake_smooth_corner_lut(smoothing: f32) -> Vec<u8> {
    let s = smoothing.clamp(0.0, 1.0);
    let extent = lut_extent(s);
    let polyline = sample_corner_polyline(s);

    let size = LUT_SIZE as usize;
    let denom = (LUT_SIZE - 1) as f32;
    let mut data = Vec::with_capacity(size * size);

    for row in 0..size {
        let qy = (row as f32 / denom) * extent;
        for col in 0..size {
            let qx = (col as f32 / denom) * extent;
            let dist = signed_distance_to_corner(qx, qy, &polyline);
            data.push(encode_lut_distance(dist));
        }
    }

    data
}

/// Samples the exact Figma smooth-corner curve (for `r = 1.0`, unclamped `s`)
/// from `(p, 0)` to `(0, p)` into a dense polyline.
///
/// The resulting points have monotonically non-increasing `x` and monotonically
/// non-decreasing `y`.
#[expect(
    clippy::many_single_char_names,
    reason = "the figma-squircle construction names its lengths p, a, b, c, d and r"
)]
fn sample_corner_polyline(s: f32) -> Vec<(f32, f32)> {
    let r = 1.0_f32;
    let p = 1.0 + s;
    let sweep = FRAC_PI_2 * (1.0 - s);
    let arc_len = (sweep / 2.0).sin() * r * SQRT_2;
    let alpha = (FRAC_PI_2 - sweep) / 2.0;
    let beta = FRAC_PI_4 * s;
    let c = r * (alpha / 2.0).tan() * beta.cos();
    let d = c * beta.tan();
    let b = (p - arc_len - c - d) / 3.0;
    let a = 2.0 * b;

    let x1 = p - a - b - c;
    let y1 = d;
    let x2 = x1 - arc_len;
    let y2 = y1 + arc_len;

    let mut pts = Vec::with_capacity(SEGMENTS_PER_PIECE * 3 + 1);
    pts.push((p, 0.0));

    // Cubic 1: (p, 0) -> (x1, y1)
    if s > 0.0 {
        let p0 = (p, 0.0);
        let c1 = (p - a, 0.0);
        let c2 = (p - a - b, 0.0);
        let p1 = (x1, y1);
        for i in 1..=SEGMENTS_PER_PIECE {
            let t = i as f32 / SEGMENTS_PER_PIECE as f32;
            pts.push(eval_cubic(p0, c1, c2, p1, t));
        }
    }

    // Circular arc centered at (1, 1) with radius 1, from (x1, y1) to (x2, y2).
    if sweep > 0.0 {
        let start = (y1 - r).atan2(x1 - r);
        for i in 1..=SEGMENTS_PER_PIECE {
            let t = i as f32 / SEGMENTS_PER_PIECE as f32;
            let angle = start - sweep * t;
            pts.push((r + r * angle.cos(), r + r * angle.sin()));
        }
    }

    // Cubic 2: (x2, y2) -> (0, p)
    if s > 0.0 {
        let p2 = (x2, y2);
        let c1 = (x2 - d, y2 + c);
        let c2 = (x2 - d, y2 + b + c);
        let p3 = (0.0, p);
        for i in 1..=SEGMENTS_PER_PIECE {
            let t = i as f32 / SEGMENTS_PER_PIECE as f32;
            pts.push(eval_cubic(p2, c1, c2, p3, t));
        }
    }

    pts
}

fn eval_cubic(
    p0: (f32, f32),
    c1: (f32, f32),
    c2: (f32, f32),
    p3: (f32, f32),
    t: f32,
) -> (f32, f32) {
    let m = 1.0 - t;
    let w0 = m * m * m;
    let w1 = 3.0 * m * m * t;
    let w2 = 3.0 * m * t * t;
    let w3 = t * t * t;
    (
        w0 * p0.0 + w1 * c1.0 + w2 * c2.0 + w3 * p3.0,
        w0 * p0.1 + w1 * c1.1 + w2 * c2.1 + w3 * p3.1,
    )
}

/// Computes the exact signed distance from `(qx, qy)` to the corner polyline.
/// Negative inside the shape, positive outside toward `(0, 0)`.
fn signed_distance_to_corner(qx: f32, qy: f32, polyline: &[(f32, f32)]) -> f32 {
    let mut min_dist_sq = f32::INFINITY;
    for seg in polyline.windows(2) {
        let d2 = point_segment_dist_sq(qx, qy, seg[0], seg[1]);
        if d2 < min_dist_sq {
            min_dist_sq = d2;
        }
    }
    let dist = min_dist_sq.sqrt();
    let y_curve = curve_y_at_x(qx, polyline);
    if qy >= y_curve { -dist } else { dist }
}

/// Squared Euclidean distance from point `(px, py)` to segment `a..b`.
fn point_segment_dist_sq(px: f32, py: f32, a: (f32, f32), b: (f32, f32)) -> f32 {
    let dx = b.0 - a.0;
    let dy = b.1 - a.1;
    let len_sq = dx * dx + dy * dy;
    let t = if len_sq > 0.0 {
        (((px - a.0) * dx + (py - a.1) * dy) / len_sq).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let cx = a.0 + t * dx;
    let cy = a.1 + t * dy;
    let ex = px - cx;
    let ey = py - cy;
    ex * ex + ey * ey
}

/// Evaluates `y(x)` on the monotone polyline (sorted by non-increasing `x`).
fn curve_y_at_x(qx: f32, polyline: &[(f32, f32)]) -> f32 {
    let Some(&(first_x, first_y)) = polyline.first() else {
        return 0.0;
    };
    if qx >= first_x {
        return first_y;
    }
    let Some(&(last_x, last_y)) = polyline.last() else {
        return 0.0;
    };
    if qx <= last_x {
        return last_y;
    }

    // Binary search for the segment [lo, lo + 1] with polyline[lo].0 >= qx >= polyline[lo + 1].0.
    let mut lo = 0_usize;
    let mut hi = polyline.len() - 1;
    while hi - lo > 1 {
        let mid = lo + (hi - lo) / 2;
        if polyline[mid].0 >= qx {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let (x0, y0) = polyline[lo];
    let (x1, y1) = polyline[hi];
    let span = x0 - x1;
    if span.abs() < 1e-12 {
        return f32::midpoint(y0, y1);
    }
    let t = ((x0 - qx) / span).clamp(0.0, 1.0);
    y0 + t * (y1 - y0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_SMOOTHING: f32 = 0.6;

    #[test]
    fn lut_is_symmetric() {
        let lut = bake_smooth_corner_lut(TEST_SMOOTHING);
        let n = LUT_SIZE as usize;
        assert_eq!(lut.len(), n * n);

        for y in 0..n {
            for x in (y + 1)..n {
                let a = lut[y * n + x];
                let b = lut[x * n + y];
                assert!(a.abs_diff(b) <= 1, "asymmetry at ({x}, {y}): {a} vs {b}");
            }
        }
    }

    #[test]
    fn outer_corner_is_outside_and_interior_is_inside() {
        let lut = bake_smooth_corner_lut(TEST_SMOOTHING);
        let n = LUT_SIZE as usize;

        let d_outer = decode_lut_distance(lut[0]);
        assert!(
            d_outer > 0.35 && d_outer < 0.45,
            "expected outer corner distance ~0.38..0.42, got {d_outer}"
        );

        let d_inner = decode_lut_distance(lut[n * n - 1]);
        assert!(
            d_inner < 0.0,
            "expected interior corner (p, p) to be inside (d < 0), got {d_inner}"
        );
    }

    #[test]
    fn endpoints_have_zero_distance() {
        let lut = bake_smooth_corner_lut(TEST_SMOOTHING);
        let n = LUT_SIZE as usize;

        let d_horiz = decode_lut_distance(lut[n - 1]);
        let d_vert = decode_lut_distance(lut[(n - 1) * n]);
        assert!(
            d_horiz.abs() < 0.02,
            "expected (p, 0) to be on curve (|d| < 0.02), got {d_horiz}"
        );
        assert!(
            d_vert.abs() < 0.02,
            "expected (0, p) to be on curve (|d| < 0.02), got {d_vert}"
        );
    }

    #[test]
    fn zero_smoothing_matches_quarter_circle() {
        let lut = bake_smooth_corner_lut(0.0);
        let d_outer = decode_lut_distance(lut[0]);
        let expected = SQRT_2 - 1.0;
        assert!(
            (d_outer - expected).abs() < 0.02,
            "expected quarter-circle outer distance {expected}, got {d_outer}"
        );
    }
}
