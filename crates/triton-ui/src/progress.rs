//! Progress indicators and the wave shape, drawn on iced's canvas.
//!
//! - Wavy linear: the done part is a wave (amplitude 3, wavelength 24), then a 4 px
//!   gap, then a flat track. Height 14.
//! - Flat linear: an 8 px bar, for quiet places (storage cards).
//! - Wavy circular: a wavy arc over a flat arc, 48 px box with a 34 px ring.
//! - The wave shape: Triton's identity blob (launcher, avatars, album art).
//!
//! `phase` (radians) moves the wave; the surface advances it while work is running.
//! Under reduce motion the surface keeps it still.

use iced_core::{Color, Element, Length, Point, Rectangle, mouse};
use iced_widget::canvas::{self, Frame, Geometry, LineCap, LineJoin, Path, Stroke};
use triton_design::wave;

use crate::{Renderer, Theme};

/// Stroke width of every progress line.
const STROKE: f32 = 4.0;
/// Visible gap between the done part and the track. The round caps each reach
/// `STROKE / 2` past the ends, so the ends sit `GAP + STROKE` apart.
const GAP: f32 = 4.0;
/// Wavy linear height: room for the wave's swing plus the stroke.
const LINEAR_HEIGHT: f32 = 14.0;
/// Wavy circular box and ring diameter.
const CIRCULAR_BOX: f32 = 48.0;
const CIRCULAR_RING: f32 = 34.0;
/// Flat linear height.
const FLAT_HEIGHT: f32 = 8.0;

/// Which color the wave shape takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaveTone {
    /// The launcher button.
    Primary,
    /// Avatars and accents.
    Tertiary,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Shape {
    WavyLinear { progress: f32, phase: f32 },
    FlatLinear { progress: f32 },
    WavyCircular { progress: f32, phase: f32 },
    Wave(WaveTone),
}

#[derive(Debug, Clone, Copy)]
struct Program(Shape);

fn stroke(color: Color) -> Stroke<'static> {
    Stroke::default()
        .with_color(color)
        .with_width(STROKE)
        .with_line_cap(LineCap::Round)
        .with_line_join(LineJoin::Round)
}

fn polyline(points: &[Point]) -> Path {
    Path::new(|b| {
        if let Some((first, rest)) = points.split_first() {
            b.move_to(*first);
            for p in rest {
                b.line_to(*p);
            }
        }
    })
}

fn draw_wavy_linear(frame: &mut Frame<Renderer>, theme: &Theme, progress: f32, phase: f32) {
    let s = &theme.scheme;
    let size = frame.size();
    let cap = STROKE / 2.0;
    let y = size.height / 2.0;
    let width = size.width - STROKE;
    let done = width * progress.clamp(0.0, 1.0);
    if done > 0.0 {
        let points = wave::wavy_line(Point::new(cap, y), width, progress, phase);
        frame.stroke(&polyline(&points), stroke(s.primary));
    }
    let track_start = if done > 0.0 {
        cap + done + GAP + STROKE
    } else {
        cap
    };
    let track_end = size.width - cap;
    if track_end > track_start {
        let line = Path::line(Point::new(track_start, y), Point::new(track_end, y));
        frame.stroke(&line, stroke(s.secondary_container));
    }
}

fn draw_flat_linear(frame: &mut Frame<Renderer>, theme: &Theme, progress: f32) {
    let s = &theme.scheme;
    let size = frame.size();
    let r = FLAT_HEIGHT / 2.0;
    let track = Path::rounded_rectangle(Point::ORIGIN, size, r.into());
    frame.fill(&track, s.secondary_container);
    let done = size.width * progress.clamp(0.0, 1.0);
    if done > 0.0 {
        let bar = Path::rounded_rectangle(
            Point::ORIGIN,
            iced_core::Size::new(done.max(FLAT_HEIGHT), size.height),
            r.into(),
        );
        frame.fill(&bar, s.primary);
    }
}

fn draw_wavy_circular(frame: &mut Frame<Renderer>, theme: &Theme, progress: f32, phase: f32) {
    use std::f32::consts::{FRAC_PI_2, TAU};
    let s = &theme.scheme;
    let center = frame.center();
    let progress = progress.clamp(0.0, 1.0);
    if progress > 0.0 {
        let points = wave::wavy_ring(center, CIRCULAR_RING, progress, phase);
        frame.stroke(&polyline(&points), stroke(s.primary));
    }
    // The track: the rest of the turn, `GAP + STROKE` of arc clear of each end.
    let r = CIRCULAR_RING / 2.0;
    let clear = if progress > 0.0 {
        (GAP + STROKE) / r
    } else {
        0.0
    };
    let start = TAU * progress + clear;
    let end = TAU - clear;
    if end > start {
        let arc = Path::new(|b| {
            b.arc(canvas::path::Arc {
                center,
                radius: r,
                start_angle: iced_core::Radians(start - FRAC_PI_2),
                end_angle: iced_core::Radians(end - FRAC_PI_2),
            });
        });
        frame.stroke(&arc, stroke(s.secondary_container));
    }
}

fn draw_wave(frame: &mut Frame<Renderer>, theme: &Theme, tone: WaveTone) {
    let s = &theme.scheme;
    let size = frame.size().width.min(frame.size().height);
    let origin = Point::new(
        (frame.size().width - size) / 2.0,
        (frame.size().height - size) / 2.0,
    );
    let points = wave::outline(origin, size);
    let path = Path::new(|b| {
        if let Some((first, rest)) = points.split_first() {
            b.move_to(*first);
            for p in rest {
                b.line_to(*p);
            }
            b.close();
        }
    });
    let color = match tone {
        WaveTone::Primary => s.primary,
        WaveTone::Tertiary => s.tertiary_container,
    };
    frame.fill(&path, color);
}

impl<M> canvas::Program<M, Theme, Renderer> for Program {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry<Renderer>> {
        let mut frame = Frame::new(renderer, bounds.size());
        match self.0 {
            Shape::WavyLinear { progress, phase } => {
                draw_wavy_linear(&mut frame, theme, progress, phase);
            }
            Shape::FlatLinear { progress } => draw_flat_linear(&mut frame, theme, progress),
            Shape::WavyCircular { progress, phase } => {
                draw_wavy_circular(&mut frame, theme, progress, phase);
            }
            Shape::Wave(tone) => draw_wave(&mut frame, theme, tone),
        }
        vec![frame.into_geometry()]
    }
}

fn canvas<'a, M: 'a>(shape: Shape, width: Length, height: f32) -> Element<'a, M, Theme, Renderer> {
    iced_widget::canvas(Program(shape))
        .width(width)
        .height(height)
        .into()
}

/// Wavy linear progress, full width, 14 high.
pub fn wavy_progress<'a, M: 'a>(progress: f32, phase: f32) -> Element<'a, M, Theme, Renderer> {
    canvas(
        Shape::WavyLinear { progress, phase },
        Length::Fill,
        LINEAR_HEIGHT,
    )
}

/// Flat linear progress, full width, 8 high.
pub fn flat_progress<'a, M: 'a>(progress: f32) -> Element<'a, M, Theme, Renderer> {
    canvas(Shape::FlatLinear { progress }, Length::Fill, FLAT_HEIGHT)
}

/// Wavy circular progress in a 48 px box.
pub fn circular_progress<'a, M: 'a>(progress: f32, phase: f32) -> Element<'a, M, Theme, Renderer> {
    canvas(
        Shape::WavyCircular { progress, phase },
        Length::Fixed(CIRCULAR_BOX),
        CIRCULAR_BOX,
    )
}

/// The wave shape, `size` px square. Stack content (initials, an icon) on top.
pub fn wave_shape<'a, M: 'a>(size: f32, tone: WaveTone) -> Element<'a, M, Theme, Renderer> {
    canvas(Shape::Wave(tone), Length::Fixed(size), size)
}
