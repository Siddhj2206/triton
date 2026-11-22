//! Pure-safe partial damage tracking for EGL buffer-age swaps.
//!
//! Tracks the dirty bounding rects of the last three presented frames so that
//! `EGL_BUFFER_AGE_KHR` can be used to union only the regions that changed since
//! the current backbuffer was last presented.

#![forbid(unsafe_code)]

/// Integer pixel damage rectangle in top-left surface coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DamageRect {
    /// Left edge in pixels.
    pub x: i32,
    /// Top edge in pixels.
    pub y: i32,
    /// Width in pixels.
    pub w: i32,
    /// Height in pixels.
    pub h: i32,
}

impl DamageRect {
    /// Returns `true` if the rectangle has zero or negative area.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.w <= 0 || self.h <= 0
    }

    /// Returns the smallest rectangle enclosing both `self` and `other`.
    ///
    /// If either rectangle is empty, returns the other.
    #[must_use]
    pub fn union(self, other: Self) -> Self {
        if self.is_empty() {
            return other;
        }
        if other.is_empty() {
            return self;
        }
        let x0 = self.x.min(other.x);
        let y0 = self.y.min(other.y);
        let x1 = self
            .x
            .saturating_add(self.w)
            .max(other.x.saturating_add(other.w));
        let y1 = self
            .y
            .saturating_add(self.h)
            .max(other.y.saturating_add(other.h));
        Self {
            x: x0,
            y: y0,
            w: x1.saturating_sub(x0),
            h: y1.saturating_sub(y0),
        }
    }

    /// Returns the intersection of `self` and `other` (empty when disjoint).
    #[must_use]
    pub fn intersect(self, other: Self) -> Self {
        let x0 = self.x.max(other.x);
        let y0 = self.y.max(other.y);
        let x1 = self
            .x
            .saturating_add(self.w)
            .min(other.x.saturating_add(other.w));
        let y1 = self
            .y
            .saturating_add(self.h)
            .min(other.y.saturating_add(other.h));
        Self {
            x: x0,
            y: y0,
            w: (x1 - x0).max(0),
            h: (y1 - y0).max(0),
        }
    }

    /// Clamp this rectangle to `(0, 0, surface_w, surface_h)`.
    #[must_use]
    pub fn clamp_to_surface(self, surface_w: i32, surface_h: i32) -> Self {
        let max_w = surface_w.max(0);
        let max_h = surface_h.max(0);
        let x0 = self.x.clamp(0, max_w);
        let y0 = self.y.clamp(0, max_h);
        let x1 = self.x.saturating_add(self.w).clamp(0, max_w);
        let y1 = self.y.saturating_add(self.h).clamp(0, max_h);
        Self {
            x: x0,
            y: y0,
            w: (x1 - x0).max(0),
            h: (y1 - y0).max(0),
        }
    }

    /// Convert from top-left surface coordinates to EGL bottom-left coordinates
    /// `[x, surface_h - (y + h), w, h]`.
    #[must_use]
    pub const fn to_egl_rect(self, surface_h: i32) -> [i32; 4] {
        [
            self.x,
            surface_h.saturating_sub(self.y.saturating_add(self.h)),
            self.w,
            self.h,
        ]
    }
}

/// Tracks a 3-frame ring buffer of dirty bounding rects for `EGL_BUFFER_AGE_KHR`.
#[derive(Debug, Default)]
pub struct DamageTracker {
    /// Most recent frame at index 0, previous at index 1, oldest at index 2.
    history: [Option<DamageRect>; 3],
    full_damage_pending: bool,
}

impl DamageTracker {
    /// Create a new damage tracker with no recorded history.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            history: [None; 3],
            full_damage_pending: false,
        }
    }

    /// Force a full-surface redraw on the next frame (e.g. on resize or first frame).
    pub fn mark_full(&mut self) {
        self.full_damage_pending = true;
    }

    /// Push this frame's dirty rect into the history ring and clear `full_damage_pending`.
    pub fn record_frame(&mut self, dirty: Option<DamageRect>) {
        self.history[2] = self.history[1];
        self.history[1] = self.history[0];
        self.history[0] = dirty;
        self.full_damage_pending = false;
    }

    /// Compute the effective damage rectangle to redraw and submit for this frame.
    ///
    /// - If `full_damage_pending` is set, or `buffer_age` is `None`, `Some(0)`, or `Some(> 3)`,
    ///   returns the full surface rect `(0, 0, surface_size.0, surface_size.1)`.
    /// - If `buffer_age == Some(1)`, returns `current_dirty` clamped to the surface.
    /// - If `buffer_age == Some(2)`, unions `current_dirty` with `history[0]`.
    /// - If `buffer_age == Some(3)`, unions `current_dirty` with `history[0]` and `history[1]`.
    #[must_use]
    pub fn compute_damage(
        &self,
        buffer_age: Option<u32>,
        current_dirty: Option<DamageRect>,
        surface_size: (i32, i32),
    ) -> Option<DamageRect> {
        let (w, h) = (surface_size.0.max(0), surface_size.1.max(0));
        let full = DamageRect { x: 0, y: 0, w, h };

        if self.full_damage_pending {
            return Some(full);
        }

        let Some(age @ 1..=3) = buffer_age else {
            return Some(full);
        };

        let mut combined = current_dirty;
        if age >= 2 {
            combined = union_opt(combined, self.history[0]);
        }
        if age >= 3 {
            combined = union_opt(combined, self.history[1]);
        }

        combined.map(|rect| rect.clamp_to_surface(w, h))
    }
}

fn union_opt(a: Option<DamageRect>, b: Option<DamageRect>) -> Option<DamageRect> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.union(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SURFACE: (i32, i32) = (1920, 1080);

    #[test]
    fn buffer_age_none_zero_or_gt_3_returns_full_surface() {
        let mut tracker = DamageTracker::new();
        let r1 = DamageRect {
            x: 10,
            y: 20,
            w: 30,
            h: 40,
        };
        tracker.record_frame(Some(r1));
        let full = Some(DamageRect {
            x: 0,
            y: 0,
            w: 1920,
            h: 1080,
        });

        assert_eq!(tracker.compute_damage(None, Some(r1), SURFACE), full);
        assert_eq!(tracker.compute_damage(Some(0), Some(r1), SURFACE), full);
        assert_eq!(tracker.compute_damage(Some(4), Some(r1), SURFACE), full);
        assert_eq!(tracker.compute_damage(Some(99), Some(r1), SURFACE), full);
    }

    #[test]
    fn buffer_age_1_returns_only_current_dirty() {
        let mut tracker = DamageTracker::new();
        tracker.record_frame(Some(DamageRect {
            x: 0,
            y: 0,
            w: 50,
            h: 50,
        }));
        let current = DamageRect {
            x: 100,
            y: 200,
            w: 80,
            h: 60,
        };

        assert_eq!(
            tracker.compute_damage(Some(1), Some(current), SURFACE),
            Some(current)
        );
        assert_eq!(tracker.compute_damage(Some(1), None, SURFACE), None);
    }

    #[test]
    fn buffer_age_2_unions_current_with_previous_frame() {
        let mut tracker = DamageTracker::new();
        let prev = DamageRect {
            x: 10,
            y: 20,
            w: 30,
            h: 40,
        };
        let current = DamageRect {
            x: 100,
            y: 120,
            w: 50,
            h: 60,
        };
        tracker.record_frame(Some(prev));

        assert_eq!(
            tracker.compute_damage(Some(2), Some(current), SURFACE),
            Some(DamageRect {
                x: 10,
                y: 20,
                w: 140,
                h: 160
            })
        );
    }

    #[test]
    fn buffer_age_3_unions_current_with_previous_two_frames() {
        let mut tracker = DamageTracker::new();
        let older = DamageRect {
            x: 500,
            y: 400,
            w: 100,
            h: 100,
        };
        let prev = DamageRect {
            x: 10,
            y: 20,
            w: 30,
            h: 40,
        };
        let current = DamageRect {
            x: 100,
            y: 120,
            w: 50,
            h: 60,
        };
        tracker.record_frame(Some(older));
        tracker.record_frame(Some(prev));

        assert_eq!(
            tracker.compute_damage(Some(3), Some(current), SURFACE),
            Some(DamageRect {
                x: 10,
                y: 20,
                w: 590,
                h: 480
            })
        );
    }

    #[test]
    fn to_egl_rect_flips_y_correctly() {
        let rect = DamageRect {
            x: 12,
            y: 30,
            w: 100,
            h: 40,
        };
        assert_eq!(rect.to_egl_rect(1080), [12, 1010, 100, 40]);

        let bottom = DamageRect {
            x: 0,
            y: 1040,
            w: 1920,
            h: 40,
        };
        assert_eq!(bottom.to_egl_rect(1080), [0, 0, 1920, 40]);
    }

    #[test]
    fn mark_full_forces_full_damage_regardless_of_buffer_age() {
        let mut tracker = DamageTracker::new();
        let current = DamageRect {
            x: 10,
            y: 10,
            w: 20,
            h: 20,
        };
        tracker.record_frame(Some(current));
        tracker.mark_full();

        let full = Some(DamageRect {
            x: 0,
            y: 0,
            w: 1920,
            h: 1080,
        });
        assert_eq!(
            tracker.compute_damage(Some(1), Some(current), SURFACE),
            full
        );
        assert_eq!(
            tracker.compute_damage(Some(2), Some(current), SURFACE),
            full
        );
        assert_eq!(
            tracker.compute_damage(Some(3), Some(current), SURFACE),
            full
        );

        tracker.record_frame(Some(current));
        assert_eq!(
            tracker.compute_damage(Some(1), Some(current), SURFACE),
            Some(current)
        );
    }

    #[test]
    fn compute_damage_clamps_to_surface_bounds() {
        let tracker = DamageTracker::new();
        let oversized = DamageRect {
            x: -20,
            y: -10,
            w: 2000,
            h: 1200,
        };
        assert_eq!(
            tracker.compute_damage(Some(1), Some(oversized), SURFACE),
            Some(DamageRect {
                x: 0,
                y: 0,
                w: 1920,
                h: 1080
            })
        );
    }
}
