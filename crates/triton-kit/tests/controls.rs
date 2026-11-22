//! Headless `DrawList` integration tests for triton-kit Phase 1A.3 controls:
//! `switch`, `checkbox`, `radio`, `slider`. Everything is exercised through
//! public APIs: `layout_and_paint` at 1.0 and 1.75 scale,
//! `BuiltFrame::dispatch_event`, deterministic `Animator::begin_frame`, dark
//! and light `Theme`.

use std::time::{Duration, Instant};

use triton_engine::anim::Animator;
use triton_engine::gpu::{DrawBatch, DrawList, QuadInstance};
use triton_engine::image::ImageStore;
use triton_engine::style::{Rgba, Theme};
use triton_engine::text::TextEngine;
use triton_engine::ui::{
    BuiltFrame, Element, InputEvent, InteractionState, NodeKey, layout_and_paint,
};
use triton_kit::{
    CheckState, SliderMode, SliderOptions, SliderSurface, checkbox, radio, slider, switch,
};

const EPS: f32 = 1e-3;

#[derive(Debug, Clone, PartialEq)]
enum Msg {
    Toggled,
    Selected,
    Changed(f32),
}

fn assert_rgba(actual: [f32; 4], expected: [f32; 4], label: &str) {
    for (i, (&a, &e)) in actual.iter().zip(expected.iter()).enumerate() {
        assert!(
            (a - e).abs() <= EPS,
            "{label}[{i}]: expected {e}, got {a} (full={actual:?})"
        );
    }
}

fn quads(dl: &DrawList) -> Vec<QuadInstance> {
    let mut out = Vec::new();
    for b in &dl.batches {
        if let DrawBatch::Quads { instances, .. } = b {
            out.extend(instances.iter().copied());
        }
    }
    out
}

fn glyph_count(dl: &DrawList) -> usize {
    dl.batches
        .iter()
        .map(|b| match b {
            DrawBatch::Glyphs { instances, .. } => instances.len(),
            _ => 0,
        })
        .sum()
}

/// First quad whose buffer-px size is within `tol` of `(w * scale, h * scale)`.
fn quad_sized(dl: &DrawList, w: f32, h: f32, scale: f32, tol: f32) -> Option<QuadInstance> {
    quads(dl)
        .into_iter()
        .find(|q| (q.dst[2] - w * scale).abs() <= tol && (q.dst[3] - h * scale).abs() <= tol)
}

fn paint<M>(
    root: Element<M>,
    size: (f32, f32),
    scale: f32,
    interaction: &InteractionState,
    text: &mut TextEngine,
    dl: &mut DrawList,
) -> BuiltFrame<M> {
    layout_and_paint(
        root,
        (Some(size.0), Some(size.1)),
        scale,
        interaction,
        text,
        &ImageStore::new(),
        dl,
    )
}

fn click(frame: &BuiltFrame<Msg>, interaction: &mut InteractionState, x: f32, y: f32) -> Vec<Msg> {
    frame.dispatch_event(&InputEvent::PointerDown { x, y, button: 272 }, interaction);
    frame
        .dispatch_event(&InputEvent::PointerUp { x, y, button: 272 }, interaction)
        .0
}

// ── switch ──────────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn switch_frame(
    checked: bool,
    disabled: bool,
    scale: f32,
    interaction: &InteractionState,
    anim: &mut Animator,
    text: &mut TextEngine,
    dl: &mut DrawList,
) -> BuiltFrame<Msg> {
    paint(
        switch(
            "sw",
            checked,
            disabled,
            &Theme::dark(),
            interaction,
            anim,
            || Msg::Toggled,
        ),
        (80.0, 60.0),
        scale,
        interaction,
        text,
        dl,
    )
}

#[test]
fn switch_off_paints_track_border_and_left_thumb() {
    let mut text = TextEngine::new();
    let mut anim = Animator::new();
    let interaction = InteractionState::default();
    anim.begin_frame(Instant::now(), false);

    for scale in [1.0_f32, 1.75] {
        let mut dl = DrawList::default();
        switch_frame(
            false,
            false,
            scale,
            &interaction,
            &mut anim,
            &mut text,
            &mut dl,
        );
        let theme = Theme::dark();
        let track = quad_sized(&dl, 52.0, 32.0, scale, 0.01).expect("52x32 track");
        assert_eq!(
            track.fill,
            Rgba::from_color(theme.scheme.surface_container_highest).to_array(),
            "off track fill"
        );
        assert_eq!(
            track.border,
            Rgba::from_color(theme.scheme.outline).to_array(),
            "off track border"
        );
        assert!(
            (track.misc[2] - 2.0 * scale).abs() < EPS,
            "2px border @ {scale}x"
        );
        let thumb = quad_sized(&dl, 16.0, 16.0, scale, 0.01).expect("16px thumb");
        assert_eq!(
            thumb.fill,
            Rgba::from_color(theme.scheme.outline).to_array(),
            "off thumb"
        );
        assert!((thumb.dst[0] - 8.0 * scale).abs() < EPS, "thumb rests left");
        assert_eq!(glyph_count(&dl), 0, "no check icon when off");
    }
}

#[test]
fn switch_click_toggles_and_thumb_springs_to_the_right() {
    let mut text = TextEngine::new();
    let mut interaction = InteractionState::default();
    let mut anim = Animator::new();
    let mut dl = DrawList::default();
    let t0 = Instant::now();

    anim.begin_frame(t0, false);
    let frame = switch_frame(
        false,
        false,
        1.0,
        &interaction,
        &mut anim,
        &mut text,
        &mut dl,
    );
    let msgs = click(&frame, &mut interaction, 26.0, 16.0);
    assert_eq!(msgs, vec![Msg::Toggled]);

    // App flips state; the next frames animate the thumb 16px -> 24px and
    // slide it from x=8 toward x=24 while the track lerps to primary.
    anim.begin_frame(t0 + Duration::from_millis(16), false);
    switch_frame(
        true,
        false,
        1.0,
        &interaction,
        &mut anim,
        &mut text,
        &mut dl,
    );
    let theme = Theme::dark();
    let mid = quad_sized(&dl, 52.0, 32.0, 1.0, 0.01).unwrap();
    assert_ne!(
        mid.fill,
        Rgba::from_color(theme.scheme.primary).to_array(),
        "track still lerping"
    );

    anim.begin_frame(t0 + Duration::from_millis(60), false);
    switch_frame(
        true,
        false,
        1.0,
        &interaction,
        &mut anim,
        &mut text,
        &mut dl,
    );
    let in_flight: Vec<QuadInstance> = quads(&dl)
        .into_iter()
        .filter(|q| q.dst[2] > 16.0 && q.dst[2] < 24.0 && (q.dst[2] - q.dst[3]).abs() < EPS)
        .collect();
    assert!(!in_flight.is_empty(), "thumb diameter mid-morph");

    anim.begin_frame(t0 + Duration::from_secs(3), false);
    switch_frame(
        true,
        false,
        1.0,
        &interaction,
        &mut anim,
        &mut text,
        &mut dl,
    );
    let track = quad_sized(&dl, 52.0, 32.0, 1.0, 0.01).unwrap();
    assert_rgba(
        track.fill,
        Rgba::from_color(theme.scheme.primary).to_array(),
        "settled track",
    );
    let thumb = quad_sized(&dl, 24.0, 24.0, 1.0, 0.01).expect("24px thumb");
    assert_rgba(
        thumb.fill,
        Rgba::from_color(theme.scheme.on_primary).to_array(),
        "settled thumb",
    );
    assert!((thumb.dst[0] - 24.0).abs() < EPS, "thumb settled right");
    assert!(glyph_count(&dl) > 0, "check icon fades in when on");
}

#[test]
fn switch_disabled_suppresses_click_and_greys_out() {
    let mut text = TextEngine::new();
    let mut interaction = InteractionState::default();
    let mut anim = Animator::new();
    let mut dl = DrawList::default();
    anim.begin_frame(Instant::now(), false);

    let frame = switch_frame(true, true, 1.0, &interaction, &mut anim, &mut text, &mut dl);
    let msgs = click(&frame, &mut interaction, 26.0, 16.0);
    assert_eq!(msgs, Vec::<Msg>::new(), "disabled switch emits nothing");

    let theme = Theme::dark();
    let track = quad_sized(&dl, 52.0, 32.0, 1.0, 0.01).unwrap();
    let expected = Rgba::from_color(theme.scheme.on_surface).with_alpha(0.12);
    for (a, e) in track.fill.iter().zip(expected.to_array()) {
        assert!((a - e).abs() < 0.02, "disabled track ~ on_surface@.12");
    }
    // Never a state-layer halo while disabled, even when hovered.
    let _ = frame.dispatch_event(
        &InputEvent::PointerMove { x: 26.0, y: 16.0 },
        &mut interaction,
    );
    assert!(!interaction.is_hovered(NodeKey::named("sw")));
}

#[test]
fn switch_reduce_motion_fades_in_120ms() {
    let mut text = TextEngine::new();
    let interaction = InteractionState::default();
    let mut anim = Animator::new();
    let mut dl = DrawList::default();
    let t0 = Instant::now();

    anim.begin_frame(t0, true);
    switch_frame(
        false,
        false,
        1.0,
        &interaction,
        &mut anim,
        &mut text,
        &mut dl,
    );
    anim.begin_frame(t0 + Duration::from_millis(16), true);
    switch_frame(
        true,
        false,
        1.0,
        &interaction,
        &mut anim,
        &mut text,
        &mut dl,
    );
    // 120ms fade from the retarget completes by +200ms.
    anim.begin_frame(t0 + Duration::from_millis(200), true);
    switch_frame(
        true,
        false,
        1.0,
        &interaction,
        &mut anim,
        &mut text,
        &mut dl,
    );
    let thumb = quad_sized(&dl, 24.0, 24.0, 1.0, 0.01).expect("24px thumb");
    assert!(
        (thumb.dst[0] - 24.0).abs() < EPS,
        "reduce-motion thumb reaches rest, x={}",
        thumb.dst[0]
    );
}

#[test]
fn switch_hovers_thumb_state_layer() {
    let mut text = TextEngine::new();
    let mut interaction = InteractionState::default();
    let mut anim = Animator::new();
    let mut dl = DrawList::default();
    let t0 = Instant::now();

    anim.begin_frame(t0, false);
    let frame = switch_frame(
        false,
        false,
        1.0,
        &interaction,
        &mut anim,
        &mut text,
        &mut dl,
    );
    let _ = frame.dispatch_event(
        &InputEvent::PointerMove { x: 26.0, y: 16.0 },
        &mut interaction,
    );
    assert!(interaction.is_hovered(NodeKey::named("sw")));

    // Retarget registers at +50ms; the effects spring has faded in by +260ms.
    anim.begin_frame(t0 + Duration::from_millis(50), false);
    switch_frame(
        false,
        false,
        1.0,
        &interaction,
        &mut anim,
        &mut text,
        &mut dl,
    );
    anim.begin_frame(t0 + Duration::from_millis(260), false);
    switch_frame(
        false,
        false,
        1.0,
        &interaction,
        &mut anim,
        &mut text,
        &mut dl,
    );
    let halo = quad_sized(&dl, 40.0, 40.0, 1.0, 0.01).expect("40px state halo");
    assert!(
        halo.fill[3] > 0.05,
        "hover halo has alpha: {}",
        halo.fill[3]
    );
}

// ── checkbox ─────────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn checkbox_frame(
    state: CheckState,
    disabled: bool,
    scale: f32,
    interaction: &InteractionState,
    anim: &mut Animator,
    text: &mut TextEngine,
    dl: &mut DrawList,
) -> BuiltFrame<Msg> {
    paint(
        checkbox(
            "cb",
            state,
            disabled,
            &Theme::dark(),
            interaction,
            anim,
            || Msg::Toggled,
        ),
        (60.0, 60.0),
        scale,
        interaction,
        text,
        dl,
    )
}

#[test]
fn checkbox_unchecked_is_an_outlined_squircle_in_touch_target() {
    let mut text = TextEngine::new();
    let mut anim = Animator::new();
    let interaction = InteractionState::default();
    anim.begin_frame(Instant::now(), false);

    for scale in [1.0_f32, 1.75] {
        let mut dl = DrawList::default();
        let frame = checkbox_frame(
            CheckState::Unchecked,
            false,
            scale,
            &interaction,
            &mut anim,
            &mut text,
            &mut dl,
        );
        // 40px touch target owns the key.
        let hit = frame
            .hits
            .iter()
            .find(|h| h.key == Some(NodeKey::named("cb")))
            .expect("hit entry");
        assert!((hit.rect.w - 40.0).abs() < EPS && (hit.rect.h - 40.0).abs() < EPS);
        let theme = Theme::dark();
        let bx = quad_sized(&dl, 20.0, 20.0, scale, 0.01).expect("20px box");
        assert_eq!(
            bx.border,
            Rgba::from_color(theme.scheme.outline).to_array(),
            "off border outline"
        );
        assert!(
            (bx.misc[2] - 2.0 * scale).abs() < EPS,
            "2px border @ {scale}x"
        );
        assert!(bx.fill[3] < EPS, "off fill transparent");
        // Smooth squircle: non-zero radius with CORNER_SMOOTHING.
        assert!(bx.radii[0] > 0.0 && bx.misc[1] > 0.4);
        assert_eq!(glyph_count(&dl), 0, "no mark when unchecked");
    }
}

#[test]
fn checkbox_check_springs_fill_and_mark() {
    let mut text = TextEngine::new();
    let mut interaction = InteractionState::default();
    let mut anim = Animator::new();
    let mut dl = DrawList::default();
    let theme = Theme::dark();
    let t0 = Instant::now();

    anim.begin_frame(t0, false);
    let frame = checkbox_frame(
        CheckState::Unchecked,
        false,
        1.0,
        &interaction,
        &mut anim,
        &mut text,
        &mut dl,
    );
    let msgs = click(&frame, &mut interaction, 20.0, 20.0);
    assert_eq!(msgs, vec![Msg::Toggled]);

    // Retarget registers at +16ms; settled by +3s.
    anim.begin_frame(t0 + Duration::from_millis(16), false);
    checkbox_frame(
        CheckState::Checked,
        false,
        1.0,
        &interaction,
        &mut anim,
        &mut text,
        &mut dl,
    );
    anim.begin_frame(t0 + Duration::from_secs(3), false);
    checkbox_frame(
        CheckState::Checked,
        false,
        1.0,
        &interaction,
        &mut anim,
        &mut text,
        &mut dl,
    );
    let bx = quad_sized(&dl, 20.0, 20.0, 1.0, 0.01).unwrap();
    assert_rgba(
        bx.fill,
        Rgba::from_color(theme.scheme.primary).to_array(),
        "checked fill",
    );
    assert!(glyph_count(&dl) > 0, "check icon visible");
}

#[test]
fn checkbox_indeterminate_shows_dash_on_primary() {
    let mut text = TextEngine::new();
    let interaction = InteractionState::default();
    let mut anim = Animator::new();
    let mut dl = DrawList::default();
    anim.begin_frame(Instant::now(), false);

    checkbox_frame(
        CheckState::Indeterminate,
        false,
        1.0,
        &interaction,
        &mut anim,
        &mut text,
        &mut dl,
    );
    let theme = Theme::dark();
    let bx = quad_sized(&dl, 20.0, 20.0, 1.0, 0.01).unwrap();
    assert_rgba(
        bx.fill,
        Rgba::from_color(theme.scheme.primary).to_array(),
        "indeterminate fill",
    );
    assert!(glyph_count(&dl) > 0, "dash mark visible");
}

#[test]
fn checkbox_disabled_suppresses_click() {
    let mut text = TextEngine::new();
    let mut interaction = InteractionState::default();
    let mut anim = Animator::new();
    let mut dl = DrawList::default();
    anim.begin_frame(Instant::now(), false);

    let frame = checkbox_frame(
        CheckState::Checked,
        true,
        1.0,
        &interaction,
        &mut anim,
        &mut text,
        &mut dl,
    );
    let msgs = click(&frame, &mut interaction, 20.0, 20.0);
    assert_eq!(msgs, Vec::<Msg>::new());
    let theme = Theme::dark();
    let bx = quad_sized(&dl, 20.0, 20.0, 1.0, 0.01).unwrap();
    let expected = Rgba::from_color(theme.scheme.on_surface).with_alpha(0.38);
    assert_rgba(bx.fill, expected.to_array(), "disabled checked fill");
}

#[test]
fn checkbox_state_layer_fades_in_on_hover() {
    let mut text = TextEngine::new();
    let mut interaction = InteractionState::default();
    let mut anim = Animator::new();
    let mut dl = DrawList::default();
    let t0 = Instant::now();

    anim.begin_frame(t0, false);
    let frame = checkbox_frame(
        CheckState::Unchecked,
        false,
        1.0,
        &interaction,
        &mut anim,
        &mut text,
        &mut dl,
    );
    let _ = frame.dispatch_event(
        &InputEvent::PointerMove { x: 20.0, y: 20.0 },
        &mut interaction,
    );

    anim.begin_frame(t0 + Duration::from_millis(16), false);
    checkbox_frame(
        CheckState::Unchecked,
        false,
        1.0,
        &interaction,
        &mut anim,
        &mut text,
        &mut dl,
    );
    anim.begin_frame(t0 + Duration::from_millis(260), false);
    checkbox_frame(
        CheckState::Unchecked,
        false,
        1.0,
        &interaction,
        &mut anim,
        &mut text,
        &mut dl,
    );
    // The 40px root is the state layer itself; hover gives it alpha.
    let halo = quad_sized(&dl, 40.0, 40.0, 1.0, 0.01).expect("state layer");
    assert!(halo.fill[3] > 0.05, "hover state layer: {}", halo.fill[3]);
    assert!(halo.radii[0] >= 20.0, "state layer is circular");
}

// ── radio ────────────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn radio_frame(
    selected: bool,
    disabled: bool,
    scale: f32,
    interaction: &InteractionState,
    anim: &mut Animator,
    text: &mut TextEngine,
    dl: &mut DrawList,
) -> BuiltFrame<Msg> {
    paint(
        radio(
            "rdo",
            selected,
            disabled,
            &Theme::dark(),
            interaction,
            anim,
            || Msg::Selected,
        ),
        (60.0, 60.0),
        scale,
        interaction,
        text,
        dl,
    )
}

#[test]
fn radio_unselected_is_an_outlined_ring_in_touch_target() {
    let mut text = TextEngine::new();
    let mut anim = Animator::new();
    let interaction = InteractionState::default();
    anim.begin_frame(Instant::now(), false);

    for scale in [1.0_f32, 1.75] {
        let mut dl = DrawList::default();
        let frame = radio_frame(
            false,
            false,
            scale,
            &interaction,
            &mut anim,
            &mut text,
            &mut dl,
        );
        let hit = frame
            .hits
            .iter()
            .find(|h| h.key == Some(NodeKey::named("rdo")))
            .expect("hit entry");
        assert!((hit.rect.w - 40.0).abs() < EPS && (hit.rect.h - 40.0).abs() < EPS);
        let theme = Theme::dark();
        let ring = quad_sized(&dl, 20.0, 20.0, scale, 0.01).expect("20px ring");
        assert_eq!(
            ring.border,
            Rgba::from_color(theme.scheme.outline).to_array(),
            "off ring outline"
        );
        assert!(
            (ring.misc[2] - 2.0 * scale).abs() < EPS,
            "2px ring @ {scale}x"
        );
        // True circle: half-size radii, zero smoothing.
        assert!((ring.radii[0] - 10.0 * scale).abs() < EPS);
        assert!(ring.misc[1].abs() < EPS, "ring smoothing 0");
        assert!(
            quad_sized(&dl, 10.0, 10.0, scale, 0.01).is_none(),
            "no dot when unselected"
        );
    }
}

#[test]
fn radio_select_springs_inner_dot() {
    let mut text = TextEngine::new();
    let mut interaction = InteractionState::default();
    let mut anim = Animator::new();
    let mut dl = DrawList::default();
    let theme = Theme::dark();
    let t0 = Instant::now();

    anim.begin_frame(t0, false);
    let frame = radio_frame(
        false,
        false,
        1.0,
        &interaction,
        &mut anim,
        &mut text,
        &mut dl,
    );
    let msgs = click(&frame, &mut interaction, 20.0, 20.0);
    assert_eq!(msgs, vec![Msg::Selected]);

    anim.begin_frame(t0 + Duration::from_millis(16), false);
    radio_frame(
        true,
        false,
        1.0,
        &interaction,
        &mut anim,
        &mut text,
        &mut dl,
    );
    anim.begin_frame(t0 + Duration::from_millis(60), false);
    radio_frame(
        true,
        false,
        1.0,
        &interaction,
        &mut anim,
        &mut text,
        &mut dl,
    );
    let growing: Vec<QuadInstance> = quads(&dl)
        .into_iter()
        .filter(|q| q.dst[2] > 0.5 && q.dst[2] < 10.0 && (q.dst[2] - q.dst[3]).abs() < EPS)
        .collect();
    assert!(!growing.is_empty(), "dot scales up mid-flight");

    anim.begin_frame(t0 + Duration::from_secs(3), false);
    radio_frame(
        true,
        false,
        1.0,
        &interaction,
        &mut anim,
        &mut text,
        &mut dl,
    );
    let dot = quad_sized(&dl, 10.0, 10.0, 1.0, 0.01).expect("10px dot");
    assert_rgba(
        dot.fill,
        Rgba::from_color(theme.scheme.primary).to_array(),
        "dot primary",
    );
    let ring = quad_sized(&dl, 20.0, 20.0, 1.0, 0.01).unwrap();
    assert_eq!(
        ring.border,
        Rgba::from_color(theme.scheme.primary).to_array(),
        "selected ring primary"
    );
}

#[test]
fn radio_disabled_suppresses_click() {
    let mut text = TextEngine::new();
    let mut interaction = InteractionState::default();
    let mut anim = Animator::new();
    let mut dl = DrawList::default();
    anim.begin_frame(Instant::now(), false);

    let frame = radio_frame(true, true, 1.0, &interaction, &mut anim, &mut text, &mut dl);
    assert_eq!(
        click(&frame, &mut interaction, 20.0, 20.0),
        Vec::<Msg>::new()
    );
    let theme = Theme::dark();
    let dot = quad_sized(&dl, 10.0, 10.0, 1.0, 0.01).expect("disabled dot");
    let expected = Rgba::from_color(theme.scheme.on_surface).with_alpha(0.38);
    assert_rgba(dot.fill, expected.to_array(), "disabled dot");
}

// ── slider ───────────────────────────────────────────────────────────────────

const SLIDER_W: f32 = 200.0;

#[allow(clippy::too_many_arguments)]
fn slider_frame(
    value: f32,
    options: SliderOptions,
    theme: &Theme,
    scale: f32,
    interaction: &InteractionState,
    anim: &mut Animator,
    text: &mut TextEngine,
    dl: &mut DrawList,
) -> BuiltFrame<Msg> {
    paint(
        slider(
            "sld",
            value,
            SLIDER_W,
            options,
            theme,
            interaction,
            anim,
            Msg::Changed,
        ),
        (SLIDER_W, 60.0),
        scale,
        interaction,
        text,
        dl,
    )
}

fn drag_from(
    frame: &BuiltFrame<Msg>,
    interaction: &mut InteractionState,
    from: (f32, f32),
    to: (f32, f32),
) -> Vec<Msg> {
    let mut msgs = Vec::new();
    msgs.extend(
        frame
            .dispatch_event(
                &InputEvent::PointerDown {
                    x: from.0,
                    y: from.1,
                    button: 272,
                },
                interaction,
            )
            .0,
    );
    msgs.extend(
        frame
            .dispatch_event(&InputEvent::PointerMove { x: to.0, y: to.1 }, interaction)
            .0,
    );
    msgs
}

#[test]
fn slider_continuous_click_to_seek_and_drag_capture() {
    let mut text = TextEngine::new();
    let mut interaction = InteractionState::default();
    let mut anim = Animator::new();
    let mut dl = DrawList::default();
    let theme = Theme::dark();
    let t0 = Instant::now();

    anim.begin_frame(t0, false);
    let frame = slider_frame(
        0.3,
        SliderOptions::default(),
        &theme,
        1.0,
        &interaction,
        &mut anim,
        &mut text,
        &mut dl,
    );

    // Click-to-seek: PointerDown emits a drag message at the exact fraction.
    let (down, _) = frame.dispatch_event(
        &InputEvent::PointerDown {
            x: 0.7 * SLIDER_W,
            y: 22.0,
            button: 272,
        },
        &mut interaction,
    );
    assert_eq!(down, vec![Msg::Changed(0.7)]);

    // Drag keeps tracking way outside the horizontal bounds, clamped.
    let (mv, _) = frame.dispatch_event(
        &InputEvent::PointerMove {
            x: SLIDER_W + 80.0,
            y: 200.0,
        },
        &mut interaction,
    );
    assert_eq!(mv, vec![Msg::Changed(1.0)]);
    assert!(interaction.is_dragging(NodeKey::named("sld")));

    // Geometry: handle center at 70%, 6px gap, split-track radii.
    anim.begin_frame(t0 + Duration::from_millis(16), false);
    let frame = slider_frame(
        0.7,
        SliderOptions::default(),
        &theme,
        1.0,
        &interaction,
        &mut anim,
        &mut text,
        &mut dl,
    );
    let handle = quad_sized(&dl, 4.0, 44.0, 1.0, 0.01).expect("4x44 handle");
    // While dragging the handle tracks the value 1:1 (no spring lag).
    assert!(
        (handle.dst[0] - (0.7 * SLIDER_W - 2.0)).abs() < EPS,
        "handle center at 70%: {}",
        handle.dst[0]
    );
    assert!(
        handle.border.iter().all(|c| c.abs() < EPS),
        "handle has no border"
    );

    let qs = quads(&dl);
    let active = qs
        .iter()
        .find(|q| (q.dst[3] - 16.0).abs() < EPS && q.dst[0].abs() < EPS)
        .expect("active track at x=0");
    assert!((active.dst[2] - (0.7 * SLIDER_W - 6.0)).abs() < EPS);
    assert_eq!(
        active.fill,
        Rgba::from_color(theme.scheme.primary).to_array()
    );
    for (i, &e) in [8.0, 2.0, 2.0, 8.0].iter().enumerate() {
        assert!((active.radii[i] - e).abs() < EPS, "active radii[{i}] = {e}");
    }
    let inactive = qs
        .iter()
        .find(|q| (q.dst[3] - 16.0).abs() < EPS && q.dst[0] > 0.0)
        .expect("inactive track");
    assert!(
        (inactive.dst[0] - (0.7 * SLIDER_W + 6.0)).abs() < EPS,
        "6px gap after handle center: {}",
        inactive.dst[0]
    );
    assert!((inactive.dst[2] - (SLIDER_W - 0.7 * SLIDER_W - 6.0)).abs() < EPS);
    for (i, &e) in [2.0, 8.0, 8.0, 2.0].iter().enumerate() {
        assert!(
            (inactive.radii[i] - e).abs() < EPS,
            "inactive radii[{i}] = {e}"
        );
    }
    let _ = frame;
}

#[test]
fn slider_stepped_snaps_and_draws_stop_dots() {
    let mut text = TextEngine::new();
    let mut interaction = InteractionState::default();
    let mut anim = Animator::new();
    let mut dl = DrawList::default();
    let theme = Theme::dark();
    anim.begin_frame(Instant::now(), false);

    let opts = SliderOptions {
        mode: SliderMode::Stepped { steps: 4 },
        ..SliderOptions::default()
    };
    let frame = slider_frame(
        0.5,
        opts,
        &theme,
        1.0,
        &interaction,
        &mut anim,
        &mut text,
        &mut dl,
    );

    // 5 stop dots at 4x4: on_primary on the active side, primary on inactive.
    let dots: Vec<QuadInstance> = quads(&dl)
        .into_iter()
        .filter(|q| (q.dst[2] - 4.0).abs() < EPS && (q.dst[3] - 4.0).abs() < EPS)
        .collect();
    assert_eq!(dots.len(), 5, "steps+1 stop dots");
    let on_primary = Rgba::from_color(theme.scheme.on_primary).to_array();
    let primary = Rgba::from_color(theme.scheme.primary).to_array();
    for d in &dots {
        let frac = (d.dst[0] + 2.0) / SLIDER_W;
        if frac <= 0.5 + EPS {
            assert_eq!(d.fill, on_primary, "dot at {frac} on active side");
        } else {
            assert_eq!(d.fill, primary, "dot at {frac} on inactive side");
        }
    }

    // Snap on click-to-seek and on drag.
    let msgs = drag_from(
        &frame,
        &mut interaction,
        (0.62 * SLIDER_W, 22.0),
        (0.66 * SLIDER_W, 40.0),
    );
    assert_eq!(msgs[0], Msg::Changed(0.5), "0.62 snaps to .5");
    assert_eq!(msgs[1], Msg::Changed(0.75), "0.66 snaps to .75");
}

#[test]
fn slider_on_glass_track_contrast_differs_from_standard() {
    let mut text = TextEngine::new();
    let interaction = InteractionState::default();
    let mut anim = Animator::new();
    let theme = Theme::dark();
    anim.begin_frame(Instant::now(), false);

    for (surface, expected) in [
        (
            SliderSurface::Standard,
            Rgba::from_color(theme.scheme.surface_container_highest).to_array(),
        ),
        (SliderSurface::OnGlass, [1.0, 1.0, 1.0, 0.20]),
    ] {
        let mut dl = DrawList::default();
        slider_frame(
            0.4,
            SliderOptions {
                surface,
                ..SliderOptions::default()
            },
            &theme,
            1.0,
            &interaction,
            &mut anim,
            &mut text,
            &mut dl,
        );
        let inactive = quads(&dl)
            .into_iter()
            .find(|q| (q.dst[3] - 16.0).abs() < EPS && q.dst[0] > 0.0)
            .expect("inactive track");
        assert_rgba(inactive.fill, expected, "{surface:?} inactive track");
    }
}

#[test]
fn slider_disabled_suppresses_drag_and_greys_out() {
    let mut text = TextEngine::new();
    let mut interaction = InteractionState::default();
    let mut anim = Animator::new();
    let mut dl = DrawList::default();
    let theme = Theme::dark();
    anim.begin_frame(Instant::now(), false);

    let frame = slider_frame(
        0.5,
        SliderOptions {
            disabled: true,
            ..SliderOptions::default()
        },
        &theme,
        1.0,
        &interaction,
        &mut anim,
        &mut text,
        &mut dl,
    );
    let msgs = drag_from(&frame, &mut interaction, (60.0, 22.0), (180.0, 22.0));
    assert_eq!(msgs, Vec::<Msg>::new(), "disabled slider emits nothing");
    let on_surface = Rgba::from_color(theme.scheme.on_surface);
    let handle = quad_sized(&dl, 4.0, 44.0, 1.0, 0.01).expect("handle");
    assert_rgba(
        handle.fill,
        on_surface.with_alpha(0.38).to_array(),
        "disabled handle",
    );
    let inactive = quads(&dl)
        .into_iter()
        .find(|q| (q.dst[3] - 16.0).abs() < EPS && q.dst[0] > 0.0)
        .expect("inactive track");
    assert_rgba(
        inactive.fill,
        on_surface.with_alpha(0.12).to_array(),
        "disabled inactive track",
    );
}

#[test]
fn slider_handle_springs_on_programmatic_change_and_tracks_light_theme() {
    let mut text = TextEngine::new();
    let interaction = InteractionState::default();
    let mut anim = Animator::new();
    let mut dl = DrawList::default();
    let theme = Theme::light();
    let t0 = Instant::now();

    anim.begin_frame(t0, false);
    slider_frame(
        0.3,
        SliderOptions::default(),
        &theme,
        1.75,
        &interaction,
        &mut anim,
        &mut text,
        &mut dl,
    );
    let at_rest = quad_sized(&dl, 4.0, 44.0, 1.75, 0.02).expect("handle");
    assert!(
        (at_rest.dst[0] - (0.3 * SLIDER_W - 2.0) * 1.75).abs() < 0.05,
        "rests at 30% @1.75x"
    );

    // Programmatic change: retarget registers at +16ms, mid-flight at +120ms.
    anim.begin_frame(t0 + Duration::from_millis(16), false);
    slider_frame(
        0.8,
        SliderOptions::default(),
        &theme,
        1.75,
        &interaction,
        &mut anim,
        &mut text,
        &mut dl,
    );
    anim.begin_frame(t0 + Duration::from_millis(120), false);
    slider_frame(
        0.8,
        SliderOptions::default(),
        &theme,
        1.75,
        &interaction,
        &mut anim,
        &mut text,
        &mut dl,
    );
    let mid = quad_sized(&dl, 4.0, 44.0, 1.75, 0.02).expect("handle");
    let lo = (0.3 * SLIDER_W - 2.0) * 1.75;
    let hi = (0.8 * SLIDER_W - 2.0) * 1.75;
    assert!(
        mid.dst[0] > lo && mid.dst[0] < hi,
        "handle mid-flight: {} in {}..{}",
        mid.dst[0],
        lo,
        hi
    );
}
