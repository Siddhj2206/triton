//! Morph-in-place Glass OSD: payloads, presence state, surface spec, and view.
//!
//! A single pill on `Layer::Overlay` (`SurfaceSize::StableExtent` + shaped
//! input) mounted on the focused output. Payload switches and level changes
//! morph in place — the surface id never changes, so the compositor keeps the
//! same layer surface and EGL buffer. Hold expiry starts an exit morph; the
//! surface stays mounted until the morph settles, then unmounts cleanly.
//!
//! The pill holds no interactive elements, so `BuiltFrame::input_rects` stays
//! empty and every click and drag passes through to windows underneath.

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use triton_design::icon::Icon;
use triton_design::material::Material;
use triton_design::motion::{Animated, Motion, Spring};
use triton_design::shape::{RADII, SIZE, SPACE};
use triton_design::typography::TypeStyle;
use triton_engine::{
    Element, NodeKey, SubtreeTransform, SurfaceAnchor, SurfaceKeyboard, SurfaceLayer, SurfaceSize,
    SurfaceSpec, Theme, ViewContext, box_, row, text,
};
use triton_kit::{Progress, icon_disc_styled, panel, shadow_gutter, wavy_progress};

use crate::state::SurfaceRole;

/// Timer tag for the 1.2 s hold deadline (re-armed on every OSD event).
pub const TIMER_OSD_HIDE: u32 = 1;

/// Timer tag polled while the exit morph settles before unmounting.
///
/// Springs have no fixed duration, so `on_timer` re-arms this tag until
/// [`OsdState::is_settled`] reports the exit morph is done.
pub const TIMER_OSD_REAP: u32 = 3;

/// OSD hold duration after the latest event (`docs/design-spec.md`: 1.2 s).
pub const OSD_HOLD_DURATION: Duration = Duration::from_millis(1200);

/// Re-check cadence while the exit morph is in flight.
pub const OSD_REAP_INTERVAL: Duration = Duration::from_millis(100);

/// Stable max extent for the pill: the layer surface and EGL buffer are
/// allocated once so every payload switch and level change morphs in place.
const OSD_EXTENT_W: u32 = 420;
const OSD_EXTENT_H: u32 = 224;

/// Logical width of the slot next to the icon disc (progress track or label).
const CONTENT_W: f32 = 160.0;

/// Upward travel across the enter/exit morph, in logical px.
const LIFT_PX: f32 = 12.0;

/// Scale at the start and end of the morph (`progress = 0`).
const MIN_SCALE: f32 = 0.92;

/// Settle epsilon for the presence channel (dimensionless `0.0..=1.0`).
const PRESENCE_EPSILON: f32 = 0.001;

/// Spatial spring driving the morph; `Motion::pick` swaps it for a 120 ms
/// linear fade under `reduce_motion` (`docs/design-spec.md`).
const MORPH_SPRING: Spring = Spring::SPATIAL;

/// Typed payload displayed by the morph-in-place OSD pill.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
#[non_exhaustive]
pub enum OsdPayload {
    Volume { level: f32, muted: bool },
    Brightness { level: f32 },
    Mic { muted: bool },
    KeyboardLayout { name: String },
    Workspace { label: String },
    LowBattery { percent: u8 },
}

/// Active OSD state: the payload, the output the pill is bound to, and the
/// app-owned presence animation that drives the enter/exit morph.
///
/// `presence` is owned here (not on the frame [`triton_engine::Animator`]) so
/// `is_animating` can keep frames flowing during the morph and `surfaces()`
/// can keep the surface listed until the exit settles.
#[derive(Debug, Clone, PartialEq)]
pub struct OsdState {
    pub payload: OsdPayload,
    /// Compositor output name the pill is bound to.
    pub output: String,
    /// Logical visibility: `true` while shown, `false` while the exit morph is
    /// in flight. The surface stays mounted until reaped.
    pub visible: bool,
    presence: Animated,
}

impl OsdState {
    /// Begin a fresh OSD: mounts at presence 0 and morphs in toward 1.
    pub fn entering(
        payload: OsdPayload,
        output: String,
        reduce_motion: bool,
        now: Instant,
    ) -> Self {
        let mut presence = Animated::new(0.0, Self::motion(reduce_motion), PRESENCE_EPSILON, now);
        presence.set_target(1.0, now);
        Self {
            payload,
            output,
            visible: true,
            presence,
        }
    }

    /// Update the payload in place and morph back to fully shown — used for
    /// repeat events and mid-exit revivals without surface teardown. The
    /// spring keeps its velocity, so interrupted exits never jump.
    pub fn show(&mut self, payload: OsdPayload, output: String, now: Instant) {
        self.payload = payload;
        self.output = output;
        self.visible = true;
        self.presence.set_target(1.0, now);
    }

    /// Begin the exit morph; the surface stays mounted until
    /// [`Self::is_settled`], then is unmounted by the reap timer.
    pub fn dismiss(&mut self, now: Instant) {
        self.visible = false;
        self.presence.set_target(0.0, now);
    }

    /// `true` once the exit morph was requested.
    pub fn is_exiting(&self) -> bool {
        self.presence.target() < 0.5
    }

    /// `true` when the presence channel reached its target — safe to unmount
    /// when exiting, done animating when entering.
    pub fn is_settled(&self, now: Instant) -> bool {
        self.presence.is_settled(now)
    }

    /// Swap the morph motion when the global `reduce_motion` flag flips,
    /// without jumping the in-flight value.
    pub fn set_reduce_motion(&mut self, reduce_motion: bool, now: Instant) {
        self.presence.set_motion(Self::motion(reduce_motion), now);
    }

    /// Current morph progress. The spatial spring may overshoot `1.0`
    /// slightly; callers clamp for opacity and let the overshoot read as the
    /// spring's settle.
    pub fn progress(&self, now: Instant) -> f32 {
        self.presence.value(now)
    }

    fn motion(reduce_motion: bool) -> Motion {
        Motion::pick(MORPH_SPRING, reduce_motion)
    }
}

/// [`SurfaceSpec`] for the OSD pill on `output_name`.
///
/// `StableExtent` + `shaped_input`: the surface is allocated once at max
/// extent and reports input regions only for `captures_input` elements — of
/// which the pill has none, so it is fully pointer-transparent.
pub fn surface_spec(output_name: &str) -> SurfaceSpec {
    SurfaceSpec {
        namespace: SurfaceRole::Osd.namespace().to_owned(),
        layer: SurfaceLayer::Overlay,
        anchor: SurfaceAnchor::BOTTOM,
        margin: (0, 0, 48, 0),
        keyboard: SurfaceKeyboard::None,
        exclusive_zone: 0,
        sizing: SurfaceSize::StableExtent {
            width: OSD_EXTENT_W,
            height: OSD_EXTENT_H,
        },
        shaped_input: true,
        output: Some(output_name.to_owned()),
    }
}

/// Render the OSD pill for `state` at `ctx.now`.
///
/// Composes `triton-kit` components only (`panel`, `icon_disc_styled`,
/// `wavy_progress`, `shadow_gutter`) and mounts no interactive elements, so
/// `input_rects` stays empty while the Glass panel reports `blur_rects`.
pub fn view<M>(
    state: &OsdState,
    theme: &Theme,
    reduce_motion: bool,
    ctx: &mut ViewContext<'_>,
) -> Element<M> {
    let content = payload_content(&state.payload, theme, ctx);

    let progress = state.progress(ctx.now).clamp(0.0, 1.0);
    // reduce_motion: 120 ms opacity fade only. Otherwise a spring morph —
    // scale up and rise while fading in (the progress spring overshoots a
    // touch, which reads as the settle).
    let tf = if reduce_motion {
        SubtreeTransform {
            opacity: progress,
            ..SubtreeTransform::IDENTITY
        }
    } else {
        SubtreeTransform {
            translate_x: 0.0,
            translate_y: (1.0 - progress) * LIFT_PX,
            scale: MIN_SCALE + (1.0 - MIN_SCALE) * progress,
            opacity: progress,
        }
    };

    let pill = shadow_gutter(
        panel(Material::Glass, RADII.full, theme)
            .px(SPACE[4])
            .py(SPACE[2])
            .items_center()
            .child(content),
    )
    .transform(tf);

    input_transparent(
        box_()
            .w_full()
            .h_full()
            .items_center()
            .justify_center()
            .child(pill),
    )
}

/// Clear `captures_input` across the whole subtree.
///
/// Painted elements claim pointer input by default (`.bg`/`.material` set
/// `captures_input`), which is what `shaped_input` surfaces turn into Wayland
/// input rects. The OSD must be fully pointer-transparent, so the composed
/// pill tree is stripped post-composition — `input_rects` stays empty and
/// clicks and drags pass straight through to windows underneath.
fn input_transparent<M>(mut el: Element<M>) -> Element<M> {
    el.captures_input = false;
    el.children = el.children.into_iter().map(input_transparent).collect();
    el
}

/// Icon disc + payload slot (progress track or label), uniform across
/// payloads so switches morph inside stable chrome.
fn payload_content<M>(
    payload: &OsdPayload,
    theme: &Theme,
    ctx: &mut ViewContext<'_>,
) -> Element<M> {
    let (glyph, slot) = match payload {
        OsdPayload::Volume { level, muted } => (
            if *muted {
                Icon::VolumeOff
            } else {
                Icon::VolumeUp
            },
            Slot::Level(level.clamp(0.0, 1.0)),
        ),
        OsdPayload::Brightness { level } => {
            (Icon::BrightnessHigh, Slot::Level(level.clamp(0.0, 1.0)))
        }
        OsdPayload::Mic { muted } => (
            if *muted { Icon::MicOff } else { Icon::Mic },
            Slot::Label(if *muted {
                "Microphone muted".to_owned()
            } else {
                "Microphone on".to_owned()
            }),
        ),
        OsdPayload::KeyboardLayout { name } => (Icon::Keyboard, Slot::Label(name.clone())),
        OsdPayload::Workspace { label } => (Icon::GridView, Slot::Label(label.clone())),
        OsdPayload::LowBattery { percent } => (
            Icon::BatteryAlert,
            Slot::Label(format!("Battery {percent}%")),
        ),
    };

    let disc = if matches!(payload, OsdPayload::LowBattery { .. }) {
        icon_disc_styled(
            glyph,
            SIZE.control_m,
            theme.scheme.error_container,
            theme.scheme.on_error_container,
        )
    } else {
        icon_disc_styled(
            glyph,
            SIZE.control_m,
            theme.scheme.secondary_container,
            theme.scheme.on_secondary_container,
        )
    };

    let slot_el = match slot {
        Slot::Level(level) => wavy_progress(
            NodeKey::named("osd-progress"),
            CONTENT_W,
            4.0,
            Progress::Determinate(level),
            theme,
            ctx.anim,
        ),
        Slot::Label(label) => box_().w(CONTENT_W).clip().items_center().child(text(
            label,
            TypeStyle::LABEL_L,
            theme.scheme.on_surface,
        )),
    };

    row()
        .gap(SPACE[3])
        .items_center()
        .child(disc)
        .child(slot_el)
}

enum Slot {
    Level(f32),
    Label(String),
}
