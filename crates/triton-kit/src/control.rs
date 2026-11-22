//! Continuous and binary controls: [`slider`] and [`switch`].

use std::time::Instant;

use triton_design::icon::{Icon, IconStyle};
use triton_design::motion::{Animated, Motion, Spring};
use triton_design::shape::SIZE;
use triton_engine::style::{Corners, Rgba, Theme};
use triton_engine::ui::{Element, NodeKey, box_, icon};

// ── Slider ───────────────────────────────────────────────────────────────────

/// M3 Expressive split-track horizontal slider (active track, 4px vertical handle,
/// inactive track) matching `QuickSettings.dc.html` and `Controls.dc.html`.
#[allow(clippy::too_many_arguments)]
pub fn slider<M: 'static>(
    key: impl Into<NodeKey>,
    value: f32,
    width: f32,
    theme: &Theme,
    on_change: impl Fn(f32) -> M + 'static,
) -> Element<M> {
    let t = value.clamp(0.0, 1.0);
    let track_h = SIZE.slider_track; // 16px
    let (handle_w, handle_h) = SIZE.slider_handle; // 4x44px
    let gap = 6.0;
    let total_w = width.max(60.0);
    let handle_x = (t * (total_w - handle_w)).clamp(0.0, total_w - handle_w);
    let active_w = (handle_x - gap).max(0.0);
    let inactive_x = (handle_x + handle_w + gap).min(total_w);
    let inactive_w = (total_w - inactive_x).max(0.0);
    let track_y = (handle_h - track_h) * 0.5;

    let active_col = Rgba::from_color(theme.scheme.primary);
    let inactive_col = Rgba::new(1.0, 1.0, 1.0, 0.18);

    let mut root = box_()
        .id(key)
        .size(total_w, handle_h)
        .on_drag(move |px, _py, rect| {
            let v = ((px - rect.x) / rect.w.max(1.0)).clamp(0.0, 1.0);
            on_change(v)
        });

    if active_w > 1.0 {
        root = root.child(
            box_()
                .absolute()
                .inset_xy(0.0, track_y)
                .size(active_w, track_h)
                .rounded(Corners::each(8.0, 2.0, 2.0, 8.0))
                .smoothing(0.0)
                .bg(active_col),
        );
    }

    if inactive_w > 1.0 {
        root = root.child(
            box_()
                .absolute()
                .inset_xy(inactive_x, track_y)
                .size(inactive_w, track_h)
                .rounded(Corners::each(2.0, 8.0, 8.0, 2.0))
                .smoothing(0.0)
                .bg(inactive_col),
        );
    }

    root.child(
        box_()
            .absolute()
            .inset_xy(handle_x, 0.0)
            .size(handle_w, handle_h)
            .rounded(2.0)
            .smoothing(0.0)
            .bg(active_col),
    )
}

// ── Switch ───────────────────────────────────────────────────────────────────

/// Spring-backed state for a Material 3 toggle [`switch`].
#[derive(Debug, Clone)]
pub struct SwitchState {
    pub checked: bool,
    pub anim: Animated,
}

impl SwitchState {
    pub fn new(checked: bool, now: Instant) -> Self {
        let initial = f32::from(u8::from(checked));
        Self {
            checked,
            anim: Animated::new(initial, Motion::Spring(Spring::SPATIAL_FAST), 0.005, now),
        }
    }

    pub fn toggle(&mut self, now: Instant) {
        self.set(!self.checked, now);
    }

    pub fn set(&mut self, checked: bool, now: Instant) {
        self.checked = checked;
        self.anim.set_target(f32::from(u8::from(checked)), now);
    }

    pub fn is_animating(&self, now: Instant) -> bool {
        !self.anim.is_settled(now)
    }
}

/// Spring-animated Material 3 switch (`52×32` pill track, morphing thumb + check icon).
pub fn switch<M: 'static>(
    key: impl Into<NodeKey>,
    state: &SwitchState,
    now: Instant,
    theme: &Theme,
    on_toggle: impl Fn() -> M + 'static,
) -> Element<M> {
    let (track_w, track_h) = SIZE.switch; // 52x32
    let t = state.anim.value(now).clamp(0.0, 1.0);

    let off_bg = Rgba::from_color(theme.scheme.surface_container_highest);
    let on_bg = Rgba::from_color(theme.scheme.primary);
    let track_bg = off_bg.lerp(on_bg, t);
    let border_col = Rgba::from_color(theme.scheme.outline).lerp(on_bg, t);

    let thumb_d = 16.0 + 8.0 * t; // 16px off -> 24px on
    let thumb_x = 8.0 + (track_w - 4.0 - 24.0 - 8.0) * t;
    let thumb_y = (track_h - thumb_d) * 0.5;
    let thumb_col =
        Rgba::from_color(theme.scheme.outline).lerp(Rgba::from_color(theme.scheme.on_primary), t);

    let mut thumb = box_()
        .absolute()
        .inset_xy(thumb_x, thumb_y)
        .size(thumb_d, thumb_d)
        .items_center()
        .justify_center()
        .rounded(thumb_d * 0.5)
        .smoothing(0.0)
        .bg(thumb_col);

    if t > 0.55 {
        let check_alpha = ((t - 0.55) / 0.45).clamp(0.0, 1.0);
        thumb = thumb.child(icon(
            Icon::Check,
            IconStyle::Filled,
            15.0,
            Rgba::from_color(theme.scheme.primary).with_alpha(check_alpha),
        ));
    }

    box_()
        .id(key)
        .size(track_w, track_h)
        .rounded(track_h * 0.5)
        .smoothing(0.0)
        .bg(track_bg)
        .border(2.0 * (1.0 - t * 0.5), border_col)
        .on_click(on_toggle)
        .child(thumb)
}
