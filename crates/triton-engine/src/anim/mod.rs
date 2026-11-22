//! First-class per-surface animation registry keyed by [`NodeKey`].
//!
//! [`Animator`] owns spring-driven scalars, colors, corner radii, layout rects,
//! presence mount/unmount lifecycles, and damped one-shot shakes. Entries not
//! touched for more than 2 frames are automatically pruned so unmounted
//! elements never leak memory.
//!
//! Time is always passed explicitly via [`Animator::begin_frame`], keeping
//! every animation deterministic and testable without sleeping.

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::f32::consts::TAU;
use std::time::{Duration, Instant};

use triton_design::motion::{Animated, Motion, Spring};

use crate::style::{Corners, Rect, Rgba};
use crate::ui::NodeKey;

const DEFAULT_SCALAR_EPSILON: f32 = 0.01;
const PIXEL_EPSILON: f32 = 0.5;
const COLOR_EPSILON: f32 = 0.002;
const CORNERS_EPSILON: f32 = 0.25;
const PRESENCE_EPSILON: f32 = 0.005;
const SHAKE_DURATION: Duration = Duration::from_millis(300);
const SHAKE_CYCLES: f32 = 3.0;
const SHAKE_DECAY: f32 = 12.0;
const PRUNE_AFTER_FRAMES: u64 = 2;

/// Mount/unmount animation state returned by [`Animator::presence`].
#[must_use]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Presence {
    /// Whether the subtree should remain mounted in the element tree.
    pub mounted: bool,
    /// Normalized enter/exit progress in `0.0..=1.0`.
    pub progress: f32,
}

#[derive(Debug, Clone, Copy)]
struct Entry<T> {
    value: T,
    last_seen: u64,
}

impl<T> Entry<T> {
    fn new(value: T, generation: u64) -> Self {
        Self {
            value,
            last_seen: generation,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct ColorState {
    channels: [Animated; 4],
    motion: Motion,
}

#[derive(Debug, Clone, Copy)]
struct CornersState {
    channels: [Animated; 4],
    motion: Motion,
}

#[derive(Debug, Clone, Copy)]
struct RectState {
    channels: [Animated; 4],
    motion: Motion,
}

#[derive(Debug, Clone, Copy)]
struct ScalarState {
    anim: Animated,
    motion: Motion,
}

#[derive(Debug, Clone, Copy)]
struct PresenceState {
    anim: Animated,
    visible: bool,
    motion: Motion,
}

#[derive(Debug, Clone, Copy)]
struct ShakeState {
    start: Instant,
    amplitude_px: f32,
}

impl ShakeState {
    fn offset(&self, now: Instant) -> f32 {
        let elapsed = now.saturating_duration_since(self.start);
        if elapsed >= SHAKE_DURATION {
            return 0.0;
        }
        let t = elapsed.as_secs_f32();
        let dur = SHAKE_DURATION.as_secs_f32();
        let decay = (-SHAKE_DECAY * t).exp();
        let wave = (TAU * SHAKE_CYCLES * (t / dur)).sin();
        self.amplitude_px * decay * wave
    }

    fn is_active(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.start) < SHAKE_DURATION && self.amplitude_px.abs() > 0.01
    }
}

/// Engine-owned per-surface animation registry.
#[derive(Debug)]
pub struct Animator {
    now: Instant,
    epoch: Option<Instant>,
    reduce_motion: bool,
    continuous_active: bool,
    generation: u64,
    scalars: HashMap<NodeKey, Entry<ScalarState>>,
    colors: HashMap<NodeKey, Entry<ColorState>>,
    corners: HashMap<NodeKey, Entry<CornersState>>,
    rects: HashMap<NodeKey, Entry<RectState>>,
    presences: HashMap<NodeKey, Entry<PresenceState>>,
    shakes: HashMap<NodeKey, Entry<ShakeState>>,
}

impl Default for Animator {
    fn default() -> Self {
        Self::new()
    }
}

impl Animator {
    /// Create an empty animation registry.
    pub fn new() -> Self {
        Self {
            now: Instant::now(),
            epoch: None,
            reduce_motion: false,
            continuous_active: false,
            generation: 0,
            scalars: HashMap::new(),
            colors: HashMap::new(),
            corners: HashMap::new(),
            rects: HashMap::new(),
            presences: HashMap::new(),
            shakes: HashMap::new(),
        }
    }

    /// Start a frame at `now` with the active `reduce_motion` preference.
    pub fn begin_frame(&mut self, now: Instant, reduce_motion: bool) {
        self.now = now;
        self.epoch.get_or_insert(now);
        self.reduce_motion = reduce_motion;
        self.continuous_active = false;
        self.generation = self.generation.wrapping_add(1);
    }

    /// Prune keys that have not been touched for more than 2 frames.
    pub fn end_frame(&mut self) {
        let generation = self.generation;
        let is_fresh = |last_seen: u64| generation.wrapping_sub(last_seen) <= PRUNE_AFTER_FRAMES;
        self.scalars.retain(|_, e| is_fresh(e.last_seen));
        self.colors.retain(|_, e| is_fresh(e.last_seen));
        self.corners.retain(|_, e| is_fresh(e.last_seen));
        self.rects.retain(|_, e| is_fresh(e.last_seen));
        self.presences.retain(|_, e| is_fresh(e.last_seen));
        self.shakes.retain(|_, e| is_fresh(e.last_seen));
    }

    /// Current frame timestamp set by [`Self::begin_frame`].
    pub fn now(&self) -> Instant {
        self.now
    }

    /// Current reduced-motion flag set by [`Self::begin_frame`].
    pub fn reduce_motion(&self) -> bool {
        self.reduce_motion
    }

    /// Seconds elapsed since this animator's first [`Self::begin_frame`] call.
    ///
    /// Anchored to the caller-provided timestamp (never wall-clock `Instant::now`),
    /// so clock-driven indicators advance deterministically in headless tests.
    pub fn elapsed_secs(&self) -> f32 {
        self.epoch
            .map_or(0.0, |e| self.now.saturating_duration_since(e).as_secs_f32())
    }

    /// Register continuous clock-driven motion for this frame (e.g. indeterminate
    /// wavy progress or a live audio level meter).
    ///
    /// Causes [`Self::is_animating`] to return `true` for this frame unless
    /// `reduce_motion` is enabled; automatically resets on the next [`Self::begin_frame`].
    pub fn request_continuous_frame(&mut self) {
        if !self.reduce_motion {
            self.continuous_active = true;
        }
    }

    /// Animate a scalar value with default epsilon (`0.01`).
    ///
    /// First-seen keys start at rest at `target` (no jump from zero).
    pub fn spring(&mut self, key: impl Into<NodeKey>, target: f32, spring: Spring) -> f32 {
        self.spring_with_epsilon(key, target, spring, DEFAULT_SCALAR_EPSILON)
    }

    /// Animate a pixel-valued scalar with pixel settle epsilon (`0.5`).
    pub fn spring_px(&mut self, key: impl Into<NodeKey>, target: f32, spring: Spring) -> f32 {
        self.spring_with_epsilon(key, target, spring, PIXEL_EPSILON)
    }

    /// Animate a scalar value with an explicit settle `epsilon`.
    pub fn spring_with_epsilon(
        &mut self,
        key: impl Into<NodeKey>,
        target: f32,
        spring: Spring,
        epsilon: f32,
    ) -> f32 {
        let key = key.into();
        let motion = Motion::pick(spring, self.reduce_motion);
        let now = self.now;
        let generation = self.generation;

        let entry = self.scalars.entry(key).or_insert_with(|| {
            Entry::new(
                ScalarState {
                    anim: Animated::new(target, motion, epsilon, now),
                    motion,
                },
                generation,
            )
        });
        entry.last_seen = generation;
        if entry.value.motion != motion {
            entry.value.anim.set_motion(motion, now);
            entry.value.motion = motion;
        }
        entry.value.anim.set_target(target, now);
        entry.value.anim.value(now)
    }

    /// Seed or retarget a scalar spring with an explicit initial velocity (e.g.
    /// from a pointer drag/fling release).
    ///
    /// The velocity is injected by stepping a spring from `initial_value` toward
    /// `target` with `velocity` at the current frame timestamp.
    pub fn spring_with_velocity(
        &mut self,
        key: impl Into<NodeKey>,
        initial_value: f32,
        target: f32,
        velocity: f32,
        spring: Spring,
        epsilon: f32,
    ) -> f32 {
        let key = key.into();
        let motion = Motion::pick(spring, self.reduce_motion);
        let now = self.now;
        let generation = self.generation;

        // `Animated` keeps velocity private, so we seed it via a 2ms linear ramp
        // centered at `now` (`t = 1ms < 2ms`) so `sample(now)` yields the exact
        // `(initial_value, velocity)` pair before switching to the spring.
        let mut anim = if velocity.abs() > f32::EPSILON && !self.reduce_motion {
            let half_dt = Duration::from_millis(1);
            let full_dt = Duration::from_millis(2);
            if let Some(prev) = now.checked_sub(half_dt) {
                let delta = velocity * half_dt.as_secs_f32();
                let mut seeded =
                    Animated::new(initial_value - delta, Motion::Fade(full_dt), epsilon, prev);
                seeded.set_target(initial_value + delta, prev);
                seeded.set_motion(motion, now);
                seeded.set_target(target, now);
                seeded
            } else {
                let mut a = Animated::new(initial_value, motion, epsilon, now);
                a.set_target(target, now);
                a
            }
        } else {
            let mut a = Animated::new(initial_value, motion, epsilon, now);
            a.set_target(target, now);
            a
        };

        let value = anim.value(now);
        // Ensure target is recorded even when `initial_value == target`.
        anim.set_target(target, now);
        self.scalars
            .insert(key, Entry::new(ScalarState { anim, motion }, generation));
        value
    }

    /// Animate an [`Rgba`] color across all 4 channels (epsilon `0.002`).
    pub fn color(&mut self, key: impl Into<NodeKey>, target: Rgba, spring: Spring) -> Rgba {
        let key = key.into();
        let motion = Motion::pick(spring, self.reduce_motion);
        let now = self.now;
        let generation = self.generation;
        let arr = target.to_array();

        let entry = self.colors.entry(key).or_insert_with(|| {
            let channels = arr.map(|c| Animated::new(c, motion, COLOR_EPSILON, now));
            Entry::new(ColorState { channels, motion }, generation)
        });
        entry.last_seen = generation;
        if entry.value.motion != motion {
            for ch in &mut entry.value.channels {
                ch.set_motion(motion, now);
            }
            entry.value.motion = motion;
        }
        let mut out = [0.0; 4];
        for (i, ch) in entry.value.channels.iter_mut().enumerate() {
            ch.set_target(arr[i], now);
            out[i] = ch.value(now).clamp(0.0, 1.0);
        }
        Rgba::from(out)
    }

    /// Animate per-corner radii [`Corners`] across all 4 corners (epsilon `0.25`).
    pub fn corners(&mut self, key: impl Into<NodeKey>, target: Corners, spring: Spring) -> Corners {
        let key = key.into();
        let motion = Motion::pick(spring, self.reduce_motion);
        let now = self.now;
        let generation = self.generation;

        let entry = self.corners.entry(key).or_insert_with(|| {
            let channels = target
                .0
                .map(|r| Animated::new(r, motion, CORNERS_EPSILON, now));
            Entry::new(CornersState { channels, motion }, generation)
        });
        entry.last_seen = generation;
        if entry.value.motion != motion {
            for ch in &mut entry.value.channels {
                ch.set_motion(motion, now);
            }
            entry.value.motion = motion;
        }
        let mut out = [0.0; 4];
        for (i, ch) in entry.value.channels.iter_mut().enumerate() {
            ch.set_target(target.0[i], now);
            out[i] = ch.value(now).max(0.0);
        }
        Corners(out)
    }

    /// Animate a layout [`Rect`] (`x`, `y`, `w`, `h`) with pixel epsilon (`0.5`).
    pub fn layout_rect(&mut self, key: impl Into<NodeKey>, target: Rect, spring: Spring) -> Rect {
        let key = key.into();
        let motion = Motion::pick(spring, self.reduce_motion);
        let now = self.now;
        let generation = self.generation;
        let arr = [target.x, target.y, target.w, target.h];

        let entry = self.rects.entry(key).or_insert_with(|| {
            let channels = arr.map(|v| Animated::new(v, motion, PIXEL_EPSILON, now));
            Entry::new(RectState { channels, motion }, generation)
        });
        entry.last_seen = generation;
        if entry.value.motion != motion {
            for ch in &mut entry.value.channels {
                ch.set_motion(motion, now);
            }
            entry.value.motion = motion;
        }
        let mut out = [0.0; 4];
        for (i, ch) in entry.value.channels.iter_mut().enumerate() {
            ch.set_target(arr[i], now);
            out[i] = ch.value(now);
        }
        Rect::new(out[0], out[1], out[2].max(0.0), out[3].max(0.0))
    }

    /// Trigger a damped ~300 ms horizontal shake animation on `key`.
    ///
    /// Has no effect when `reduce_motion` is enabled.
    pub fn trigger_shake(&mut self, key: impl Into<NodeKey>, amplitude_px: f32) {
        if self.reduce_motion {
            return;
        }
        let key = key.into();
        self.shakes.insert(
            key,
            Entry::new(
                ShakeState {
                    start: self.now,
                    amplitude_px,
                },
                self.generation,
            ),
        );
    }

    /// Sample the current horizontal shake offset in logical pixels for `key`.
    ///
    /// Returns `0.0` when idle, settled, or when `reduce_motion` is enabled.
    pub fn shake_offset(&mut self, key: impl Into<NodeKey>) -> f32 {
        if self.reduce_motion {
            return 0.0;
        }
        let key = key.into();
        let Some(entry) = self.shakes.get_mut(&key) else {
            return 0.0;
        };
        entry.last_seen = self.generation;
        entry.value.offset(self.now)
    }

    /// Track mount/unmount presence for an element with distinct `enter` and `exit` springs.
    ///
    /// - If `visible == true` on first sight, animates from `0.0 -> 1.0` so entering
    ///   elements animate in smoothly.
    /// - If `visible == false` on first sight, starts unmounted at `0.0`.
    /// - During exit, `mounted` remains `true` until `progress <= 0.005` and the
    ///   spring has settled.
    pub fn presence(
        &mut self,
        key: impl Into<NodeKey>,
        visible: bool,
        enter: Spring,
        exit: Spring,
    ) -> Presence {
        let key = key.into();
        let chosen_spring = if visible { enter } else { exit };
        let motion = Motion::pick(chosen_spring, self.reduce_motion);
        let now = self.now;
        let generation = self.generation;
        let target = if visible { 1.0 } else { 0.0 };

        let entry = self.presences.entry(key).or_insert_with(|| {
            let mut anim = Animated::new(0.0, motion, PRESENCE_EPSILON, now);
            if visible {
                anim.set_target(1.0, now);
            }
            Entry::new(
                PresenceState {
                    anim,
                    visible,
                    motion,
                },
                generation,
            )
        });
        entry.last_seen = generation;

        if entry.value.motion != motion {
            entry.value.anim.set_motion(motion, now);
            entry.value.motion = motion;
        }
        if entry.value.visible != visible {
            entry.value.visible = visible;
            entry.value.anim.set_target(target, now);
        }

        let raw = entry.value.anim.value(now);
        let progress = raw.clamp(0.0, 1.0);
        let mounted = visible || progress > PRESENCE_EPSILON;
        Presence { mounted, progress }
    }

    /// Return `true` if any active spring, color, corners, layout rect, shake,
    /// or presence animation is still unsettled at `self.now`.
    pub fn is_animating(&self) -> bool {
        let now = self.now;
        self.scalars.values().any(|e| !e.value.anim.is_settled(now))
            || self
                .colors
                .values()
                .any(|e| e.value.channels.iter().any(|c| !c.is_settled(now)))
            || self
                .corners
                .values()
                .any(|e| e.value.channels.iter().any(|c| !c.is_settled(now)))
            || self
                .rects
                .values()
                .any(|e| e.value.channels.iter().any(|c| !c.is_settled(now)))
            || self
                .presences
                .values()
                .any(|e| !e.value.anim.is_settled(now))
            || (!self.reduce_motion && self.continuous_active)
            || (!self.reduce_motion && self.shakes.values().any(|e| e.value.is_active(now)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(anim: &mut Animator, t: Instant, reduce_motion: bool) {
        anim.begin_frame(t, reduce_motion);
    }

    #[test]
    fn spring_starts_at_target_and_retargets_smoothly() {
        let t0 = Instant::now();
        let mut anim = Animator::new();

        // First sight: starts at rest at target (no jump from 0).
        step(&mut anim, t0, false);
        let v0 = anim.spring("pill", 100.0, Spring::SPATIAL);
        anim.end_frame();
        assert!((v0 - 100.0).abs() < 1e-4);
        assert!(!anim.is_animating());

        // Retarget to 200.0.
        let t1 = t0 + Duration::from_millis(16);
        step(&mut anim, t1, false);
        let v1 = anim.spring("pill", 200.0, Spring::SPATIAL);
        anim.end_frame();
        assert!((v1 - 100.0).abs() < 1e-3);
        assert!(anim.is_animating());

        // Mid-flight sample moves toward 200.0.
        let t2 = t1 + Duration::from_millis(80);
        step(&mut anim, t2, false);
        let v2 = anim.spring("pill", 200.0, Spring::SPATIAL);
        anim.end_frame();
        assert!(
            v2 > 110.0 && v2 < 220.0,
            "expected mid-flight value, got {v2}"
        );
        assert!(anim.is_animating());

        // Retarget mid-flight back to 50.0: should not teleport.
        let t3 = t2 + Duration::from_millis(16);
        step(&mut anim, t3, false);
        let v3 = anim.spring("pill", 50.0, Spring::SPATIAL);
        anim.end_frame();
        assert!(
            (v3 - v2).abs() < 40.0,
            "retarget should preserve continuity: v2={v2}, v3={v3}"
        );
        assert!(anim.is_animating());

        // Eventually settles at 50.0 and stops animating.
        let t_done = t3 + Duration::from_secs(3);
        step(&mut anim, t_done, false);
        let v_done = anim.spring("pill", 50.0, Spring::SPATIAL);
        anim.end_frame();
        assert!(
            (v_done - 50.0).abs() < 0.05,
            "expected settled ~50, got {v_done}"
        );
        assert!(!anim.is_animating());
    }

    #[test]
    fn spring_with_velocity_launches_immediately() {
        let t0 = Instant::now();
        let mut anim = Animator::new();

        step(&mut anim, t0, false);
        let v0 = anim.spring_with_velocity("drag", 10.0, 10.0, 500.0, Spring::SPATIAL, 0.5);
        anim.end_frame();
        assert!((v0 - 10.0).abs() < 0.5);
        assert!(anim.is_animating());

        // Positive velocity pushes the spring past 10.0 before settling back.
        let t1 = t0 + Duration::from_millis(30);
        step(&mut anim, t1, false);
        let v1 = anim.spring_px("drag", 10.0, Spring::SPATIAL);
        anim.end_frame();
        assert!(v1 > 12.0, "expected velocity impulse > 12, got {v1}");
    }

    #[test]
    fn color_and_corners_interpolate_and_settle() {
        let t0 = Instant::now();
        let mut anim = Animator::new();
        let c_start = Rgba::new(0.1, 0.2, 0.3, 1.0);
        let c_end = Rgba::new(0.9, 0.8, 0.7, 0.5);
        let r_start = Corners::all(8.0);
        let r_end = Corners::each(28.0, 28.0, 12.0, 12.0);

        step(&mut anim, t0, false);
        assert_eq!(anim.color("bg", c_start, Spring::EFFECTS), c_start);
        assert_eq!(anim.corners("rad", r_start, Spring::SPATIAL), r_start);
        anim.end_frame();
        assert!(!anim.is_animating());

        let t1 = t0 + Duration::from_millis(16);
        step(&mut anim, t1, false);
        let _ = anim.color("bg", c_end, Spring::EFFECTS);
        let _ = anim.corners("rad", r_end, Spring::SPATIAL);
        anim.end_frame();
        assert!(anim.is_animating());

        let t_mid = t1 + Duration::from_millis(60);
        step(&mut anim, t_mid, false);
        let c_mid = anim.color("bg", c_end, Spring::EFFECTS);
        let r_mid = anim.corners("rad", r_end, Spring::SPATIAL);
        anim.end_frame();
        assert!(c_mid.r > 0.2 && c_mid.r < 0.85, "c_mid.r={}", c_mid.r);
        assert!(
            r_mid.0[0] > 10.0 && r_mid.0[0] < 32.0,
            "r_mid={:?}",
            r_mid.0
        );

        let t_done = t_mid + Duration::from_secs(3);
        step(&mut anim, t_done, false);
        let c_done = anim.color("bg", c_end, Spring::EFFECTS);
        let r_done = anim.corners("rad", r_end, Spring::SPATIAL);
        anim.end_frame();
        assert!((c_done.r - c_end.r).abs() < 0.01);
        assert!((c_done.a - c_end.a).abs() < 0.01);
        assert!((r_done.0[0] - 28.0).abs() < 0.3);
        assert!((r_done.0[2] - 12.0).abs() < 0.3);
        assert!(!anim.is_animating());
    }

    #[test]
    fn layout_rect_interpolates_and_settles() {
        let t0 = Instant::now();
        let mut anim = Animator::new();
        let a = Rect::new(0.0, 0.0, 100.0, 40.0);
        let b = Rect::new(50.0, 20.0, 200.0, 80.0);

        step(&mut anim, t0, false);
        assert_eq!(anim.layout_rect("card", a, Spring::SPATIAL), a);
        anim.end_frame();
        assert!(!anim.is_animating());

        let t1 = t0 + Duration::from_millis(16);
        step(&mut anim, t1, false);
        let _ = anim.layout_rect("card", b, Spring::SPATIAL);
        anim.end_frame();
        assert!(anim.is_animating());

        let t_done = t1 + Duration::from_secs(3);
        step(&mut anim, t_done, false);
        let done = anim.layout_rect("card", b, Spring::SPATIAL);
        anim.end_frame();
        assert!((done.x - b.x).abs() < 0.5);
        assert!((done.w - b.w).abs() < 0.5);
        assert!(!anim.is_animating());
    }

    #[test]
    fn presence_animates_in_and_stays_mounted_during_exit() {
        let t0 = Instant::now();
        let mut anim = Animator::new();

        // First sight with visible=true starts at 0.0 and animates toward 1.0.
        step(&mut anim, t0, false);
        let p0 = anim.presence("toast", true, Spring::SPATIAL, Spring::EFFECTS_FAST);
        anim.end_frame();
        assert!(p0.mounted);
        assert!(
            p0.progress < 0.05,
            "enter should start near 0, got {}",
            p0.progress
        );
        assert!(anim.is_animating());

        // Mid-enter: mounted and progressing toward 1.0.
        let t_mid = t0 + Duration::from_millis(100);
        step(&mut anim, t_mid, false);
        let p_mid = anim.presence("toast", true, Spring::SPATIAL, Spring::EFFECTS_FAST);
        anim.end_frame();
        assert!(p_mid.mounted);
        assert!(p_mid.progress > 0.2);

        // Settled enter.
        let t_open = t0 + Duration::from_secs(3);
        step(&mut anim, t_open, false);
        let p_open = anim.presence("toast", true, Spring::SPATIAL, Spring::EFFECTS_FAST);
        anim.end_frame();
        assert!(p_open.mounted);
        assert!((p_open.progress - 1.0).abs() < 0.01);
        assert!(!anim.is_animating());

        // Begin exit: stays mounted while animating back to 0.0.
        let t_exit = t_open + Duration::from_millis(16);
        step(&mut anim, t_exit, false);
        let p_exit0 = anim.presence("toast", false, Spring::SPATIAL, Spring::EFFECTS_FAST);
        anim.end_frame();
        assert!(p_exit0.mounted);
        assert!(p_exit0.progress > 0.9);
        assert!(anim.is_animating());

        // Mid-exit: still mounted with intermediate progress.
        let t_exit_mid = t_exit + Duration::from_millis(60);
        step(&mut anim, t_exit_mid, false);
        let p_exit_mid = anim.presence("toast", false, Spring::SPATIAL, Spring::EFFECTS_FAST);
        anim.end_frame();
        assert!(p_exit_mid.mounted);
        assert!(p_exit_mid.progress > 0.01 && p_exit_mid.progress < 0.95);

        // Settled exit: unmounts cleanly.
        let t_gone = t_exit + Duration::from_secs(3);
        step(&mut anim, t_gone, false);
        let p_gone = anim.presence("toast", false, Spring::SPATIAL, Spring::EFFECTS_FAST);
        anim.end_frame();
        assert!(!p_gone.mounted);
        assert!(p_gone.progress <= PRESENCE_EPSILON);
        assert!(!anim.is_animating());
    }

    #[test]
    fn shake_oscillates_decays_and_respects_reduce_motion() {
        let t0 = Instant::now();
        let mut anim = Animator::new();

        step(&mut anim, t0, false);
        anim.trigger_shake("pw", 12.0);
        assert!(anim.is_animating());
        anim.end_frame();

        // Quarter-cycle (~25ms) should produce positive offset; ~75ms negative.
        let t_pos = t0 + Duration::from_millis(25);
        step(&mut anim, t_pos, false);
        let off_pos = anim.shake_offset("pw");
        anim.end_frame();
        assert!(
            off_pos > 1.0,
            "expected positive shake offset, got {off_pos}"
        );
        assert!(anim.is_animating());

        let t_neg = t0 + Duration::from_millis(75);
        step(&mut anim, t_neg, false);
        let off_neg = anim.shake_offset("pw");
        anim.end_frame();
        assert!(
            off_neg < -0.5,
            "expected negative shake offset, got {off_neg}"
        );

        // After 300ms it decays to 0.0 and stops animating.
        let t_done = t0 + Duration::from_millis(310);
        step(&mut anim, t_done, false);
        let off_done = anim.shake_offset("pw");
        anim.end_frame();
        assert!((off_done - 0.0).abs() < 1e-6);
        assert!(!anim.is_animating());

        // Under reduce_motion, shake is disabled.
        let t_rm = t_done + Duration::from_millis(16);
        step(&mut anim, t_rm, true);
        anim.trigger_shake("pw2", 12.0);
        assert!((anim.shake_offset("pw2") - 0.0).abs() < 1e-6);
        assert!(!anim.is_animating());
        anim.end_frame();
    }

    #[test]
    fn reduce_motion_uses_120ms_fade() {
        let t0 = Instant::now();
        let mut anim = Animator::new();

        step(&mut anim, t0, true);
        let _ = anim.spring("fade", 0.0, Spring::SPATIAL_SLOW);
        anim.end_frame();

        let t1 = t0 + Duration::from_millis(10);
        step(&mut anim, t1, true);
        let _ = anim.spring("fade", 100.0, Spring::SPATIAL_SLOW);
        anim.end_frame();
        assert!(anim.is_animating());

        // Halfway through 120ms fade -> ~50.0.
        let t_half = t1 + Duration::from_millis(60);
        step(&mut anim, t_half, true);
        let v_half = anim.spring("fade", 100.0, Spring::SPATIAL_SLOW);
        anim.end_frame();
        assert!(
            (v_half - 50.0).abs() < 1.0,
            "expected ~50 at 60ms, got {v_half}"
        );
        assert!(anim.is_animating());

        // At 120ms fade is completely settled.
        let t_done = t1 + Duration::from_millis(120);
        step(&mut anim, t_done, true);
        let v_done = anim.spring("fade", 100.0, Spring::SPATIAL_SLOW);
        anim.end_frame();
        assert!((v_done - 100.0).abs() < 1e-4);
        assert!(!anim.is_animating());
    }

    #[test]
    fn pruning_removes_untouched_keys_after_two_frames() {
        let t0 = Instant::now();
        let mut anim = Animator::new();

        // Frame 1: create an actively animating key.
        step(&mut anim, t0, false);
        let _ = anim.spring("ephemeral", 0.0, Spring::SPATIAL);
        anim.end_frame();

        let t1 = t0 + Duration::from_millis(16);
        step(&mut anim, t1, false);
        let _ = anim.spring("ephemeral", 100.0, Spring::SPATIAL);
        anim.end_frame();
        assert!(anim.is_animating());

        // Frames 2 & 3 (1 and 2 frames untouched): still retained.
        for i in 1..=2 {
            step(&mut anim, t1 + Duration::from_millis(16 * i), false);
            anim.end_frame();
            assert!(
                anim.is_animating(),
                "key should still be retained after {i} untouched frame(s)"
            );
        }

        // Frame 4 (3 frames untouched, > 2): pruned cleanly.
        step(&mut anim, t1 + Duration::from_millis(16 * 3), false);
        anim.end_frame();
        assert!(
            !anim.is_animating(),
            "key should be pruned after >2 untouched frames"
        );
    }

    #[test]
    fn continuous_frame_and_elapsed_secs_are_deterministic() {
        let t0 = Instant::now();
        let mut anim = Animator::new();

        step(&mut anim, t0, false);
        assert!(anim.elapsed_secs().abs() < 1e-6);
        assert!(!anim.is_animating());

        anim.request_continuous_frame();
        assert!(anim.is_animating());

        // Next frame without request clears continuous_active; elapsed advances cleanly.
        let t1 = t0 + Duration::from_millis(250);
        step(&mut anim, t1, false);
        assert!((anim.elapsed_secs() - 0.25).abs() < 1e-4);
        assert!(!anim.is_animating());

        // Under reduce_motion, request_continuous_frame does not keep rendering.
        step(&mut anim, t1 + Duration::from_millis(16), true);
        anim.request_continuous_frame();
        assert!(!anim.is_animating());
    }
}
