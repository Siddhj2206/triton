//! How every Triton shape is painted: one routine shared by panels and pressables.
//!
//! Performance on tiny-skia (ADR-0005) decides the split:
//! - A shape whose corners smoothing cannot change (square, pill, circle) or that
//!   floats (it keeps niri's circular window corners) is an iced quad: the fastest
//!   primitive, and the only one with a shadow.
//! - A smoothed shape is a path, built once and kept in a [`Cache`] until its size
//!   or look changes. Animating shapes rebuild it per frame only while they move.
//! - The Glass highlight is a cached 1 px band that follows the top corners.

use std::cell::Cell;

use iced_core::border::Radius;
use iced_core::renderer::Quad;
use iced_core::{Background, Border, Color, Point, Rectangle, Shadow, Size, Vector};
use iced_widget::canvas::{Cache, Frame, Path, Stroke, path::Builder};
use iced_widget::graphics::geometry::Renderer as _;
use triton_design::material::HIGHLIGHT;
use triton_design::shape::{CORNER_SMOOTHING, Segment, outline, top_edge};

use crate::Renderer;

/// What to paint behind a component's content.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) struct Paint {
    pub fill: Option<Color>,
    /// Inner border: color and width.
    pub border: Option<(Color, f32)>,
    /// Top-left, top-right, bottom-right, bottom-left.
    pub radii: [f32; 4],
    /// Ease the corners in (in-shell shapes). Floating shapes stay circular.
    pub smooth: bool,
    pub shadow: Option<Shadow>,
    /// The Glass and Thick 1 px top highlight.
    pub highlight: bool,
}

impl Paint {
    /// Smoothing changes these corners: some corner is round but shorter than half
    /// the short side (pills and circles have no straight edge to ease from).
    fn needs_path(&self, size: Size) -> bool {
        let budget = size.width.min(size.height) / 2.0;
        self.smooth
            && CORNER_SMOOTHING > 0.0
            && self.radii.iter().any(|&r| r > 0.5 && r < budget - 0.5)
    }
}

/// Cached paths for one painted shape. Lives in the widget's tree state.
#[derive(Debug, Default)]
pub(crate) struct PaintCache {
    body: Cache<Renderer>,
    highlight: Cache<Renderer>,
    key: Cell<Option<(Paint, Size)>>,
}

impl PaintCache {
    fn validate(&self, paint: Paint, size: Size) {
        if self.key.get() != Some((paint, size)) {
            self.body.clear();
            self.highlight.clear();
            self.key.set(Some((paint, size)));
        }
    }
}

#[expect(
    clippy::many_single_char_names,
    reason = "x, y, w, h is the box; o the outline, b the builder"
)]
fn path_from(x: f32, y: f32, w: f32, h: f32, radii: [f32; 4]) -> Path {
    let o = outline(x, y, w, h, radii, CORNER_SMOOTHING);
    let mut b = Builder::new();
    b.move_to(o.start);
    for seg in o.segments {
        match seg {
            Segment::Line(p) => b.line_to(p),
            Segment::Cubic(c1, c2, p) => b.bezier_curve_to(c1, c2, p),
            // iced's `arc` begins a new subpath (it moves to the arc's start), which
            // splits the fill; a quarter arc or less is one cubic, accurate to 0.03%.
            Segment::Arc {
                center,
                radius,
                start,
                end,
            } => {
                let k = 4.0 / 3.0 * ((end - start) / 4.0).tan() * radius;
                let at =
                    |a: f32| Point::new(center.x + radius * a.cos(), center.y + radius * a.sin());
                let (p0, p3) = (at(start), at(end));
                let c1 = Point::new(p0.x - k * start.sin(), p0.y + k * start.cos());
                let c2 = Point::new(p3.x + k * end.sin(), p3.y - k * end.cos());
                b.bezier_curve_to(c1, c2, p3);
            }
        }
    }
    b.close();
    b.build()
}

/// The highlight band: the top edge and the same edge 1 px lower, closed.
fn highlight_path(size: Size, radii: [f32; 4], smooth: bool) -> Path {
    let smoothing = if smooth { CORNER_SMOOTHING } else { 0.0 };
    let top = top_edge(size.width, size.height, radii, smoothing, 1.0);
    Path::new(|b| {
        let mut points = top.iter();
        if let Some(first) = points.next() {
            b.move_to(*first);
            for p in points {
                b.line_to(*p);
            }
            for p in top.iter().rev() {
                b.line_to(Point::new(p.x, p.y + 1.0));
            }
            b.close();
        }
    })
}

/// Paints `paint` over `bounds`. Returns true if it drew paths: iced draws a layer's
/// quads before its paths, so the caller must draw its content in a new layer or
/// the path would cover the content's own quads.
#[must_use]
pub(crate) fn draw(
    renderer: &mut Renderer,
    bounds: Rectangle,
    paint: Paint,
    cache: &PaintCache,
) -> bool {
    use iced_core::Renderer as _;
    let size = bounds.size();
    if size.width <= 0.0 || size.height <= 0.0 {
        return false;
    }
    let as_path = paint.needs_path(size) && paint.shadow.is_none();
    if !as_path && (paint.fill.is_some() || paint.border.is_some() || paint.shadow.is_some()) {
        {
            let (color, width) = paint.border.unwrap_or((Color::TRANSPARENT, 0.0));
            let [top_left, top_right, bottom_right, bottom_left] = paint.radii;
            renderer.fill_quad(
                Quad {
                    bounds,
                    border: Border {
                        color,
                        width,
                        radius: Radius {
                            top_left,
                            top_right,
                            bottom_right,
                            bottom_left,
                        },
                    },
                    shadow: paint.shadow.unwrap_or_default(),
                    snap: true,
                },
                Background::Color(paint.fill.unwrap_or(Color::TRANSPARENT)),
            );
        }
    }
    let needs_highlight = paint.highlight;
    if !as_path && !needs_highlight {
        return false;
    }
    cache.validate(paint, size);
    let offset = Vector::new(bounds.x, bounds.y);
    if as_path {
        let body = cache
            .body
            .draw(renderer, size, |frame: &mut Frame<Renderer>| {
                let path = path_from(0.0, 0.0, size.width, size.height, paint.radii);
                if let Some(fill) = paint.fill {
                    frame.fill(&path, fill);
                }
                if let Some((color, width)) = paint.border {
                    let half = width / 2.0;
                    let inset = paint.radii.map(|r| (r - half).max(0.0));
                    let ring =
                        path_from(half, half, size.width - width, size.height - width, inset);
                    frame.stroke(&ring, Stroke::default().with_color(color).with_width(width));
                }
            });
        renderer.with_translation(offset, |r| r.draw_geometry(body));
    }
    if needs_highlight {
        let band = cache
            .highlight
            .draw(renderer, size, |frame: &mut Frame<Renderer>| {
                frame.fill(&highlight_path(size, paint.radii, as_path), HIGHLIGHT);
            });
        renderer.with_translation(offset, |r| r.draw_geometry(band));
    }
    true
}
