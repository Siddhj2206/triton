//! Headless `DrawList` integration tests for triton-kit Phase 1A.5 chrome,
//! identity, and progress components. Everything is exercised through public
//! APIs: `layout_and_paint` at 1.0 and 1.75 scale, `BuiltFrame::dispatch_event`,
//! deterministic `Animator::begin_frame`, dark and light `Theme`.

use std::time::{Duration, Instant};

use triton_engine::anim::Animator;
use triton_engine::gpu::{
    DrawBatch, DrawList, IMAGE_CLIP_WAVE, KIND_CAPSULE, KIND_FILL, KIND_WAVE, QuadInstance,
};
use triton_engine::image::{ImageData, ImageStore};
use triton_engine::style::{Rgba, Theme};
use triton_engine::text::TextEngine;
use triton_engine::ui::{
    BuiltFrame, Element, InputEvent, InteractionState, NodeKey, box_, col, layout_and_paint,
};
use triton_kit::{
    Badge, KeycapStyle, MeterMode, PillStatus, Progress, badge, badged, hint_bar, icon_disc,
    icon_disc_styled, keycap, level_meter, overflow_pill, pill, status_pill, wave_avatar,
    wave_avatar_image, wavy_progress, wavy_ring,
};

const EPS: f32 = 1e-3;

#[derive(Debug, Clone, PartialEq)]
enum Msg {
    Select,
    Open,
}

fn fill_is(q: &QuadInstance, color: [f32; 4]) -> bool {
    q.fill
        .iter()
        .zip(color.iter())
        .all(|(a, b)| (a - b).abs() <= EPS)
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

/// Pill background quad is the only `KIND_FILL` quad — the child text emits glyphs.
fn fill_quad(dl: &DrawList) -> QuadInstance {
    quads(dl)
        .into_iter()
        .find(|q| q.misc[0] == KIND_FILL)
        .expect("fill quad")
}

// ── pill (workspace dot-to-pill morph) ───────────────────────────────────────

#[test]
fn pill_spring_morphs_40_to_56_and_dispatches_click() {
    let mut text = TextEngine::new();
    let mut dl = DrawList::default();
    let mut anim = Animator::new();
    let mut interaction = InteractionState::default();
    let theme = Theme::dark();
    let t0 = Instant::now();

    for scale in [1.0_f32, 1.75] {
        // Idle pill: 40px wide, full-pill radius, veil fill, onv text.
        anim.begin_frame(t0, false);
        let frame = paint(
            pill("ws", "1", false, &theme, &mut anim, || Msg::Select),
            (200.0, 80.0),
            scale,
            &interaction,
            &mut text,
            &mut dl,
        );
        let q = fill_quad(&dl);
        assert!((q.dst[2] - 40.0 * scale).abs() < EPS, "idle width {scale}x");
        assert!(
            (q.radii[0] - 20.0 * scale).abs() < EPS,
            "idle radius {scale}x"
        );
        assert!(
            fill_is(&q, theme.veil().to_array()),
            "idle veil fill: {:?}",
            q.fill
        );

        // Click dispatches the message.
        let cx = 40.0 * 0.5 + 0.0;
        frame.dispatch_event(
            &InputEvent::PointerDown {
                x: cx,
                y: 20.0,
                button: 272,
            },
            &mut interaction,
        );
        let (msgs, _) = frame.dispatch_event(
            &InputEvent::PointerUp {
                x: cx,
                y: 20.0,
                button: 272,
            },
            &mut interaction,
        );
        assert_eq!(msgs, vec![Msg::Select]);

        // Activate → width springs 40 → 56, radius 20 → 12, fill → primary.
        let t_act = t0 + Duration::from_millis(100);
        anim.begin_frame(t_act, false);
        paint(
            pill("ws", "1", true, &theme, &mut anim, || Msg::Select),
            (200.0, 80.0),
            scale,
            &interaction,
            &mut text,
            &mut dl,
        );
        let w0 = fill_quad(&dl).dst[2];
        assert!(
            (w0 - 40.0 * scale).abs() < EPS,
            "retarget frame still at start: {w0}"
        );

        anim.begin_frame(t_act + Duration::from_millis(80), false);
        paint(
            pill("ws", "1", true, &theme, &mut anim, || Msg::Select),
            (200.0, 80.0),
            scale,
            &interaction,
            &mut text,
            &mut dl,
        );
        let w_mid = fill_quad(&dl).dst[2];
        assert!(
            w_mid > 40.0 * scale && w_mid < 56.0 * scale,
            "mid-morph width {w_mid} at {scale}x"
        );

        anim.begin_frame(t_act + Duration::from_secs(2), false);
        paint(
            pill("ws", "1", true, &theme, &mut anim, || Msg::Select),
            (200.0, 80.0),
            scale,
            &interaction,
            &mut text,
            &mut dl,
        );
        let q = fill_quad(&dl);
        assert!((q.dst[2] - 56.0 * scale).abs() < 0.5, "settled 56px");
        assert!(
            (q.radii[0] - 12.0 * scale).abs() < 0.5,
            "settled r12 squircle"
        );
        assert!(
            fill_is(&q, Rgba::from_color(theme.scheme.primary).to_array()),
            "active primary fill: {:?}",
            q.fill
        );
        assert!(q.misc[1] > 0.3, "settled smoothing > 0");
    }
}

#[test]
fn pill_reduce_motion_fades_to_active() {
    let mut text = TextEngine::new();
    let mut dl = DrawList::default();
    let mut anim = Animator::new();
    let theme = Theme::dark();
    let t0 = Instant::now();

    anim.begin_frame(t0, true);
    paint(
        pill::<Msg>("ws", "1", false, &theme, &mut anim, || Msg::Select),
        (200.0, 80.0),
        1.0,
        &InteractionState::default(),
        &mut text,
        &mut dl,
    );
    assert!((fill_quad(&dl).dst[2] - 40.0).abs() < EPS);

    anim.begin_frame(t0 + Duration::from_millis(100), true);
    paint(
        pill::<Msg>("ws", "1", true, &theme, &mut anim, || Msg::Select),
        (200.0, 80.0),
        1.0,
        &InteractionState::default(),
        &mut text,
        &mut dl,
    );
    // 120ms fade: after another 200ms the morph is complete.
    anim.begin_frame(t0 + Duration::from_millis(400), true);
    paint(
        pill::<Msg>("ws", "1", true, &theme, &mut anim, || Msg::Select),
        (200.0, 80.0),
        1.0,
        &InteractionState::default(),
        &mut text,
        &mut dl,
    );
    assert!(
        (fill_quad(&dl).dst[2] - 56.0).abs() < EPS,
        "reduce_motion fade reaches 56px"
    );
}

// ── status_pill ──────────────────────────────────────────────────────────────

#[test]
fn status_pill_paints_h44_r22_per_status() {
    let mut text = TextEngine::new();
    let theme = Theme::dark();

    let expect = [
        (PillStatus::Neutral, theme.veil().to_array()),
        (
            PillStatus::Primary,
            Rgba::from_color(theme.scheme.primary_container).to_array(),
        ),
        (
            PillStatus::Attention,
            Rgba::from_color(theme.scheme.tertiary_container).to_array(),
        ),
        (
            PillStatus::Urgent,
            Rgba::from_color(theme.scheme.error_container).to_array(),
        ),
    ];

    for scale in [1.0_f32, 1.75] {
        let mut anim = Animator::new();
        let mut dl = DrawList::default();
        anim.begin_frame(Instant::now(), false);
        paint::<Msg>(
            col()
                .gap(4.0)
                .children(expect.iter().enumerate().map(|(i, (status, _))| {
                    status_pill(
                        NodeKey::from_pair("sp", i),
                        *status,
                        &theme,
                        &mut anim,
                        None,
                        [box_().w(60.0).h(20.0)],
                    )
                })),
            (300.0, 300.0),
            scale,
            &InteractionState::default(),
            &mut text,
            &mut dl,
        );
        let qs = quads(&dl);
        assert_eq!(qs.len(), 4, "one fill quad per status pill");
        for (i, (status, color)) in expect.iter().enumerate() {
            let q = qs[i];
            assert_eq!(q.fill, *color, "{status:?} fill");
            assert!(
                (q.dst[3] - 44.0 * scale).abs() < EPS,
                "{status:?} h44 at {scale}x"
            );
            assert!(
                (q.radii[0] - 22.0 * scale).abs() < EPS,
                "{status:?} r22 at {scale}x"
            );
            assert!(q.misc[1].abs() < EPS, "{status:?} pill smoothing 0");
        }
    }
}

#[test]
fn status_pill_click_dispatches() {
    let mut text = TextEngine::new();
    let mut dl = DrawList::default();
    let mut anim = Animator::new();
    let mut interaction = InteractionState::default();
    let theme = Theme::dark();
    anim.begin_frame(Instant::now(), false);

    let frame = paint(
        status_pill(
            "stat",
            PillStatus::Urgent,
            &theme,
            &mut anim,
            Some(Box::new(|| Msg::Open)),
            [box_().w(80.0).h(20.0)],
        ),
        (200.0, 60.0),
        1.0,
        &interaction,
        &mut text,
        &mut dl,
    );
    frame.dispatch_event(
        &InputEvent::PointerDown {
            x: 10.0,
            y: 22.0,
            button: 272,
        },
        &mut interaction,
    );
    let (msgs, _) = frame.dispatch_event(
        &InputEvent::PointerUp {
            x: 10.0,
            y: 22.0,
            button: 272,
        },
        &mut interaction,
    );
    assert_eq!(msgs, vec![Msg::Open]);
}

// ── overflow_pill ("N more") ─────────────────────────────────────────────────

#[test]
fn overflow_pill_shows_icon_count_and_clicks() {
    let mut text = TextEngine::new();
    let mut dl = DrawList::default();
    let mut interaction = InteractionState::default();
    let theme = Theme::dark();

    let frame = paint(
        overflow_pill("more", 7, &theme, || Msg::Open),
        (200.0, 60.0),
        1.0,
        &interaction,
        &mut text,
        &mut dl,
    );
    // Icon glyph + "7 more" text glyphs.
    assert!(glyph_count(&dl) >= 6, "icon + '7 more' glyphs");
    let q = fill_quad(&dl);
    assert_eq!(q.fill, theme.veil().to_array(), "veil overflow pill");
    assert!((q.dst[3] - 40.0).abs() < EPS, "h40");
    assert!((q.radii[0] - 20.0).abs() < EPS, "full pill radius");

    frame.dispatch_event(
        &InputEvent::PointerDown {
            x: 20.0,
            y: 20.0,
            button: 272,
        },
        &mut interaction,
    );
    let (msgs, _) = frame.dispatch_event(
        &InputEvent::PointerUp {
            x: 20.0,
            y: 20.0,
            button: 272,
        },
        &mut interaction,
    );
    assert_eq!(msgs, vec![Msg::Open]);
}

// ── icon_disc ────────────────────────────────────────────────────────────────

#[test]
fn icon_disc_is_smoothing_zero_circle() {
    let mut text = TextEngine::new();
    for theme in [Theme::dark(), Theme::light()] {
        for scale in [1.0_f32, 1.75] {
            let mut dl = DrawList::default();
            paint::<Msg>(
                icon_disc(triton_design::icon::Icon::Wifi, &theme),
                (100.0, 100.0),
                scale,
                &InteractionState::default(),
                &mut text,
                &mut dl,
            );
            let q = fill_quad(&dl);
            assert!((q.dst[2] - 40.0 * scale).abs() < EPS, "40px disc");
            assert!((q.dst[3] - 40.0 * scale).abs() < EPS);
            for r in q.radii {
                assert!((r - 20.0 * scale).abs() < EPS, "circular radius");
            }
            assert!(q.misc[1].abs() < EPS, "smoothing 0");
            assert_eq!(
                q.fill,
                Rgba::from_color(theme.scheme.secondary_container).to_array()
            );
            assert!(glyph_count(&dl) > 0, "icon glyph painted");
        }
    }
}

#[test]
fn icon_disc_styled_uses_custom_size_and_colors() {
    let mut text = TextEngine::new();
    let mut dl = DrawList::default();
    paint::<Msg>(
        icon_disc_styled(
            triton_design::icon::Icon::Wifi,
            48.0,
            Rgba::new(0.5, 0.1, 0.2, 1.0),
            Rgba::WHITE,
        ),
        (100.0, 100.0),
        1.0,
        &InteractionState::default(),
        &mut text,
        &mut dl,
    );
    let q = fill_quad(&dl);
    assert!((q.dst[2] - 48.0).abs() < EPS && (q.dst[3] - 48.0).abs() < EPS);
    assert!(q.radii.iter().all(|&r| (r - 24.0).abs() < EPS));
    assert_eq!(q.fill, [0.5, 0.1, 0.2, 1.0]);
}

// ── wave_avatar ──────────────────────────────────────────────────────────────

#[test]
fn wave_avatar_emits_wave_quad_not_circle() {
    let mut text = TextEngine::new();
    for scale in [1.0_f32, 1.75] {
        let mut dl = DrawList::default();
        let theme = Theme::dark();
        paint::<Msg>(
            wave_avatar("SJ", 48.0, &theme),
            (100.0, 100.0),
            scale,
            &InteractionState::default(),
            &mut text,
            &mut dl,
        );
        let wave = quads(&dl)
            .into_iter()
            .find(|q| q.misc[0] == KIND_WAVE)
            .expect("KIND_WAVE quad");
        // Wave SDF params: [phase, amp_ratio, lobes, _].
        assert!(wave.radii[0].abs() < EPS, "static phase 0");
        assert!(wave.radii[1] > 0.0, "amplitude ratio set");
        assert!((wave.radii[2] - 9.0).abs() < EPS, "9 lobes");
        assert_eq!(
            wave.fill,
            Rgba::from_color(theme.scheme.tertiary_container).to_array()
        );
        assert!((wave.dst[2] - 48.0 * scale).abs() < EPS, "48px avatar");
        assert!(glyph_count(&dl) >= 2, "initials glyphs");
    }
}

#[test]
fn wave_avatar_image_clips_to_wave() {
    let mut text = TextEngine::new();
    let mut images = ImageStore::new();
    let id =
        images.insert(ImageData::from_rgba(4, 4, vec![128u8; 4 * 4 * 4]).expect("valid image"));
    let mut dl = DrawList::default();
    let frame = layout_and_paint::<Msg>(
        wave_avatar_image(id, triton_engine::image::ImageStyle::default(), 48.0),
        (Some(100.0), Some(100.0)),
        1.0,
        &InteractionState::default(),
        &mut text,
        &images,
        &mut dl,
    );
    assert!(frame.content_size.0 >= 48.0);
    let img = dl
        .batches
        .iter()
        .find_map(|b| match b {
            DrawBatch::Images { instances, .. } => instances.first(),
            _ => None,
        })
        .expect("image batch");
    assert!(
        (img.misc[0] - IMAGE_CLIP_WAVE).abs() < EPS,
        "image clipped to wave"
    );
    assert!((img.radii[2] - 9.0).abs() < EPS, "9 lobes");
    assert!((img.dst[2] - 48.0).abs() < EPS, "48px dst");
}

// ── badge / badged ───────────────────────────────────────────────────────────

#[test]
fn badge_dot_is_10px_tertiary_with_2px_ring() {
    let mut text = TextEngine::new();
    let mut dl = DrawList::default();
    let theme = Theme::dark();
    let ring = Rgba::from_color(theme.scheme.surface);
    paint::<Msg>(
        badge(Badge::Dot(ring), &theme),
        (60.0, 60.0),
        1.75,
        &InteractionState::default(),
        &mut text,
        &mut dl,
    );
    let q = fill_quad(&dl);
    // 14px element = 10px dot + 2px ring on each side.
    assert!((q.dst[2] - 14.0 * 1.75).abs() < EPS, "14px incl ring");
    assert!(q.radii.iter().all(|&r| (r - 7.0 * 1.75).abs() < EPS));
    assert_eq!(q.fill, Rgba::from_color(theme.scheme.tertiary).to_array());
    assert_eq!(q.border, ring.to_array(), "ring uses parent surface color");
    assert!((q.misc[2] - 2.0 * 1.75).abs() < EPS, "2px ring at 1.75x");
}

#[test]
fn badge_count_pill_min_width_and_clamp() {
    let mut text = TextEngine::new();
    let theme = Theme::dark();
    for (count, min_glyphs) in [(5u32, 1usize), (42, 2), (150, 3)] {
        let mut dl = DrawList::default();
        paint::<Msg>(
            badge(Badge::Count(count), &theme),
            (100.0, 40.0),
            1.0,
            &InteractionState::default(),
            &mut text,
            &mut dl,
        );
        let q = fill_quad(&dl);
        assert!(q.dst[2] >= 20.0 - EPS, "min-w 20: {}", q.dst[2]);
        assert!((q.dst[3] - 20.0).abs() < EPS, "h20");
        assert!(q.radii.iter().all(|&r| (r - 10.0).abs() < EPS), "r10");
        assert_eq!(q.fill, Rgba::from_color(theme.scheme.tertiary).to_array());
        assert!(
            glyph_count(&dl) >= min_glyphs,
            "count {count} renders digits"
        );
    }
    // Count 150 clamps to "99+".
    let mut dl = DrawList::default();
    paint::<Msg>(
        badge(Badge::Count(150), &theme),
        (100.0, 40.0),
        1.0,
        &InteractionState::default(),
        &mut text,
        &mut dl,
    );
    assert_eq!(glyph_count(&dl), 3, "'99+' is three glyphs");
}

#[test]
fn badged_overlays_top_right_corner() {
    let mut text = TextEngine::new();
    for scale in [1.0_f32, 1.75] {
        let mut dl = DrawList::default();
        let theme = Theme::dark();
        let host = box_().size(40.0, 40.0).bg(Rgba::WHITE);
        paint::<Msg>(
            badged(host, Badge::Dot(Rgba::BLACK), &theme),
            (200.0, 100.0),
            scale,
            &InteractionState::default(),
            &mut text,
            &mut dl,
        );
        let qs = quads(&dl);
        let dot = qs
            .iter()
            .find(|q| (q.misc[2] - 2.0 * scale).abs() < EPS)
            .expect("dot with 2px ring");
        // Overhangs the host's top-right corner.
        assert!(dot.dst[1] < 0.0, "dot extends above host top");
        assert!(
            dot.dst[0] + dot.dst[2] > 40.0 * scale,
            "dot extends past host right edge"
        );
    }
}

// ── keycap ───────────────────────────────────────────────────────────────────

#[test]
fn keycap_flat_is_24px_r7_bordered() {
    let mut text = TextEngine::new();
    for scale in [1.0_f32, 1.75] {
        let mut dl = DrawList::default();
        let theme = Theme::dark();
        paint::<Msg>(
            keycap("K", KeycapStyle::Flat, &theme),
            (80.0, 40.0),
            scale,
            &InteractionState::default(),
            &mut text,
            &mut dl,
        );
        let q = fill_quad(&dl);
        assert!(q.dst[2] >= 24.0 * scale - EPS, "min-w 24");
        assert!((q.dst[3] - 24.0 * scale).abs() < EPS, "h24");
        assert!(q.radii.iter().all(|&r| (r - 7.0 * scale).abs() < EPS), "r7");
        assert!((q.misc[2] - scale).abs() < EPS, "1px border");
        assert!(q.border[3] > 0.0, "border painted");
        assert!(glyph_count(&dl) >= 1, "key label");
    }
}

#[test]
fn keycap_raised_has_darker_bottom_edge() {
    let mut text = TextEngine::new();
    let mut dl = DrawList::default();
    let theme = Theme::dark();
    paint::<Msg>(
        keycap("Ctrl", KeycapStyle::Raised, &theme),
        (120.0, 60.0),
        1.0,
        &InteractionState::default(),
        &mut text,
        &mut dl,
    );
    let qs = quads(&dl);
    let body = qs
        .iter()
        .find(|q| {
            fill_is(
                q,
                Rgba::from_color(theme.scheme.surface_container_high).to_array(),
            )
        })
        .expect("surface_container_high body");
    assert!(body.dst[2] >= 30.0 - EPS, "min-w 30");
    assert!((body.dst[3] - 30.0).abs() < EPS, "h30");
    assert!(body.radii.iter().all(|&r| (r - 8.0).abs() < EPS), "r8");
    // 2px darker strip hugging the bottom edge.
    let strip = qs
        .iter()
        .find(|q| (q.dst[3] - 2.0).abs() < EPS)
        .expect("2px bottom strip");
    assert!(strip.fill[3] > 0.2, "darker strip visible");
    let strip_bottom = strip.dst[1] + strip.dst[3];
    let body_bottom = body.dst[1] + body.dst[3];
    assert!(
        (strip_bottom - body_bottom).abs() < EPS,
        "strip hugs bottom edge"
    );
}

// ── hint_bar ─────────────────────────────────────────────────────────────────

#[test]
fn hint_bar_composes_keycap_label_rows() {
    let mut text = TextEngine::new();
    let mut dl = DrawList::default();
    let theme = Theme::dark();
    paint::<Msg>(
        hint_bar([("Enter", "Open"), ("Esc", "Close")], &theme),
        (400.0, 40.0),
        1.0,
        &InteractionState::default(),
        &mut text,
        &mut dl,
    );
    let qs = quads(&dl);
    let keycaps: Vec<_> = qs
        .iter()
        .filter(|q| (q.misc[2] - 1.0).abs() < EPS && q.border[3] > 0.0)
        .collect();
    assert_eq!(keycaps.len(), 2, "two flat keycaps");
    assert!(glyph_count(&dl) >= 6, "key labels + action labels");
}

// ── wavy_progress ────────────────────────────────────────────────────────────

fn capsules(dl: &DrawList) -> Vec<QuadInstance> {
    quads(dl)
        .into_iter()
        .filter(|q| q.misc[0] == KIND_CAPSULE)
        .collect()
}

#[test]
fn wavy_progress_paints_flat_track_and_wavy_played() {
    let mut text = TextEngine::new();
    let mut anim = Animator::new();
    let theme = Theme::dark();
    let t0 = Instant::now();
    let highest = Rgba::from_color(theme.scheme.surface_container_highest).to_array();
    let primary = Rgba::from_color(theme.scheme.primary).to_array();

    for scale in [1.0_f32, 1.75] {
        let mut dl = DrawList::default();
        anim.begin_frame(t0, false);
        paint::<Msg>(
            wavy_progress(
                "dl",
                200.0,
                4.0,
                Progress::Determinate(0.5),
                &theme,
                &mut anim,
            ),
            (300.0, 40.0),
            scale,
            &InteractionState::default(),
            &mut text,
            &mut dl,
        );
        let caps = capsules(&dl);
        let track: Vec<_> = caps.iter().filter(|q| fill_is(q, highest)).collect();
        let played: Vec<_> = caps.iter().filter(|q| fill_is(q, primary)).collect();
        assert_eq!(track.len(), 1, "flat track is a single capsule");
        assert!(track[0].dst[2] >= 200.0 * scale, "track spans full width");
        assert!(
            played.len() > 20,
            "wavy played part tessellates: {}",
            played.len()
        );
        // Played part covers ~half the width.
        let max_x = played
            .iter()
            .map(|q| q.dst[0] + q.dst[2])
            .fold(0f32, f32::max);
        assert!(max_x < 160.0 * scale, "played ends near half: {max_x}");
        assert!(max_x > 80.0 * scale);
    }
}

#[test]
fn wavy_progress_complete_renders_flat_line() {
    let mut text = TextEngine::new();
    let mut anim = Animator::new();
    let theme = Theme::dark();
    anim.begin_frame(Instant::now(), false);
    let mut dl = DrawList::default();
    paint::<Msg>(
        wavy_progress(
            "dl",
            200.0,
            4.0,
            Progress::Determinate(1.0),
            &theme,
            &mut anim,
        ),
        (300.0, 40.0),
        1.0,
        &InteractionState::default(),
        &mut text,
        &mut dl,
    );
    let primary = Rgba::from_color(theme.scheme.primary).to_array();
    let played: Vec<_> = capsules(&dl)
        .into_iter()
        .filter(|q| fill_is(q, primary))
        .collect();
    assert_eq!(played.len(), 1, "complete = single flat capsule");
    assert!(played[0].dst[2] >= 200.0, "flat line spans width");
}

#[test]
fn wavy_progress_indeterminate_travels_and_freezes_under_reduce_motion() {
    let mut text = TextEngine::new();
    let mut anim = Animator::new();
    let theme = Theme::dark();
    let t0 = Instant::now();
    let primary = Rgba::from_color(theme.scheme.primary).to_array();

    let played_min_x = |dl: &DrawList| -> f32 {
        capsules(dl)
            .into_iter()
            .filter(|q| fill_is(q, primary))
            .map(|q| q.dst[0])
            .fold(f32::MAX, f32::min)
    };

    let mut xs = Vec::new();
    for ms in [0u64, 400, 800] {
        let mut dl = DrawList::default();
        anim.begin_frame(t0 + Duration::from_millis(ms), false);
        paint::<Msg>(
            wavy_progress(
                "wait",
                200.0,
                4.0,
                Progress::Indeterminate,
                &theme,
                &mut anim,
            ),
            (300.0, 40.0),
            1.0,
            &InteractionState::default(),
            &mut text,
            &mut dl,
        );
        xs.push(played_min_x(&dl));
    }
    assert!(
        (xs[0] - xs[1]).abs() > EPS || (xs[1] - xs[2]).abs() > EPS,
        "indeterminate window travels: {xs:?}"
    );

    // reduce_motion: position is static between frames.
    let mut rm_xs = Vec::new();
    for ms in [2000u64, 2400] {
        let mut dl = DrawList::default();
        anim.begin_frame(t0 + Duration::from_millis(ms), true);
        paint::<Msg>(
            wavy_progress(
                "wait",
                200.0,
                4.0,
                Progress::Indeterminate,
                &theme,
                &mut anim,
            ),
            (300.0, 40.0),
            1.0,
            &InteractionState::default(),
            &mut text,
            &mut dl,
        );
        rm_xs.push(played_min_x(&dl));
    }
    assert!(
        (rm_xs[0] - rm_xs[1]).abs() < EPS,
        "reduce_motion holds a static gentle indicator"
    );
}

// ── wavy_ring ────────────────────────────────────────────────────────────────

#[test]
fn wavy_ring_drains_clockwise_from_top() {
    let mut text = TextEngine::new();
    let mut anim = Animator::new();
    let theme = Theme::dark();
    let primary = Rgba::from_color(theme.scheme.primary).to_array();
    let t0 = Instant::now();

    for scale in [1.0_f32, 1.75] {
        let mut counts = Vec::new();
        for (i, progress) in [1.0f32, 0.5, 0.25].iter().enumerate() {
            let mut dl = DrawList::default();
            anim.begin_frame(t0 + Duration::from_millis(i as u64 * 100), false);
            paint::<Msg>(
                wavy_ring(96.0, 4.0, *progress, &theme, &mut anim),
                (120.0, 120.0),
                scale,
                &InteractionState::default(),
                &mut text,
                &mut dl,
            );
            let caps = capsules(&dl);
            assert!(
                caps.iter().any(|q| !fill_is(q, primary)),
                "subtle track ring painted"
            );
            counts.push(caps.iter().filter(|q| fill_is(q, primary)).count());
        }
        assert!(
            counts[0] > counts[1] && counts[1] > counts[2],
            "arc length shrinks as countdown drains: {counts:?} at {scale}x"
        );

        // Drain start sits at 12 o'clock: topmost played capsule near top center.
        let mut dl = DrawList::default();
        anim.begin_frame(t0, false);
        paint::<Msg>(
            wavy_ring(96.0, 4.0, 0.35, &theme, &mut anim),
            (120.0, 120.0),
            scale,
            &InteractionState::default(),
            &mut text,
            &mut dl,
        );
        let top = capsules(&dl)
            .into_iter()
            .filter(|q| fill_is(q, primary))
            .map(|q| q.dst[1])
            .fold(f32::MAX, f32::min);
        assert!(top < 30.0 * scale, "drain starts near 12 o'clock: {top}");
    }
}

// ── level_meter ──────────────────────────────────────────────────────────────

#[test]
fn level_meter_paints_three_bars_anchored_bottom() {
    let mut text = TextEngine::new();
    let mut anim = Animator::new();
    let theme = Theme::dark();
    anim.begin_frame(Instant::now(), false);
    let mut dl = DrawList::default();
    paint::<Msg>(
        level_meter(
            "eq",
            MeterMode::Levels([0.2, 0.6, 1.0]),
            (24.0, 16.0),
            &theme,
            &mut anim,
        ),
        (80.0, 40.0),
        1.0,
        &InteractionState::default(),
        &mut text,
        &mut dl,
    );
    let bars = capsules_or_fills(&dl);
    assert_eq!(bars.len(), 3, "three equalizer bars");
    let bottom = bars[0].dst[1] + bars[0].dst[3];
    for b in &bars {
        assert!((b.dst[1] + b.dst[3] - bottom).abs() < EPS, "bottom aligned");
    }
    // Ordered heights 0.2 < 0.6 < 1.0 (first-seen springs start at target).
    assert!(bars[0].dst[3] < bars[1].dst[3] && bars[1].dst[3] < bars[2].dst[3]);
    assert!(
        (bars[2].dst[3] - 16.0).abs() < EPS,
        "level 1.0 fills height"
    );
}

fn capsules_or_fills(dl: &DrawList) -> Vec<QuadInstance> {
    quads(dl)
        .into_iter()
        .filter(|q| q.misc[0] == KIND_FILL || q.misc[0] == KIND_CAPSULE)
        .collect()
}

#[test]
fn level_meter_playing_animates_and_freezes_under_reduce_motion() {
    let mut text = TextEngine::new();
    let mut anim = Animator::new();
    let theme = Theme::dark();
    let t0 = Instant::now();

    let heights =
        |dl: &DrawList| -> Vec<f32> { capsules_or_fills(dl).iter().map(|q| q.dst[3]).collect() };

    let mut frames = Vec::new();
    for ms in [0u64, 250, 500] {
        let mut dl = DrawList::default();
        anim.begin_frame(t0 + Duration::from_millis(ms), false);
        paint::<Msg>(
            level_meter("eq", MeterMode::Playing, (24.0, 16.0), &theme, &mut anim),
            (80.0, 40.0),
            1.0,
            &InteractionState::default(),
            &mut text,
            &mut dl,
        );
        frames.push(heights(&dl));
    }
    assert!(
        frames[0] != frames[1] || frames[1] != frames[2],
        "playing animates: {frames:?}"
    );

    // First reduced frame retargets the springs onto the static gentle state;
    // later frames are settled and identical.
    for ms in [2000u64, 2200, 2500] {
        let mut dl = DrawList::default();
        anim.begin_frame(t0 + Duration::from_millis(ms), true);
        paint::<Msg>(
            level_meter("eq", MeterMode::Playing, (24.0, 16.0), &theme, &mut anim),
            (80.0, 40.0),
            1.0,
            &InteractionState::default(),
            &mut text,
            &mut dl,
        );
    }
    let mut rm = Vec::new();
    for ms in [2500u64, 2750] {
        let mut dl = DrawList::default();
        anim.begin_frame(t0 + Duration::from_millis(ms), true);
        paint::<Msg>(
            level_meter("eq", MeterMode::Playing, (24.0, 16.0), &theme, &mut anim),
            (80.0, 40.0),
            1.0,
            &InteractionState::default(),
            &mut text,
            &mut dl,
        );
        rm.push(heights(&dl));
    }
    assert_eq!(rm[0], rm[1], "reduce_motion holds gentle static bars");
}

// ── clock-driven continuous frames ───────────────────────────────────────────

#[test]
fn clock_driven_indicators_drive_is_animating() {
    let mut text = TextEngine::new();
    let mut anim = Animator::new();
    let theme = Theme::dark();
    let interaction = InteractionState::default();
    let t0 = Instant::now();
    let mut dl = DrawList::default();

    // Active clock-driven indicators keep `is_animating` true so the host
    // repaints every frame — no app-level workaround needed.
    anim.begin_frame(t0, false);
    paint::<Msg>(
        wavy_progress("p", 200.0, 4.0, Progress::Indeterminate, &theme, &mut anim),
        (300.0, 40.0),
        1.0,
        &interaction,
        &mut text,
        &mut dl,
    );
    assert!(anim.is_animating(), "indeterminate progress animates");

    anim.begin_frame(t0 + Duration::from_millis(16), false);
    paint::<Msg>(
        wavy_ring(96.0, 4.0, 0.5, &theme, &mut anim),
        (120.0, 120.0),
        1.0,
        &interaction,
        &mut text,
        &mut dl,
    );
    assert!(anim.is_animating(), "draining ring animates");

    anim.begin_frame(t0 + Duration::from_millis(32), false);
    paint::<Msg>(
        level_meter("eq", MeterMode::Playing, (24.0, 16.0), &theme, &mut anim),
        (80.0, 40.0),
        1.0,
        &interaction,
        &mut text,
        &mut dl,
    );
    assert!(anim.is_animating(), "playing meter animates");

    // A completed ring requests nothing.
    anim.begin_frame(t0 + Duration::from_millis(48), false);
    paint::<Msg>(
        wavy_ring(96.0, 4.0, 0.0, &theme, &mut anim),
        (120.0, 120.0),
        1.0,
        &interaction,
        &mut text,
        &mut dl,
    );
    assert!(!anim.is_animating(), "empty ring is static");

    // Under reduce_motion the same indicators go static: run enough frames for
    // the meter's 120ms fade to settle, then nothing requests repaint.
    anim.begin_frame(t0 + Duration::from_millis(64), true);
    paint::<Msg>(
        wavy_progress("p", 200.0, 4.0, Progress::Indeterminate, &theme, &mut anim),
        (300.0, 40.0),
        1.0,
        &interaction,
        &mut text,
        &mut dl,
    );
    assert!(
        !anim.is_animating(),
        "reduce_motion freezes the indeterminate bar"
    );

    for ms in [80u64, 300, 500] {
        anim.begin_frame(t0 + Duration::from_millis(ms), true);
        paint::<Msg>(
            level_meter("eq", MeterMode::Playing, (24.0, 16.0), &theme, &mut anim),
            (80.0, 40.0),
            1.0,
            &interaction,
            &mut text,
            &mut dl,
        );
    }
    assert!(
        !anim.is_animating(),
        "reduce_motion settles the meter to static bars"
    );
}

#[test]
fn clock_uses_animator_epoch_so_prewarm_timestamps_are_deterministic() {
    // `elapsed_secs` anchors on the FIRST begin_frame; a frame timestamped
    // before that epoch saturates to 0 rather than panicking or drifting —
    // headless callers can rewind the clock freely.
    let mut text = TextEngine::new();
    let mut anim = Animator::new();
    let theme = Theme::dark();
    let interaction = InteractionState::default();
    let mut dl = DrawList::default();
    let epoch = Instant::now() + Duration::from_secs(60);

    anim.begin_frame(epoch, false);
    assert!(anim.elapsed_secs().abs() < EPS);
    anim.begin_frame(
        epoch
            .checked_sub(Duration::from_secs(30))
            .expect("epoch is +60s"),
        false,
    );
    assert!(
        anim.elapsed_secs().abs() < EPS,
        "pre-epoch timestamp saturates to 0: {}",
        anim.elapsed_secs()
    );
    // Painting at that timestamp still produces a deterministic frame.
    paint::<Msg>(
        wavy_progress("p", 200.0, 4.0, Progress::Indeterminate, &theme, &mut anim),
        (300.0, 40.0),
        1.0,
        &interaction,
        &mut text,
        &mut dl,
    );
    assert!(anim.is_animating());
}
