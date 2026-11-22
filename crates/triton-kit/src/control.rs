//! Interactive controls: [`switch`], [`checkbox`], [`radio`], [`slider`].
//!
//! Every transition runs through [`Animator`] springs keyed by [`NodeKey`] (XOR
//! salts give each control private sub-channels), so `reduce_motion` degrades
//! them all to the standard 120 ms fade with zero per-widget opt-in — callers
//! never store per-control `Animated` state.

use triton_design::color::STATE_LAYER;
use triton_design::icon::{Icon, IconStyle};
use triton_design::motion::Spring;
use triton_design::shape::{CORNER_SMOOTHING, SIZE};
use triton_engine::anim::Animator;
use triton_engine::style::{Corners, Rgba, Theme};
use triton_engine::ui::{Element, InteractionState, NodeKey, box_, icon};

// ── Animator sub-channel salts ───────────────────────────────────────────────
// XOR onto a control's `NodeKey` to derive private animation channels; XOR is a
// bijection so distinct keys can never collide.

const CHECKED_SALT: u64 = 0xc7c1_0000_0000_0001;
const LAYER_SALT: u64 = 0xc7c1_0000_0000_0002;
const MARK_SALT: u64 = 0xc7c1_0000_0000_0003;
const HANDLE_SALT: u64 = 0xc7c1_0000_0000_0004;

/// Absolute-positioned rounded box — the quad primitive behind track segments,
/// thumbs, handles, stop dots, and state-layer halos.
fn quad_el<M>(x: f32, y: f32, w: f32, h: f32, radii: Corners, col: Rgba) -> Element<M> {
    box_()
        .absolute()
        .inset_xy(x, y)
        .size(w, h)
        .rounded(radii)
        .smoothing(0.0)
        .bg(col)
}

/// `SIZE.touch_min` circular hover/press/drag state layer centered on `(cx, cy)`,
/// emitted only while its faded alpha is visible.
fn state_halo<M>(cx: f32, cy: f32, col: Rgba, alpha: f32) -> Option<Element<M>> {
    (alpha > 0.004).then(|| {
        quad_el(
            cx - SIZE.touch_min * 0.5,
            cy - SIZE.touch_min * 0.5,
            SIZE.touch_min,
            SIZE.touch_min,
            Corners::all(SIZE.touch_min * 0.5),
            col.with_alpha(alpha),
        )
    })
}

fn checked_progress(key: NodeKey, on: bool, anim: &mut Animator) -> f32 {
    anim.spring(
        NodeKey(key.0 ^ CHECKED_SALT),
        f32::from(u8::from(on)),
        Spring::SPATIAL_FAST,
    )
    .clamp(0.0, 1.0)
}

/// Alpha of the hover/press/drag state layer for `key`, faded via `Animator`.
fn state_layer_alpha(
    key: NodeKey,
    disabled: bool,
    interaction: &InteractionState,
    anim: &mut Animator,
) -> f32 {
    let target = if disabled {
        0.0
    } else if interaction.is_dragging(key) {
        STATE_LAYER.drag
    } else if interaction.is_pressed(key) {
        STATE_LAYER.press
    } else if interaction.is_hovered(key) {
        STATE_LAYER.hover
    } else {
        0.0
    };
    anim.spring(NodeKey(key.0 ^ LAYER_SALT), target, Spring::EFFECTS_FAST)
}

// ── Switch ───────────────────────────────────────────────────────────────────

/// Material 3 switch (`SIZE.switch` = `52×32` pill track).
///
/// `Components.dc.html` `.t-switch`: off = `surface_container_highest` + 2px
/// `outline` border, thumb 16px `outline` resting left; on = `primary`
/// track+border, thumb 24px `on_primary` at right with a `check` icon. The
/// thumb slide/diameter and the M3 40px hover/press state layer behind the
/// thumb all animate through `anim` — no per-widget animation state needed.
pub fn switch<M: 'static>(
    key: impl Into<NodeKey>,
    checked: bool,
    disabled: bool,
    theme: &Theme,
    interaction: &InteractionState,
    anim: &mut Animator,
    on_toggle: impl Fn() -> M + 'static,
) -> Element<M> {
    let key = key.into();
    let (track_w, track_h) = SIZE.switch;
    let t = checked_progress(key, checked, anim);
    let halo_a = state_layer_alpha(key, disabled, interaction, anim);

    let on_surface = Rgba::from_color(theme.scheme.on_surface);
    let (off_bg, on_bg, off_border, off_thumb, on_thumb) = if disabled {
        let dim = on_surface.with_alpha(STATE_LAYER.disabled_container);
        (
            dim,
            dim,
            dim,
            on_surface.with_alpha(STATE_LAYER.disabled_content),
            Rgba::from_color(theme.scheme.surface),
        )
    } else {
        (
            Rgba::from_color(theme.scheme.surface_container_highest),
            Rgba::from_color(theme.scheme.primary),
            Rgba::from_color(theme.scheme.outline),
            Rgba::from_color(theme.scheme.outline),
            Rgba::from_color(theme.scheme.on_primary),
        )
    };
    let on_border = on_bg;
    let track_bg = off_bg.lerp(on_bg, t);
    let border_col = off_border.lerp(on_border, t);
    let thumb_col = off_thumb.lerp(on_thumb, t);

    let thumb_d = 16.0 + 8.0 * t; // 16px off -> 24px on
    let thumb_cx = 16.0 + 20.0 * t; // center travels 16 -> 36 (board: 8+16t left edge)
    let thumb_cy = track_h * 0.5;
    let thumb_x = thumb_cx - thumb_d * 0.5;
    let thumb_y = thumb_cy - thumb_d * 0.5;

    let layer_col = on_surface.lerp(Rgba::from_color(theme.scheme.primary), t);
    let check_alpha = ((t - 0.55) / 0.45).clamp(0.0, 1.0);
    let check_col = if disabled {
        on_surface.with_alpha(STATE_LAYER.disabled_content)
    } else {
        Rgba::from_color(theme.scheme.primary)
    };

    let mut thumb = box_()
        .absolute()
        .inset_xy(thumb_x, thumb_y)
        .size(thumb_d, thumb_d)
        .items_center()
        .justify_center()
        .rounded(thumb_d * 0.5)
        .smoothing(0.0)
        .bg(thumb_col);
    if check_alpha > 0.01 {
        thumb = thumb.child(icon(
            Icon::Check,
            IconStyle::Filled,
            16.0,
            check_col.with_alpha(check_alpha),
        ));
    }

    let mut track = box_()
        .id(key)
        .size(track_w, track_h)
        .rounded(track_h * 0.5)
        .smoothing(0.0)
        .bg(track_bg)
        .border(2.0, border_col)
        .child_opt(state_halo(thumb_cx, thumb_cy, layer_col, halo_a))
        .child(thumb);

    if !disabled {
        track = track.on_click(on_toggle);
    }
    track
}

// ── Checkbox ─────────────────────────────────────────────────────────────────

/// Checked-ness of a [`checkbox`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CheckState {
    Unchecked,
    Checked,
    /// Primary fill with a dash mark — a "some children selected" state.
    Indeterminate,
}

/// Material 3 checkbox: a `20px` smooth squircle centered in the `40px`
/// (`SIZE.touch_min`) hit target. Off = 2px `outline` border over transparent;
/// checked/indeterminate = `primary` fill + `check`/`remove` mark that crossfades.
/// The state layer is the 40px circle itself, faded through `anim`.
pub fn checkbox<M: 'static>(
    key: impl Into<NodeKey>,
    state: CheckState,
    disabled: bool,
    theme: &Theme,
    interaction: &InteractionState,
    anim: &mut Animator,
    on_toggle: impl Fn() -> M + 'static,
) -> Element<M> {
    let key = key.into();
    let on = !matches!(state, CheckState::Unchecked);
    let t = checked_progress(key, on, anim);
    let mark_t = anim
        .spring(
            NodeKey(key.0 ^ MARK_SALT),
            f32::from(u8::from(matches!(state, CheckState::Indeterminate))),
            Spring::EFFECTS,
        )
        .clamp(0.0, 1.0);
    let layer_a = state_layer_alpha(key, disabled, interaction, anim);

    let on_surface = Rgba::from_color(theme.scheme.on_surface);
    let (fill_off, fill_on, border_off, border_on, mark_col) = if disabled {
        let content = on_surface.with_alpha(STATE_LAYER.disabled_content);
        (
            Rgba::TRANSPARENT,
            content,
            content,
            content,
            Rgba::from_color(theme.scheme.surface),
        )
    } else {
        let primary = Rgba::from_color(theme.scheme.primary);
        (
            Rgba::TRANSPARENT,
            primary,
            Rgba::from_color(theme.scheme.outline),
            primary,
            Rgba::from_color(theme.scheme.on_primary),
        )
    };
    let fill = fill_off.lerp(fill_on, t);
    let border_col = border_off.lerp(border_on, t);
    let layer_col = on_surface.lerp(Rgba::from_color(theme.scheme.primary), t);

    let mut bx = box_()
        .size(20.0, 20.0)
        .items_center()
        .justify_center()
        .rounded(4.0)
        .smoothing(CORNER_SMOOTHING)
        .border(2.0, border_col)
        .bg(fill);
    let check_a = t * (1.0 - mark_t);
    let dash_a = t * mark_t;
    if check_a > 0.01 {
        bx = bx.child(
            box_()
                .absolute()
                .inset_all(0.0)
                .items_center()
                .justify_center()
                .child(icon(
                    Icon::Check,
                    IconStyle::Filled,
                    18.0,
                    mark_col.with_alpha(check_a),
                )),
        );
    }
    if dash_a > 0.01 {
        bx = bx.child(
            box_()
                .absolute()
                .inset_all(0.0)
                .items_center()
                .justify_center()
                .child(icon(
                    Icon::Remove,
                    IconStyle::Filled,
                    18.0,
                    mark_col.with_alpha(dash_a),
                )),
        );
    }

    let mut target = box_()
        .id(key)
        .size(SIZE.touch_min, SIZE.touch_min)
        .items_center()
        .justify_center()
        .rounded(SIZE.touch_min * 0.5)
        .smoothing(0.0)
        .bg(layer_col.with_alpha(layer_a))
        .child(bx);
    if !disabled {
        target = target.on_click(on_toggle);
    }
    target
}

// ── Radio ────────────────────────────────────────────────────────────────────

/// Material 3 radio: a `20px` circular ring centered in the `40px`
/// (`SIZE.touch_min`) hit target. Off = 2px `outline` ring; selected = `primary`
/// ring + a `10px` inner dot that springs in. The state layer is the 40px
/// circle itself, faded through `anim`.
pub fn radio<M: 'static>(
    key: impl Into<NodeKey>,
    selected: bool,
    disabled: bool,
    theme: &Theme,
    interaction: &InteractionState,
    anim: &mut Animator,
    on_select: impl Fn() -> M + 'static,
) -> Element<M> {
    let key = key.into();
    let t = checked_progress(key, selected, anim);
    let layer_a = state_layer_alpha(key, disabled, interaction, anim);

    let on_surface = Rgba::from_color(theme.scheme.on_surface);
    let (ring_off, ring_on) = if disabled {
        let content = on_surface.with_alpha(STATE_LAYER.disabled_content);
        (content, content)
    } else {
        (
            Rgba::from_color(theme.scheme.outline),
            Rgba::from_color(theme.scheme.primary),
        )
    };
    let ring_col = ring_off.lerp(ring_on, t);
    let dot_col = if disabled {
        on_surface.with_alpha(STATE_LAYER.disabled_content)
    } else {
        Rgba::from_color(theme.scheme.primary)
    };
    let layer_col = on_surface.lerp(Rgba::from_color(theme.scheme.primary), t);

    let dot_d = 10.0 * t;
    let mut ring = box_()
        .size(20.0, 20.0)
        .rounded(10.0)
        .smoothing(0.0)
        .border(2.0, ring_col)
        .bg(Rgba::TRANSPARENT);
    if dot_d > 0.5 {
        ring = ring.child(
            box_()
                .absolute()
                .inset_xy(10.0 - dot_d * 0.5, 10.0 - dot_d * 0.5)
                .size(dot_d, dot_d)
                .rounded(dot_d * 0.5)
                .smoothing(0.0)
                .bg(dot_col),
        );
    }

    let mut target = box_()
        .id(key)
        .size(SIZE.touch_min, SIZE.touch_min)
        .items_center()
        .justify_center()
        .rounded(SIZE.touch_min * 0.5)
        .smoothing(0.0)
        .bg(layer_col.with_alpha(layer_a))
        .child(ring);
    if !disabled {
        target = target.on_click(on_select);
    }
    target
}

// ── Slider ───────────────────────────────────────────────────────────────────

/// Value behavior of a [`slider`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SliderMode {
    /// Free `0.0..=1.0` values.
    Continuous,
    /// `steps` equal intervals (`steps + 1` stops). Emitted values snap to the
    /// nearest stop and `4×4` stop dots render along the track — `on_primary`
    /// on the active side, `primary` on the inactive side (`.sld .d` board rule).
    Stepped { steps: u32 },
}

/// Inactive-track treatment of a [`slider`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SliderSurface {
    /// `surface_container_highest` — standard slider on opaque surfaces.
    Standard,
    /// Higher-contrast inactive track for Glass/Thick panels: `on_surface` at
    /// the `STATE_LAYER.drag` alpha so it reads over the translucent fill.
    OnGlass,
}

/// Layout and behavior options for [`slider`].
#[derive(Debug, Clone, Copy)]
pub struct SliderOptions {
    pub mode: SliderMode,
    pub surface: SliderSurface,
    /// Greyed-out, non-interactive presentation.
    pub disabled: bool,
    /// Handle `(w, h)` in logical px; default `SIZE.slider_handle` = `4×44`.
    /// The container is `max(h, SIZE.slider_track)` tall, so taller handles
    /// overshoot the track top/bottom as on the boards.
    pub handle: (f32, f32),
}

impl Default for SliderOptions {
    fn default() -> Self {
        Self {
            mode: SliderMode::Continuous,
            surface: SliderSurface::Standard,
            disabled: false,
            handle: SIZE.slider_handle,
        }
    }
}

fn snapped(v: f32, mode: SliderMode) -> f32 {
    let v = v.clamp(0.0, 1.0);
    match mode {
        SliderMode::Continuous => v,
        SliderMode::Stepped { steps } => {
            let steps = steps.max(1) as f32;
            (v * steps).round() / steps
        }
    }
}

/// `4×4` r2 stop dots, one per stop — `on_primary` on the active side,
/// `primary` on the inactive side (`.sld .d` board rule).
fn stop_dots<M>(
    steps: u32,
    t: f32,
    total_w: f32,
    center_y: f32,
    dot_on: Rgba,
    dot_off: Rgba,
) -> impl Iterator<Item = Element<M>> {
    let steps_f = steps.max(1) as f32;
    (0..=steps.max(1)).map(move |i| {
        let stop_t = i as f32 / steps_f;
        let col = if stop_t <= t { dot_on } else { dot_off };
        quad_el(
            stop_t * total_w - 2.0,
            center_y - 2.0,
            4.0,
            4.0,
            Corners::all(2.0),
            col,
        )
    })
}

/// M3 Expressive split-track horizontal slider matching `.t-slider`/`.sld` on
/// `Controls.dc.html`, `QuickSettings.dc.html`, and `SettingsStates.dc.html`:
/// `16px` (`SIZE.slider_track`) track split 6px around a `4×44`
/// (`SIZE.slider_handle`) handle centered at `value * width`, active radii
/// `[8,2,2,8]` `primary`, inactive `[2,8,8,2]` `surface_container_highest`.
///
/// `on_drag` covers click-to-seek (the engine fires it on pointer-down) and
/// captures the pointer outside horizontal bounds. While dragging the handle
/// tracks 1:1; programmatic value changes spring via `anim`. The 40px
/// hover/press/drag state-layer halo behind the handle fades through `anim`.
#[expect(
    clippy::too_many_arguments,
    reason = "widget fn signature mirrors the element contract: identity, value, \
              layout, options, theme, live interaction + animator, and the message \
              sink are all independent axes the caller owns"
)]
pub fn slider<M: 'static>(
    key: impl Into<NodeKey>,
    value: f32,
    width: f32,
    options: SliderOptions,
    theme: &Theme,
    interaction: &InteractionState,
    anim: &mut Animator,
    on_change: impl Fn(f32) -> M + 'static,
) -> Element<M> {
    let key = key.into();
    let t = value.clamp(0.0, 1.0);
    let track_h = SIZE.slider_track;
    let (handle_w, handle_h) = options.handle;
    let total_w = width.max(SIZE.touch_min);
    let height = handle_h.max(track_h);
    let gap = 6.0;

    let dragging = !options.disabled && interaction.is_dragging(key);
    let target_cx = t * total_w;
    // Reseed the channel at the pointer position every drag frame so the
    // handle tracks 1:1 and hands off to the spring with no jump on release.
    let handle_cx = if dragging {
        anim.spring_with_velocity(
            NodeKey(key.0 ^ HANDLE_SALT),
            target_cx,
            target_cx,
            0.0,
            Spring::SPATIAL,
            0.5,
        )
    } else {
        anim.spring_px(NodeKey(key.0 ^ HANDLE_SALT), target_cx, Spring::SPATIAL)
    };
    let halo_a = state_layer_alpha(key, options.disabled, interaction, anim);

    let on_surface = Rgba::from_color(theme.scheme.on_surface);
    let primary = Rgba::from_color(theme.scheme.primary);
    let (active_col, inactive_col, handle_col, halo_col) = if options.disabled {
        let content = on_surface.with_alpha(STATE_LAYER.disabled_content);
        (
            content,
            on_surface.with_alpha(STATE_LAYER.disabled_container),
            content,
            Rgba::TRANSPARENT,
        )
    } else {
        let inactive = match options.surface {
            SliderSurface::Standard => Rgba::from_color(theme.scheme.surface_container_highest),
            SliderSurface::OnGlass => on_surface.with_alpha(STATE_LAYER.drag),
        };
        (primary, inactive, primary, primary)
    };

    let track_y = (height - track_h) * 0.5;
    let active_w = (handle_cx - gap).max(0.0);
    let inactive_x = (handle_cx + gap).min(total_w);
    let inactive_w = (total_w - inactive_x).max(0.0);

    let mut root = box_().id(key).size(total_w, height);
    if !options.disabled {
        root = root.on_drag(move |px, _py, rect| {
            on_change(snapped((px - rect.x) / rect.w.max(1.0), options.mode))
        });
    }

    root = root.child_opt(state_halo(handle_cx, height * 0.5, halo_col, halo_a));

    if active_w > 0.5 {
        root = root.child(quad_el(
            0.0,
            track_y,
            active_w,
            track_h,
            Corners::each(8.0, 2.0, 2.0, 8.0),
            active_col,
        ));
    }
    if inactive_w > 0.5 {
        root = root.child(quad_el(
            inactive_x,
            track_y,
            inactive_w,
            track_h,
            Corners::each(2.0, 8.0, 8.0, 2.0),
            inactive_col,
        ));
    }

    if let SliderMode::Stepped { steps } = options.mode {
        let (dot_on, dot_off) = if options.disabled {
            (
                Rgba::from_color(theme.scheme.surface),
                on_surface.with_alpha(STATE_LAYER.disabled_content),
            )
        } else {
            (Rgba::from_color(theme.scheme.on_primary), primary)
        };
        root = root.children(stop_dots(steps, t, total_w, height * 0.5, dot_on, dot_off));
    }

    root.child(quad_el(
        handle_cx - handle_w * 0.5,
        (height - handle_h) * 0.5,
        handle_w,
        handle_h,
        Corners::all(2.0),
        handle_col,
    ))
}
