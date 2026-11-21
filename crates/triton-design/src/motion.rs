//! Springs, not durations. Spatial springs may overshoot a little; effects springs
//! never do. Reduce motion swaps both for 120 ms fades.
//!
//! Time is always an argument: nothing here reads the clock, so every curve is
//! deterministic and testable.

use std::time::{Duration, Instant};

use crate::tokens;

/// A damped spring with unit mass.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spring {
    pub stiffness: f32,
    /// 1.0 is critically damped (no overshoot); below 1.0 overshoots.
    pub damping_ratio: f32,
}

impl Spring {
    /// Position, size and shape: bar islands dropping in, panels opening.
    pub const SPATIAL_FAST: Spring = tokens::SPRING_SPATIAL_FAST;
    pub const SPATIAL: Spring = tokens::SPRING_SPATIAL_DEFAULT;
    pub const SPATIAL_SLOW: Spring = tokens::SPRING_SPATIAL_SLOW;
    /// Color and opacity.
    pub const EFFECTS_FAST: Spring = tokens::SPRING_EFFECTS_FAST;
    pub const EFFECTS: Spring = tokens::SPRING_EFFECTS_DEFAULT;
    pub const EFFECTS_SLOW: Spring = tokens::SPRING_EFFECTS_SLOW;

    /// Displacement from the target and velocity after `t` seconds, starting from
    /// displacement `x0` and velocity `v0`. Closed form, so it is exact for any `t`.
    #[expect(
        clippy::many_single_char_names,
        reason = "the textbook damped-oscillator notation is clearer than long names"
    )]
    pub fn solve(self, x0: f32, v0: f32, t: f32) -> (f32, f32) {
        let w0 = self.stiffness.max(0.0).sqrt();
        let z = self.damping_ratio.max(0.0);
        if (z - 1.0).abs() < 1e-4 {
            // Critically damped.
            let b = v0 + w0 * x0;
            let e = (-w0 * t).exp();
            let x = e * (x0 + b * t);
            (x, e * b - w0 * x)
        } else if z < 1.0 {
            let wd = w0 * (1.0 - z * z).sqrt();
            let a = z * w0;
            let b = (v0 + a * x0) / wd;
            let (s, c) = (wd * t).sin_cos();
            let e = (-a * t).exp();
            let x = e * (x0 * c + b * s);
            (x, -a * x + e * wd * (b * c - x0 * s))
        } else {
            let root = (z * z - 1.0).sqrt();
            let (r1, r2) = (-w0 * (z - root), -w0 * (z + root));
            let c1 = (v0 - r2 * x0) / (r1 - r2);
            let c2 = x0 - c1;
            let (e1, e2) = ((r1 * t).exp(), (r2 * t).exp());
            (c1 * e1 + c2 * e2, c1 * r1 * e1 + c2 * r2 * e2)
        }
    }
}

/// How a value moves to its target.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Motion {
    Spring(Spring),
    /// Linear over a fixed time. What every spring becomes under reduce motion.
    Fade(Duration),
}

impl Motion {
    pub const REDUCED: Motion = Motion::Fade(Duration::from_millis(120));

    /// `spring`, or the 120 ms fade when the user asked for reduced motion.
    pub const fn pick(spring: Spring, reduce_motion: bool) -> Motion {
        if reduce_motion {
            Motion::REDUCED
        } else {
            Motion::Spring(spring)
        }
    }
}

/// A value that moves toward a target. Retargeting mid-flight keeps the velocity, so
/// interrupted animations never jump.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Animated {
    motion: Motion,
    from: f32,
    target: f32,
    velocity: f32,
    start: Instant,
    /// Below this distance and speed (value units, per second) it is settled.
    epsilon: f32,
}

impl Animated {
    /// A value at rest. `epsilon` is the settle threshold in value units: 0.5 for
    /// pixels, 0.001 for opacity.
    pub fn new(value: f32, motion: Motion, epsilon: f32, now: Instant) -> Animated {
        Animated {
            motion,
            from: value,
            target: value,
            velocity: 0.0,
            start: now,
            epsilon,
        }
    }

    pub fn target(&self) -> f32 {
        self.target
    }

    /// Move toward `target` from wherever the value is at `now`, keeping its speed.
    pub fn set_target(&mut self, target: f32, now: Instant) {
        if (target - self.target).abs() < f32::EPSILON {
            return;
        }
        let (value, velocity) = self.sample(now);
        self.from = value;
        self.velocity = velocity;
        self.target = target;
        self.start = now;
    }

    /// Jump to `value` with no animation.
    pub fn snap(&mut self, value: f32, now: Instant) {
        *self = Animated::new(value, self.motion, self.epsilon, now);
    }

    /// Change the motion (for example when reduce motion is toggled) without a jump.
    pub fn set_motion(&mut self, motion: Motion, now: Instant) {
        let (value, velocity) = self.sample(now);
        self.from = value;
        self.velocity = velocity;
        self.start = now;
        self.motion = motion;
    }

    pub fn value(&self, now: Instant) -> f32 {
        self.sample(now).0
    }

    pub fn velocity(&self, now: Instant) -> f32 {
        self.sample(now).1
    }

    /// Settled values need no more frames: the shell stops redrawing.
    pub fn is_settled(&self, now: Instant) -> bool {
        let (value, velocity) = self.sample(now);
        (value - self.target).abs() < self.epsilon && velocity.abs() < self.epsilon * 10.0
    }

    fn sample(&self, now: Instant) -> (f32, f32) {
        let t = now.saturating_duration_since(self.start).as_secs_f32();
        match self.motion {
            Motion::Spring(spring) => {
                let (x, v) = spring.solve(self.from - self.target, self.velocity, t);
                (self.target + x, v)
            }
            Motion::Fade(duration) => {
                let d = duration.as_secs_f32();
                if d <= 0.0 || t >= d {
                    (self.target, 0.0)
                } else {
                    let rate = (self.target - self.from) / d;
                    (self.from + rate * t, rate)
                }
            }
        }
    }
}
