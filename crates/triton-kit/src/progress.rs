//! Progress and level indicators: [`wavy_progress`] (linear determinate +
//! indeterminate waiting), [`wavy_ring`] (circular countdown drain), and
//! [`level_meter`] (3-bar audio equalizer).
//!
//! All strokes are round-capped SDF capsules via [`triton_engine::polyline`]
//! over the `triton_design::wave` geometry; every animated quantity derives
//! from the [`Animator`] clock or an `Animator` spring, so `reduce_motion`
//! degrades to the standard 120ms fades and static indicators globally.

use std::f32::consts::{FRAC_PI_2, TAU};
use std::sync::OnceLock;
use std::time::Instant;

use triton_design::motion::Spring;
use triton_design::shape::Point;
use triton_design::wave::{self, CIRCULAR_AMPLITUDE_AT_34, LINEAR_AMPLITUDE};
use triton_engine::anim::Animator;
use triton_engine::style::{Rgba, Theme};
use triton_engine::ui::{Element, NodeKey, box_, polyline, row};

/// Seconds since the first clock-driven indicator painted. Anchored once per
/// process so `anim.now()` maps to deterministic seconds — animation is a
/// pure function of the animator clock, never a free-running wall clock.
fn clock_secs(now: Instant) -> f32 {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    now.saturating_duration_since(*EPOCH.get_or_init(Instant::now))
        .as_secs_f32()
}

/// Wave phase drift while work is in flight (rad/s).
const PHASE_SPEED: f32 = 4.0;
/// Indeterminate window: 35% of the track, crossing it every 1.6s.
const INDETERMINATE_WINDOW: f32 = 0.35;
const INDETERMINATE_PERIOD: f32 = 1.6;

/// Animation channel salts (XOR-bijective off the caller's `NodeKey`).
const PROGRESS_SALT: u64 = 0x7072_6f67_5f70_7267; // "prog_prg"
const LEVEL_BAR_SALT: u64 = 0x6c76_6c5f_6261_7200; // "lvl_bar"

/// Fill mode for [`wavy_progress`].
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum Progress {
    /// Determinate fill `0.0..=1.0`; `>= 1.0` renders a flat complete line.
    Determinate(f32),
    /// Waiting: a wavy window travels the track while work is pending
    /// (static gentle centered wave under `reduce_motion`).
    Indeterminate,
}

/// Wavy linear progress (`triton-design` `wave::wavy_line`): a flat
/// `surface_container_highest` track stroked at `stroke` px, with the played
/// part as a sine wave (3px amplitude, 24px wavelength) in `primary`.
///
/// Determinate progress is spring-smoothed through `anim` (`key`-derived
/// channel) and the wave phase drifts while `0 < p < 1`; `p >= 1.0` flattens
/// to a solid `primary` line. [`Progress::Indeterminate`] travels a 35%
/// window along the track — or holds a static centered wave under
/// `reduce_motion`.
pub fn wavy_progress<M>(
    key: impl Into<NodeKey>,
    width: f32,
    stroke: f32,
    progress: Progress,
    theme: &Theme,
    anim: &mut Animator,
) -> Element<M> {
    let key = key.into();
    let height = LINEAR_AMPLITUDE.mul_add(2.0, stroke + 2.0);
    let cy = height * 0.5;
    let track = Rgba::from_color(theme.scheme.surface_container_highest);
    let active = Rgba::from_color(theme.scheme.primary);
    let phase = if anim.reduce_motion() {
        0.0
    } else {
        clock_secs(anim.now()) * PHASE_SPEED
    };

    let track_el = polyline(
        vec![Point::new(0.0, cy), Point::new(width, cy)],
        stroke,
        track,
    )
    .size(width, height)
    .absolute()
    .inset_xy(0.0, 0.0);

    let (points, x) = match progress {
        Progress::Determinate(target) => {
            let p = anim.spring(
                NodeKey(key.0 ^ PROGRESS_SALT),
                target.clamp(0.0, 1.0),
                Spring::EFFECTS,
            );
            if p <= f32::EPSILON {
                (Vec::new(), 0.0)
            } else if p >= 1.0 - f32::EPSILON {
                // Complete: the wave flattens into a solid line.
                (vec![Point::new(0.0, cy), Point::new(width, cy)], 0.0)
            } else {
                (wave::wavy_line(Point::new(0.0, cy), width, p, phase), 0.0)
            }
        }
        Progress::Indeterminate => {
            let window = width * INDETERMINATE_WINDOW;
            let x = if anim.reduce_motion() {
                (width - window) * 0.5
            } else {
                let frac = (clock_secs(anim.now()) / INDETERMINATE_PERIOD).fract();
                (width + window).mul_add(frac, -window)
            };
            (wave::wavy_line(Point::new(0.0, cy), window, 1.0, phase), x)
        }
    };

    let mut root = box_().w(width).h(height).clip().child(track_el);
    if !points.is_empty() {
        root = root.child(
            polyline(points, stroke, active)
                .size(width, height)
                .absolute()
                .inset_xy(x, 0.0),
        );
    }
    root
}

/// Points of a flat circle (the [`wavy_ring`] track): `segs` segments
/// clockwise from 12 o'clock.
fn ring_track_points(center: Point, r: f32) -> Vec<Point> {
    let segs = (TAU * r).ceil().max(24.0) as usize;
    (0..=segs)
        .map(|i| {
            let a = TAU * i as f32 / segs as f32 - FRAC_PI_2;
            Point::new(center.x + r * a.cos(), center.y + r * a.sin())
        })
        .collect()
}

/// Wavy circular countdown drain (`triton-design` `wave::wavy_ring`):
/// `progress` of a turn remains, sweeping clockwise from 12 o'clock — the
/// Power/Record/Clipboard sensitive-TTL indicator. Lobes ripple while the
/// ring is non-trivial and freeze under `reduce_motion`.
///
/// `diameter` is the element box; the wave radius is shrunk so lobes and the
/// `stroke` cap stay inside it. A subtle `surface_container_highest` track
/// ring sits underneath.
pub fn wavy_ring<M>(
    diameter: f32,
    stroke: f32,
    progress: f32,
    theme: &Theme,
    anim: &mut Animator,
) -> Element<M> {
    let center = Point::new(diameter * 0.5, diameter * 0.5);
    // wave_r + amp + stroke/2 <= diameter/2, amp = 1.1px at a 34px diameter.
    let wave_r =
        (diameter - stroke).max(0.0) * 0.5 / CIRCULAR_AMPLITUDE_AT_34.mul_add(2.0 / 17.0, 1.0);
    let phase = if anim.reduce_motion() {
        0.0
    } else {
        clock_secs(anim.now()) * PHASE_SPEED
    };

    let track_el = polyline(
        ring_track_points(center, wave_r),
        stroke,
        Rgba::from_color(theme.scheme.surface_container_highest),
    )
    .size(diameter, diameter)
    .absolute()
    .inset_xy(0.0, 0.0);

    let mut root = box_().size(diameter, diameter).clip().child(track_el);
    if progress > f32::EPSILON {
        root = root.child(
            polyline(
                wave::wavy_ring(center, wave_r * 2.0, progress, phase),
                stroke,
                Rgba::from_color(theme.scheme.primary),
            )
            .size(diameter, diameter)
            .absolute()
            .inset_xy(0.0, 0.0),
        );
    }
    root
}

/// Drive mode for [`level_meter`].
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum MeterMode {
    /// Spring toward the given `0.0..=1.0` levels per bar (e.g. real capture
    /// or playback volume fed in per frame).
    Levels([f32; 3]),
    /// Free-running playback animation from the animator clock (media is
    /// playing but no per-sample levels are wired); holds three static
    /// gentle bars under `reduce_motion`.
    Playing,
}

/// 3-bar animated audio equalizer: rounded vertical `primary` bars anchored
/// to the bottom edge, heights spring-smoothed through `anim` per bar
/// (`key`-derived channels) — for the media pill's live-level indicator.
pub fn level_meter<M>(
    key: impl Into<NodeKey>,
    mode: MeterMode,
    size: (f32, f32),
    theme: &Theme,
    anim: &mut Animator,
) -> Element<M> {
    let key = key.into();
    let (w, h) = size;
    let gap = 3.0;
    let bar_w = ((w - 2.0 * gap) / 3.0).max(2.0);
    let targets = match mode {
        MeterMode::Levels(levels) => levels.map(|v| v.clamp(0.0, 1.0)),
        MeterMode::Playing if anim.reduce_motion() => [0.4, 0.65, 0.5],
        MeterMode::Playing => {
            let t = clock_secs(anim.now());
            [0.3, 1.0, 2.0]
                .map(|i| 0.3 + 0.55 * (0.5 + 0.5 * (t * (2.4 + i * 1.1) + i * 2.1).sin()))
        }
    };

    let fill = Rgba::from_color(theme.scheme.primary);
    row()
        .w(w)
        .h(h)
        .gap(gap)
        .items_end()
        .children(targets.iter().enumerate().map(|(i, &target)| {
            let target_h = (h * target).clamp(h * 0.15, h);
            let bh = anim.spring_px(
                NodeKey(key.0 ^ LEVEL_BAR_SALT ^ i as u64),
                target_h,
                Spring::EFFECTS,
            );
            box_()
                .w(bar_w)
                .h(bh)
                .rounded(bar_w * 0.5)
                .smoothing(0.0)
                .bg(fill)
        }))
}
