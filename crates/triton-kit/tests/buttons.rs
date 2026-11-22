//! Headless `DrawList` integration tests for triton-kit Phase 1A.2 button
//! family: `button`, `icon_button`, `chip`, `toggle_tile`, `segmented`, and
//! `power_menu`. Everything is exercised through public APIs:
//! `layout_and_paint` at 1.0 and 1.75 scale, `BuiltFrame::dispatch_event`,
//! deterministic `Animator::begin_frame`, dark and light `Theme`.

use std::time::{Duration, Instant};

use triton_design::color::STATE_LAYER;
use triton_design::icon::{Icon, IconStyle};
use triton_design::shape::{CORNER_SMOOTHING, RADII, SIZE};
use triton_engine::anim::Animator;
use triton_engine::gpu::{
    DrawBatch, DrawList, EdgeFade, KIND_FILL, KIND_SHADOW, QuadInstance, ScissorRect,
};
use triton_engine::image::ImageStore;
use triton_engine::style::{Rgba, Theme};
use triton_engine::text::TextEngine;
use triton_engine::ui::{
    BuiltFrame, Element, ElementContent, InputEvent, InteractionState, NodeKey, layout_and_paint,
};
use triton_kit::{
    Button, ButtonSize, ButtonTone, Chip, ChipKind, IconButton, PowerCellTone, PowerItem,
    SegmentedItem, SegmentedStyle, ToggleTile, button, chip, icon_button, power_menu, segmented,
    toggle_tile,
};

const EPS: f32 = 1e-3;

#[derive(Debug, Clone, PartialEq)]
enum Msg {
    Click(&'static str),
    Index(usize),
}

fn assert_rgba(actual: [f32; 4], expected: [f32; 4], label: &str) {
    for (i, (&a, &e)) in actual.iter().zip(expected.iter()).enumerate() {
        assert!(
            (a - e).abs() <= EPS,
            "{label}[{i}]: expected {e}, got {a} (full={actual:?})"
        );
    }
}

fn quads(dl: &DrawList) -> Vec<(Option<ScissorRect>, Option<EdgeFade>, QuadInstance)> {
    let mut out = Vec::new();
    for b in &dl.batches {
        if let DrawBatch::Quads {
            scissor,
            edge_fade,
            instances,
        } = b
        {
            out.extend(instances.iter().map(|&q| (*scissor, *edge_fade, q)));
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

fn hit_rect<M>(frame: &BuiltFrame<M>, key: &str) -> triton_engine::style::Rect {
    frame
        .hits
        .iter()
        .find(|h| h.key == Some(NodeKey::named(key)))
        .unwrap_or_else(|| panic!("no hit entry for {key}"))
        .rect
}

fn click<M>(frame: &BuiltFrame<M>, interaction: &mut InteractionState, x: f32, y: f32) -> Vec<M> {
    frame.dispatch_event(&InputEvent::PointerDown { x, y, button: 272 }, interaction);
    frame
        .dispatch_event(&InputEvent::PointerUp { x, y, button: 272 }, interaction)
        .0
}

fn text_font(el: &Element<Msg>) -> &triton_engine::style::FontSpec {
    match &el.content {
        ElementContent::Text { font, .. } => font,
        other => panic!("expected text content, got {other:?}"),
    }
}

// ── button: tones, sizes, click, disabled ────────────────────────────────────

fn plain_button(tone: ButtonTone, size: ButtonSize, theme: &Theme) -> Element<Msg> {
    button(
        theme,
        &InteractionState::default(),
        &mut Animator::new(),
        Button {
            key: "b".into(),
            label: "Go".into(),
            tone,
            size,
            on_click: Some(Box::new(|| Msg::Click("b"))),
            ..Default::default()
        },
    )
}

#[test]
fn button_sizes_match_control_tokens() {
    let mut text = TextEngine::new();
    let theme = Theme::dark();
    for (size, h) in [
        (ButtonSize::S, SIZE.control_s),
        (ButtonSize::M, SIZE.control_m),
        (ButtonSize::L, SIZE.control_l),
        (ButtonSize::Xl, SIZE.control_xl),
    ] {
        for scale in [1.0_f32, 1.75] {
            let mut dl = DrawList::default();
            let frame = paint(
                plain_button(ButtonTone::Filled, size, &theme),
                (200.0, 100.0),
                scale,
                &InteractionState::default(),
                &mut text,
                &mut dl,
            );
            let q = quads(&dl)[0].2;
            assert!(
                (q.dst[3] - h * scale).abs() < EPS,
                "{size:?} height at {scale}x: {}",
                q.dst[3]
            );
            // Rest radius = pill (h/2), corners are clamped to that.
            assert!(
                (q.radii[0] - h * 0.5 * scale).abs() < 0.5,
                "{size:?} rest radius: {}",
                q.radii[0]
            );
            assert!(frame.hits.iter().any(|e| e.key == Some("b".into())));
        }
    }
}

#[test]
fn button_tones_paint_expected_fills() {
    let mut text = TextEngine::new();
    for theme in [Theme::dark(), Theme::light()] {
        let s = &theme.scheme;
        let cases: [(ButtonTone, Option<Rgba>); 6] = [
            (ButtonTone::Filled, Some(Rgba::from_color(s.primary))),
            (
                ButtonTone::Tonal,
                Some(Rgba::from_color(s.secondary_container)),
            ),
            (ButtonTone::Outlined, None),
            (ButtonTone::Text, None),
            (ButtonTone::Veil, Some(theme.veil())),
            (
                ButtonTone::Destructive,
                Some(Rgba::from_color(s.error_container)),
            ),
        ];
        for (tone, fill) in cases {
            let mut dl = DrawList::default();
            paint(
                plain_button(tone, ButtonSize::M, &theme),
                (200.0, 100.0),
                1.0,
                &InteractionState::default(),
                &mut text,
                &mut dl,
            );
            let q = quads(&dl)[0].2;
            match fill {
                Some(c) => assert_rgba(q.fill, c.to_array(), &format!("{tone:?} fill")),
                None => assert!(
                    q.fill[3] < EPS,
                    "{tone:?} should have transparent fill, got {:?}",
                    q.fill
                ),
            }
            if tone == ButtonTone::Outlined {
                assert!((q.misc[2] - 1.0).abs() < EPS, "outlined 1px border");
                assert_rgba(
                    q.border,
                    Rgba::from_color(s.outline).to_array(),
                    "outlined border",
                );
            }
            if tone == ButtonTone::Destructive {
                let glyphs_ok = dl.batches.iter().any(|b| match b {
                    DrawBatch::Glyphs { instances, .. } => instances.iter().all(|g| {
                        g.color
                            .iter()
                            .zip(Rgba::from_color(s.on_error_container).to_array())
                            .all(|(a, e)| (a - e).abs() <= EPS)
                    }),
                    _ => false,
                });
                assert!(glyphs_ok, "destructive label uses on_error_container");
            }
        }
    }
}

#[test]
fn button_click_dispatches_and_disabled_suppresses() {
    let mut text = TextEngine::new();
    let mut dl = DrawList::default();
    let mut interaction = InteractionState::default();
    let theme = Theme::dark();

    let frame = paint(
        plain_button(ButtonTone::Filled, ButtonSize::M, &theme),
        (200.0, 100.0),
        1.0,
        &interaction,
        &mut text,
        &mut dl,
    );
    let r = hit_rect(&frame, "b");
    let (cx, cy) = (r.x + r.w * 0.5, r.y + r.h * 0.5);
    assert_eq!(
        click(&frame, &mut interaction, cx, cy),
        vec![Msg::Click("b")]
    );

    // Disabled: no handler → no press, no click; container on_surface@.12,
    // content on_surface@.38.
    let mut dl = DrawList::default();
    let frame = paint(
        button(
            &theme,
            &interaction,
            &mut Animator::new(),
            Button {
                key: "dis".into(),
                label: "Nope".into(),
                disabled: true,
                on_click: Some(Box::new(|| Msg::Click("dis"))),
                ..Default::default()
            },
        ),
        (200.0, 100.0),
        1.0,
        &interaction,
        &mut text,
        &mut dl,
    );
    // A disabled button is inert: it paints but owns no interactive hit entry.
    assert!(
        !frame.hits.iter().any(|h| h.key == Some("dis".into())),
        "disabled button should not be hit-testable"
    );
    let q0 = quads(&dl)[0].2;
    assert!(
        click(&frame, &mut interaction, q0.dst[0] + 2.0, q0.dst[1] + 2.0).is_empty(),
        "disabled button must not emit"
    );
    let q = quads(&dl)[0].2;
    assert_rgba(
        q.fill,
        Rgba::from_color(theme.scheme.on_surface)
            .with_alpha(STATE_LAYER.disabled_container)
            .to_array(),
        "disabled container",
    );
    let glyph_alpha = dl
        .batches
        .iter()
        .find_map(|b| match b {
            DrawBatch::Glyphs { instances, .. } => instances.first().map(|g| g.color[3]),
            _ => None,
        })
        .expect("label glyphs");
    assert!(
        (glyph_alpha - STATE_LAYER.disabled_content).abs() < EPS,
        "disabled content alpha: {glyph_alpha}"
    );
}

// ── button: press corner-tightening spring ───────────────────────────────────

fn rest_radius(dl: &DrawList) -> f32 {
    quads(dl)[0].2.radii[0]
}

fn held_button(interaction: &InteractionState, anim: &mut Animator) -> Element<Msg> {
    button(
        &Theme::dark(),
        interaction,
        anim,
        Button {
            key: "press".into(),
            label: "Hold".into(),
            on_click: Some(Box::new(|| Msg::Click("press"))),
            ..Default::default()
        },
    )
}

fn press_frame(
    interaction: &InteractionState,
    anim: &mut Animator,
    text: &mut TextEngine,
    dl: &mut DrawList,
) -> BuiltFrame<Msg> {
    paint(
        held_button(interaction, anim),
        (200.0, 100.0),
        1.0,
        interaction,
        text,
        dl,
    )
}

#[test]
fn button_press_springs_corners_to_r12() {
    let mut text = TextEngine::new();
    let mut dl = DrawList::default();
    let mut interaction = InteractionState::default();
    let mut anim = Animator::new();
    let t0 = Instant::now();

    // Frame 1: at rest the M button is a 20px pill.
    anim.begin_frame(t0, false);
    let frame = press_frame(&interaction, &mut anim, &mut text, &mut dl);
    assert!((rest_radius(&dl) - 20.0).abs() < EPS, "rest r20");

    // Pointer down → pressed → next frame retargets the corner spring at r12.
    let r = hit_rect(&frame, "press");
    frame.dispatch_event(
        &InputEvent::PointerDown {
            x: r.x + r.w * 0.5,
            y: r.y + r.h * 0.5,
            button: 272,
        },
        &mut interaction,
    );
    assert!(interaction.is_pressed("press".into()));

    anim.begin_frame(t0 + Duration::from_millis(16), false);
    press_frame(&interaction, &mut anim, &mut text, &mut dl);
    anim.begin_frame(t0 + Duration::from_millis(60), false);
    press_frame(&interaction, &mut anim, &mut text, &mut dl);
    let mid = rest_radius(&dl);
    assert!(
        mid < 20.0 && mid > RADII.sm,
        "corners tightening mid-flight: {mid}"
    );

    // Settled: the active state is the 12px squircle.
    anim.begin_frame(t0 + Duration::from_millis(900), false);
    press_frame(&interaction, &mut anim, &mut text, &mut dl);
    assert!(
        (rest_radius(&dl) - RADII.sm).abs() < 0.3,
        "pressed corners settle to r12, got {}",
        rest_radius(&dl)
    );

    // Release over the button → click fires and the pill springs back out.
    let (msgs, _) = frame.dispatch_event(
        &InputEvent::PointerUp {
            x: r.x + r.w * 0.5,
            y: r.y + r.h * 0.5,
            button: 272,
        },
        &mut interaction,
    );
    assert_eq!(msgs, vec![Msg::Click("press")]);
    // This frame retargets the spring; a later frame samples the settled pill.
    anim.begin_frame(t0 + Duration::from_millis(1800), false);
    press_frame(&interaction, &mut anim, &mut text, &mut dl);
    anim.begin_frame(t0 + Duration::from_millis(2100), false);
    press_frame(&interaction, &mut anim, &mut text, &mut dl);
    assert!(
        (rest_radius(&dl) - 20.0).abs() < 0.3,
        "released pill returns to r20, got {}",
        rest_radius(&dl)
    );
}

#[test]
fn button_press_morph_respects_reduce_motion() {
    let mut text = TextEngine::new();
    let mut dl = DrawList::default();
    let mut interaction = InteractionState::default();
    let mut anim = Animator::new();
    let theme = Theme::dark();
    let t0 = Instant::now();

    let build = |interaction: &InteractionState, anim: &mut Animator| {
        button(
            &theme,
            interaction,
            anim,
            Button {
                key: "press".into(),
                label: "Hold".into(),
                on_click: Some(Box::new(|| Msg::Click("press"))),
                ..Default::default()
            },
        )
    };

    anim.begin_frame(t0, true);
    let frame = paint(
        build(&interaction, &mut anim),
        (200.0, 100.0),
        1.0,
        &interaction,
        &mut text,
        &mut dl,
    );
    let r = hit_rect(&frame, "press");
    frame.dispatch_event(
        &InputEvent::PointerDown {
            x: r.x + r.w * 0.5,
            y: r.y + r.h * 0.5,
            button: 272,
        },
        &mut interaction,
    );

    // reduce_motion swaps the spring for a 120ms fade starting on the retarget
    // frame; it has finished by the next sampled frame.
    anim.begin_frame(t0 + Duration::from_millis(200), true);
    paint(
        build(&interaction, &mut anim),
        (200.0, 100.0),
        1.0,
        &interaction,
        &mut text,
        &mut dl,
    );
    anim.begin_frame(t0 + Duration::from_millis(400), true);
    paint(
        build(&interaction, &mut anim),
        (200.0, 100.0),
        1.0,
        &interaction,
        &mut text,
        &mut dl,
    );
    assert!(
        (rest_radius(&dl) - RADII.sm).abs() < EPS,
        "reduce_motion reaches r12 quickly, got {}",
        rest_radius(&dl)
    );
}

#[test]
fn button_hover_and_press_apply_state_layers() {
    let mut text = TextEngine::new();
    let mut dl = DrawList::default();
    let mut interaction = InteractionState::default();
    let mut anim = Animator::new();
    let theme = Theme::dark();
    let t0 = Instant::now();
    let bg = Rgba::from_color(theme.scheme.primary);
    let fg = Rgba::from_color(theme.scheme.on_primary);

    let build = |interaction: &InteractionState, anim: &mut Animator| {
        button(
            &theme,
            interaction,
            anim,
            Button {
                key: "hp".into(),
                label: "Hi".into(),
                on_click: Some(Box::new(|| Msg::Click("hp"))),
                ..Default::default()
            },
        )
    };

    anim.begin_frame(t0, false);
    let frame = paint(
        build(&interaction, &mut anim),
        (200.0, 100.0),
        1.0,
        &interaction,
        &mut text,
        &mut dl,
    );
    let r = hit_rect(&frame, "hp");
    let (cx, cy) = (r.x + r.w * 0.5, r.y + r.h * 0.5);

    // Hover → fg at 8% over the primary fill.
    frame.dispatch_event(&InputEvent::PointerMove { x: cx, y: cy }, &mut interaction);
    paint(
        build(&interaction, &mut anim),
        (200.0, 100.0),
        1.0,
        &interaction,
        &mut text,
        &mut dl,
    );
    assert_rgba(
        quads(&dl)[0].2.fill,
        bg.over(fg, STATE_LAYER.hover).to_array(),
        "hover state layer",
    );

    // Press → fg at 10%.
    frame.dispatch_event(
        &InputEvent::PointerDown {
            x: cx,
            y: cy,
            button: 272,
        },
        &mut interaction,
    );
    paint(
        build(&interaction, &mut anim),
        (200.0, 100.0),
        1.0,
        &interaction,
        &mut text,
        &mut dl,
    );
    assert_rgba(
        quads(&dl)[0].2.fill,
        bg.over(fg, STATE_LAYER.press).to_array(),
        "press state layer",
    );
}

// ── icon_button: circle↔squircle morph, glyph style swap, disabled ──────────

fn icon_btn(toggled: bool, interaction: &InteractionState, anim: &mut Animator) -> Element<Msg> {
    icon_button(
        &Theme::dark(),
        interaction,
        anim,
        IconButton {
            key: "ib".into(),
            icon: Icon::Settings,
            toggled,
            on_click: Some(Box::new(|| Msg::Click("ib"))),
            ..Default::default()
        },
    )
}

#[test]
fn icon_button_springs_circle_to_squircle_and_swaps_glyph_style() {
    let mut text = TextEngine::new();
    let mut dl = DrawList::default();
    let mut anim = Animator::new();
    let interaction = InteractionState::default();
    let theme = Theme::dark();
    let t0 = Instant::now();

    // At rest: 40px circle (r20), smoothing 0, transparent veil-free standard
    // icon button, glyph in the Outlined family.
    anim.begin_frame(t0, false);
    let el = icon_btn(false, &interaction, &mut anim);
    assert_eq!(
        text_font(&el.children[0]).family,
        IconStyle::Outlined.family()
    );
    paint(el, (100.0, 100.0), 1.0, &interaction, &mut text, &mut dl);
    let q = quads(&dl)[0].2;
    assert!(
        (q.radii[0] - 20.0).abs() < EPS,
        "rest circle r20: {}",
        q.radii[0]
    );
    assert!(q.misc[1].abs() < EPS, "rest smoothing 0: {}", q.misc[1]);
    assert!(q.fill[3] < EPS, "standard icon button is transparent");

    // Toggle on → retarget; mid-flight radius between circle and squircle.
    anim.begin_frame(t0 + Duration::from_millis(16), false);
    paint(
        icon_btn(true, &interaction, &mut anim),
        (100.0, 100.0),
        1.0,
        &interaction,
        &mut text,
        &mut dl,
    );
    anim.begin_frame(t0 + Duration::from_millis(80), false);
    paint(
        icon_btn(true, &interaction, &mut anim),
        (100.0, 100.0),
        1.0,
        &interaction,
        &mut text,
        &mut dl,
    );
    let mid = quads(&dl)[0].2;
    assert!(
        mid.radii[0] < 20.0 && mid.radii[0] > 12.0,
        "radius morphing: {}",
        mid.radii[0]
    );
    assert!(
        mid.misc[1] > 0.0 && mid.misc[1] < CORNER_SMOOTHING,
        "smoothing morphing: {}",
        mid.misc[1]
    );

    // Settled: r12 squircle at smoothing 0.6, primary fill, Filled glyph.
    anim.begin_frame(t0 + Duration::from_millis(1200), false);
    let el = icon_btn(true, &interaction, &mut anim);
    assert_eq!(
        text_font(&el.children[0]).family,
        IconStyle::Filled.family()
    );
    paint(el, (100.0, 100.0), 1.0, &interaction, &mut text, &mut dl);
    let done = quads(&dl)[0].2;
    assert!(
        (done.radii[0] - 12.0).abs() < 0.3,
        "toggled squircle r12: {}",
        done.radii[0]
    );
    assert!(
        (done.misc[1] - CORNER_SMOOTHING).abs() < EPS,
        "toggled smoothing 0.6: {}",
        done.misc[1]
    );
    assert_rgba(
        done.fill,
        Rgba::from_color(theme.scheme.primary).to_array(),
        "toggled primary fill",
    );
}

#[test]
fn icon_button_toggle_morph_respects_reduce_motion() {
    let mut text = TextEngine::new();
    let mut dl = DrawList::default();
    let mut anim = Animator::new();
    let interaction = InteractionState::default();
    let t0 = Instant::now();

    anim.begin_frame(t0, true);
    paint(
        icon_btn(false, &interaction, &mut anim),
        (100.0, 100.0),
        1.0,
        &interaction,
        &mut text,
        &mut dl,
    );
    anim.begin_frame(t0 + Duration::from_millis(200), true);
    paint(
        icon_btn(true, &interaction, &mut anim),
        (100.0, 100.0),
        1.0,
        &interaction,
        &mut text,
        &mut dl,
    );
    anim.begin_frame(t0 + Duration::from_millis(400), true);
    paint(
        icon_btn(true, &interaction, &mut anim),
        (100.0, 100.0),
        1.0,
        &interaction,
        &mut text,
        &mut dl,
    );
    let q = quads(&dl)[0].2;
    assert!(
        (q.radii[0] - 12.0).abs() < EPS,
        "reduce_motion lands on r12: {}",
        q.radii[0]
    );
    assert!((q.misc[1] - CORNER_SMOOTHING).abs() < EPS);
}

#[test]
fn icon_button_disabled_suppresses_click() {
    let mut text = TextEngine::new();
    let mut dl = DrawList::default();
    let interaction = InteractionState::default();
    let theme = Theme::dark();

    let frame = paint(
        icon_button(
            &theme,
            &interaction,
            &mut Animator::new(),
            IconButton {
                key: "ibd".into(),
                icon: Icon::Lock,
                disabled: true,
                on_click: Some(Box::new(|| Msg::Click("ibd"))),
                ..Default::default()
            },
        ),
        (100.0, 100.0),
        1.0,
        &interaction,
        &mut text,
        &mut dl,
    );
    assert!(!frame.hits.iter().any(|h| h.key == Some("ibd".into())));

    // Default tone is Text → transparent container, on_surface@.38 glyph only.
    let glyph_alpha = dl
        .batches
        .iter()
        .find_map(|b| match b {
            DrawBatch::Glyphs { instances, .. } => instances.first().map(|g| g.color[3]),
            _ => None,
        })
        .expect("icon glyph");
    assert!((glyph_alpha - STATE_LAYER.disabled_content).abs() < EPS);
}

// ── chip: filter selection, status kinds, dismiss affordance ─────────────────

#[test]
fn chip_filter_outlines_at_rest_and_fills_when_selected() {
    let mut text = TextEngine::new();
    for theme in [Theme::dark(), Theme::light()] {
        for scale in [1.0_f32, 1.75] {
            // Unselected: transparent + 1px `outline` hairline, r10, h36.
            let mut dl = DrawList::default();
            paint(
                chip(
                    &theme,
                    Chip {
                        key: "c".into(),
                        label: "Filter".into(),
                        on_click: Some(Box::new(|| Msg::Click("c"))),
                        ..Default::default()
                    },
                ),
                (160.0, 80.0),
                scale,
                &InteractionState::default(),
                &mut text,
                &mut dl,
            );
            let q = quads(&dl)[0].2;
            assert!((q.dst[3] - 36.0 * scale).abs() < EPS, "chip h36");
            assert!(q.fill[3] < EPS, "unselected chip is transparent");
            assert!((q.misc[2] - scale).abs() < EPS, "1px outline border");
            assert_rgba(
                q.border,
                Rgba::from_color(theme.scheme.outline).to_array(),
                "chip hairline",
            );
            for r in q.radii {
                assert!((r - 10.0 * scale).abs() < EPS, "chip r10: {r}");
            }

            // Selected: secondary_container fill, hairline gone, check leads.
            let mut dl = DrawList::default();
            let el = chip::<Msg>(
                &theme,
                Chip {
                    key: "c".into(),
                    label: "Filter".into(),
                    selected: true,
                    on_click: Some(Box::new(|| Msg::Click("c"))),
                    ..Default::default()
                },
            );
            assert_eq!(
                text_font(&el.children[0]).family,
                IconStyle::Outlined.family()
            );
            paint(
                el,
                (160.0, 80.0),
                scale,
                &InteractionState::default(),
                &mut text,
                &mut dl,
            );
            let q = quads(&dl)[0].2;
            assert_rgba(
                q.fill,
                Rgba::from_color(theme.scheme.secondary_container).to_array(),
                "selected chip fill",
            );
            assert!(q.misc[2].abs() < EPS, "selected chip drops hairline");
            // Leading check + label → at least two glyph runs' worth of glyphs.
            assert!(glyph_count(&dl) >= 2, "check + label glyphs");
        }
    }
}

#[test]
fn chip_status_kinds_paint_container_fills() {
    let mut text = TextEngine::new();
    let theme = Theme::dark();
    let s = &theme.scheme;
    for (kind, expected) in [
        (ChipKind::StatusNeutral, s.surface_container_high),
        (ChipKind::StatusWarm, s.tertiary_container),
        (ChipKind::StatusError, s.error_container),
    ] {
        let mut dl = DrawList::default();
        let frame = paint(
            chip::<Msg>(
                &theme,
                Chip {
                    key: "st".into(),
                    label: "Status".into(),
                    kind,
                    ..Default::default()
                },
            ),
            (160.0, 80.0),
            1.0,
            &InteractionState::default(),
            &mut text,
            &mut dl,
        );
        let q = quads(&dl)[0].2;
        assert_rgba(
            q.fill,
            Rgba::from_color(expected).to_array(),
            &format!("{kind:?} fill"),
        );
        assert!(q.misc[2].abs() < EPS, "status chip has no hairline");
        // Status chips are read-only — no interactive hit entry.
        assert!(!frame.hits.iter().any(|h| h.key == Some("st".into())));
    }
}

#[test]
fn chip_click_and_dismiss_are_separate_hit_areas() {
    let mut text = TextEngine::new();
    let mut dl = DrawList::default();
    let mut interaction = InteractionState::default();
    let theme = Theme::dark();

    let frame = paint(
        chip(
            &theme,
            Chip {
                key: "in".into(),
                label: "Tag".into(),
                on_click: Some(Box::new(|| Msg::Click("chip"))),
                on_dismiss: Some(Box::new(|| Msg::Click("dismiss"))),
                ..Default::default()
            },
        ),
        (200.0, 80.0),
        1.0,
        &interaction,
        &mut text,
        &mut dl,
    );
    let body = hit_rect(&frame, "in");
    let dismiss = frame
        .hits
        .iter()
        .find(|h| h.key.is_some() && h.key != Some("in".into()))
        .expect("dismiss hit area")
        .rect;

    assert_eq!(
        click(
            &frame,
            &mut interaction,
            body.x + body.w * 0.4,
            body.y + body.h * 0.5
        ),
        vec![Msg::Click("chip")],
        "chip body click"
    );
    assert_eq!(
        click(
            &frame,
            &mut interaction,
            dismiss.x + dismiss.w * 0.5,
            dismiss.y + dismiss.h * 0.5
        ),
        vec![Msg::Click("dismiss")],
        "trailing close click"
    );
}

// ── toggle_tile: r36↔r20 veil↔primary morph, sub-page chevron ────────────────

fn tile(on: bool, anim: &mut Animator) -> Element<Msg> {
    toggle_tile(
        &Theme::dark(),
        anim,
        ToggleTile {
            key: "tile".into(),
            icon: Icon::Wifi,
            title: "Wi-Fi".into(),
            subtitle: Some("Neptuno-5G".into()),
            on,
            on_click: Some(Box::new(|| Msg::Click("tile"))),
            on_sub_page: Some(Box::new(|| Msg::Click("chevron"))),
        },
    )
}

#[test]
fn toggle_tile_springs_r36_veil_pill_to_r20_primary() {
    let mut text = TextEngine::new();
    let mut dl = DrawList::default();
    let mut anim = Animator::new();
    let interaction = InteractionState::default();
    let theme = Theme::dark();
    let t0 = Instant::now();

    // Off: 72px r36 veil pill with on_surface content.
    anim.begin_frame(t0, false);
    paint(
        tile(false, &mut anim),
        (300.0, 100.0),
        1.0,
        &interaction,
        &mut text,
        &mut dl,
    );
    let q = quads(&dl)[0].2;
    assert!((q.dst[3] - 72.0).abs() < EPS, "tile h72");
    assert!((q.radii[0] - 36.0).abs() < EPS, "off r36: {}", q.radii[0]);
    assert_rgba(q.fill, theme.veil().to_array(), "off veil fill");

    // Toggle on → corners + fill spring on SPATIAL.
    anim.begin_frame(t0 + Duration::from_millis(16), false);
    paint(
        tile(true, &mut anim),
        (300.0, 100.0),
        1.0,
        &interaction,
        &mut text,
        &mut dl,
    );
    anim.begin_frame(t0 + Duration::from_millis(120), false);
    paint(
        tile(true, &mut anim),
        (300.0, 100.0),
        1.0,
        &interaction,
        &mut text,
        &mut dl,
    );
    let mid = quads(&dl)[0].2;
    assert!(
        mid.radii[0] < 36.0 && mid.radii[0] > RADII.lg,
        "corner morph mid-flight: {}",
        mid.radii[0]
    );

    // Settled: r20 primary squircle.
    anim.begin_frame(t0 + Duration::from_millis(1500), false);
    paint(
        tile(true, &mut anim),
        (300.0, 100.0),
        1.0,
        &interaction,
        &mut text,
        &mut dl,
    );
    let done = quads(&dl)[0].2;
    assert!(
        (done.radii[0] - RADII.lg).abs() < 0.3,
        "on r20: {}",
        done.radii[0]
    );
    assert_rgba(
        done.fill,
        Rgba::from_color(theme.scheme.primary).to_array(),
        "on primary fill",
    );
}

#[test]
fn toggle_tile_body_and_chevron_are_separate_hit_areas() {
    let mut text = TextEngine::new();
    let mut dl = DrawList::default();
    let mut interaction = InteractionState::default();
    let mut anim = Animator::new();

    anim.begin_frame(Instant::now(), false);
    let frame = paint(
        tile(true, &mut anim),
        (300.0, 100.0),
        1.0,
        &interaction,
        &mut text,
        &mut dl,
    );
    let body = hit_rect(&frame, "tile");
    let chevron = frame
        .hits
        .iter()
        .find(|h| h.key.is_some() && h.key != Some("tile".into()))
        .expect("chevron hit area")
        .rect;

    assert_eq!(
        click(
            &frame,
            &mut interaction,
            body.x + 20.0,
            body.y + body.h * 0.5
        ),
        vec![Msg::Click("tile")],
        "tile body toggles"
    );
    assert_eq!(
        click(
            &frame,
            &mut interaction,
            chevron.x + chevron.w * 0.5,
            chevron.y + chevron.h * 0.5
        ),
        vec![Msg::Click("chevron")],
        "chevron opens sub-page"
    );
}

#[test]
fn toggle_tile_morph_respects_reduce_motion() {
    let mut text = TextEngine::new();
    let mut dl = DrawList::default();
    let mut anim = Animator::new();
    let interaction = InteractionState::default();
    let theme = Theme::dark();
    let t0 = Instant::now();

    anim.begin_frame(t0, true);
    paint(
        tile(false, &mut anim),
        (300.0, 100.0),
        1.0,
        &interaction,
        &mut text,
        &mut dl,
    );
    anim.begin_frame(t0 + Duration::from_millis(200), true);
    paint(
        tile(true, &mut anim),
        (300.0, 100.0),
        1.0,
        &interaction,
        &mut text,
        &mut dl,
    );
    anim.begin_frame(t0 + Duration::from_millis(400), true);
    paint(
        tile(true, &mut anim),
        (300.0, 100.0),
        1.0,
        &interaction,
        &mut text,
        &mut dl,
    );
    let q = quads(&dl)[0].2;
    assert!(
        (q.radii[0] - RADII.lg).abs() < EPS,
        "reduce_motion lands on r20: {}",
        q.radii[0]
    );
    assert_rgba(
        q.fill,
        Rgba::from_color(theme.scheme.primary).to_array(),
        "reduce_motion primary fill",
    );
}

// ── segmented: connected corners, selection fill, styles, clicks ─────────────

fn seg(style: SegmentedStyle, selected: Option<usize>, theme: &Theme) -> Element<Msg> {
    segmented(
        "seg",
        style,
        selected,
        theme,
        (0..3)
            .map(|i| SegmentedItem {
                icon: Some(Icon::Settings),
                label: Some(format!("S{i}")),
                on_click: Some(Box::new(move || Msg::Index(i))),
            })
            .collect(),
    )
}

#[test]
fn segmented_cells_get_connected_corners_and_selection_fill() {
    let mut text = TextEngine::new();
    let theme = Theme::dark();
    for scale in [1.0_f32, 1.75] {
        let mut dl = DrawList::default();
        paint(
            seg(SegmentedStyle::Standard, Some(1), &theme),
            (400.0, 100.0),
            scale,
            &InteractionState::default(),
            &mut text,
            &mut dl,
        );
        let cells: Vec<_> = quads(&dl)
            .into_iter()
            .map(|q| q.2)
            .filter(|q| q.misc[0] == KIND_FILL && q.dst[3] > 40.0 * scale)
            .collect();
        assert_eq!(cells.len(), 3, "three cell quads");
        let inner = RADII.connected_inner * scale;
        let end = RADII.connected_end * scale;
        let expect = [
            [end, inner, inner, end],
            [inner; 4],
            [inner, end, end, inner],
        ];
        for (i, q) in cells.iter().enumerate() {
            for c in 0..4 {
                assert!(
                    (q.radii[c] - expect[i][c]).abs() < EPS,
                    "cell {i} corner {c}: {} vs {}",
                    q.radii[c],
                    expect[i][c]
                );
            }
        }
        // Selected cell 1 = primary; rest = surface_container_high.
        assert_rgba(
            cells[1].fill,
            Rgba::from_color(theme.scheme.primary).to_array(),
            "selected cell",
        );
        assert_rgba(
            cells[0].fill,
            Rgba::from_color(theme.scheme.surface_container_high).to_array(),
            "unselected cell",
        );
        // h44 cells separated by a 3px gap.
        assert!((cells[0].dst[3] - 44.0 * scale).abs() < EPS, "cell h44");
        let gap = cells[1].dst[0] - (cells[0].dst[0] + cells[0].dst[2]);
        assert!((gap - 3.0 * scale).abs() < EPS, "3px join gap: {gap}");
        // flex:1 — all cells share the width within pixel snapping.
        assert!(
            (cells[0].dst[2] - cells[1].dst[2]).abs() <= 2.0,
            "cell widths {:?}",
            cells.iter().map(|c| c.dst[2]).collect::<Vec<_>>()
        );
    }
}

#[test]
fn segmented_clicks_emit_per_cell_message() {
    let mut text = TextEngine::new();
    let mut dl = DrawList::default();
    let mut interaction = InteractionState::default();
    let theme = Theme::dark();

    let frame = paint(
        seg(SegmentedStyle::Standard, None, &theme),
        (400.0, 100.0),
        1.0,
        &interaction,
        &mut text,
        &mut dl,
    );
    let cells: Vec<_> = frame
        .hits
        .iter()
        .filter(|h| h.key.is_some() && h.key != Some("seg".into()))
        .collect();
    assert_eq!(cells.len(), 3, "three interactive cells");
    for (i, cell) in cells.iter().enumerate() {
        assert_eq!(
            click(
                &frame,
                &mut interaction,
                cell.rect.x + cell.rect.w * 0.5,
                cell.rect.y + cell.rect.h * 0.5,
            ),
            vec![Msg::Index(i)],
            "cell {i} click"
        );
    }
}

#[test]
fn segmented_large_and_veil_styles() {
    let mut text = TextEngine::new();
    let theme = Theme::dark();

    // Large: h48, 24px end radii.
    let mut dl = DrawList::default();
    paint(
        seg(SegmentedStyle::Large, Some(0), &theme),
        (400.0, 100.0),
        1.0,
        &InteractionState::default(),
        &mut text,
        &mut dl,
    );
    let cells: Vec<_> = quads(&dl)
        .into_iter()
        .map(|q| q.2)
        .filter(|q| q.misc[0] == KIND_FILL && q.dst[3] > 40.0)
        .collect();
    assert!((cells[0].dst[3] - SIZE.control_l).abs() < EPS, "large h48");
    assert!(
        (cells[0].radii[0] - 24.0).abs() < EPS && (cells[0].radii[3] - 24.0).abs() < EPS,
        "large end r24"
    );

    // Veil: unselected cells take the veil fill, selected stays primary.
    let mut dl = DrawList::default();
    paint(
        seg(SegmentedStyle::Veil, Some(2), &theme),
        (400.0, 100.0),
        1.0,
        &InteractionState::default(),
        &mut text,
        &mut dl,
    );
    let cells: Vec<_> = quads(&dl)
        .into_iter()
        .map(|q| q.2)
        .filter(|q| q.misc[0] == KIND_FILL && q.dst[3] > 40.0)
        .collect();
    assert_rgba(cells[0].fill, theme.veil().to_array(), "veil cell");
    assert_rgba(
        cells[2].fill,
        Rgba::from_color(theme.scheme.primary).to_array(),
        "selected over veil",
    );
}

// ── power_menu: expanding cell morph, compact corners, clicks ────────────────

fn power(focused: usize, anim: &mut Animator) -> Element<Msg> {
    power_menu(
        "pm",
        focused,
        &Theme::dark(),
        anim,
        vec![
            PowerItem {
                icon: Icon::Lock,
                label: "Lock".into(),
                hint: None,
                tone: PowerCellTone::Primary,
                on_click: Some(Box::new(|| Msg::Index(0))),
            },
            PowerItem {
                icon: Icon::Bedtime,
                label: "Sleep".into(),
                hint: None,
                tone: PowerCellTone::Neutral,
                on_click: Some(Box::new(|| Msg::Index(1))),
            },
            PowerItem {
                icon: Icon::PowerSettingsNew,
                label: "Shut down".into(),
                hint: Some("Press Enter".into()),
                tone: PowerCellTone::Tertiary,
                on_click: Some(Box::new(|| Msg::Index(2))),
            },
        ],
    )
}

/// Fill quads for the three cells, in paint order.
fn power_cells(dl: &DrawList) -> Vec<QuadInstance> {
    quads(dl)
        .into_iter()
        .map(|q| q.2)
        .filter(|q| q.misc[0] == KIND_FILL && q.dst[3] >= 139.0)
        .collect()
}

/// Assert all three compact 140×140 cells with 70/14 connected corners.
fn assert_compact_cells(cells: &[QuadInstance]) {
    assert_eq!(cells.len(), 3, "three cell quads");
    for (i, q) in cells.iter().enumerate() {
        assert!((q.dst[2] - 140.0).abs() < EPS && (q.dst[3] - 140.0).abs() < EPS);
        let exp = match i {
            0 => [70.0, 14.0, 14.0, 70.0],
            2 => [14.0, 70.0, 70.0, 14.0],
            _ => [14.0; 4],
        };
        for (c, (&r, &e)) in q.radii.iter().zip(exp.iter()).enumerate() {
            assert!((r - e).abs() < EPS, "cell {i} corner {c}: {r} vs {e}");
        }
    }
}

#[test]
fn power_menu_focused_cell_expands_into_labeled_pill() {
    let mut text = TextEngine::new();
    let mut dl = DrawList::default();
    let mut anim = Animator::new();
    let interaction = InteractionState::default();
    let theme = Theme::dark();
    let t0 = Instant::now();

    // No focus: all three cells are compact with 70/14 connected corners.
    anim.begin_frame(t0, false);
    paint(
        power(usize::MAX, &mut anim),
        (600.0, 200.0),
        1.0,
        &interaction,
        &mut text,
        &mut dl,
    );
    let cells = power_cells(&dl);
    assert_compact_cells(&cells);
    // Neutral cell fill + no floating shadow anywhere at rest.
    assert_rgba(
        cells[1].fill,
        Rgba::from_color(theme.scheme.surface_container_high).to_array(),
        "neutral cell",
    );
    assert!(
        !quads(&dl).iter().any(|q| q.2.misc[0] == KIND_SHADOW),
        "no shadow on compact cells"
    );

    // Focus cell 2 → width + radii spring toward the 300px pill.
    anim.begin_frame(t0 + Duration::from_millis(16), false);
    paint(
        power(2, &mut anim),
        (700.0, 200.0),
        1.0,
        &interaction,
        &mut text,
        &mut dl,
    );
    anim.begin_frame(t0 + Duration::from_millis(120), false);
    paint(
        power(2, &mut anim),
        (700.0, 200.0),
        1.0,
        &interaction,
        &mut text,
        &mut dl,
    );
    let mid = power_cells(&dl);
    assert!(
        mid[2].dst[2] > 140.0 && mid[2].dst[2] < 300.0,
        "expanding mid-flight: {}",
        mid[2].dst[2]
    );
    assert!(
        mid[0].dst[2] < 141.0 && mid[1].dst[2] < 141.0,
        "siblings stay compact"
    );

    // Settled: 300px pill (all corners ≈ 70 = h/2) with tertiary fill, shadow,
    // and the expanded label+hint glyphs visible.
    anim.begin_frame(t0 + Duration::from_millis(1500), false);
    paint(
        power(2, &mut anim),
        (700.0, 200.0),
        1.0,
        &interaction,
        &mut text,
        &mut dl,
    );
    let done = power_cells(&dl);
    assert!(
        (done[2].dst[2] - 300.0).abs() < 1.0,
        "expanded w300: {}",
        done[2].dst[2]
    );
    for (c, &r) in done[2].radii.iter().enumerate() {
        assert!((r - 70.0).abs() < 0.5, "expanded pill corner {c}: {r}");
    }
    assert_rgba(
        done[2].fill,
        Rgba::from_color(theme.scheme.tertiary).to_array(),
        "expanded tertiary fill",
    );
    assert!(
        quads(&dl).iter().any(|q| q.2.misc[0] == KIND_SHADOW),
        "expanded cell floats with a shadow"
    );
    assert!(
        glyph_count(&dl) >= 4,
        "icon + caption + title + hint glyphs"
    );
}

#[test]
fn power_menu_clicks_emit_per_cell_message() {
    let mut text = TextEngine::new();
    let mut dl = DrawList::default();
    let mut interaction = InteractionState::default();
    let mut anim = Animator::new();

    anim.begin_frame(Instant::now(), false);
    let frame = paint(
        power(1, &mut anim),
        (700.0, 200.0),
        1.0,
        &interaction,
        &mut text,
        &mut dl,
    );
    let cells: Vec<_> = frame
        .hits
        .iter()
        .filter(|h| h.key.is_some() && h.key != Some("pm".into()))
        .collect();
    assert_eq!(cells.len(), 3);
    for (i, cell) in cells.iter().enumerate() {
        assert_eq!(
            click(
                &frame,
                &mut interaction,
                cell.rect.x + cell.rect.w * 0.5,
                cell.rect.y + cell.rect.h * 0.5,
            ),
            vec![Msg::Index(i)],
            "cell {i} click"
        );
    }
}

#[test]
fn power_menu_expansion_respects_reduce_motion() {
    let mut text = TextEngine::new();
    let mut dl = DrawList::default();
    let mut anim = Animator::new();
    let interaction = InteractionState::default();
    let t0 = Instant::now();

    anim.begin_frame(t0, true);
    paint(
        power(usize::MAX, &mut anim),
        (600.0, 200.0),
        1.0,
        &interaction,
        &mut text,
        &mut dl,
    );
    anim.begin_frame(t0 + Duration::from_millis(200), true);
    paint(
        power(0, &mut anim),
        (700.0, 200.0),
        1.0,
        &interaction,
        &mut text,
        &mut dl,
    );
    anim.begin_frame(t0 + Duration::from_millis(400), true);
    paint(
        power(0, &mut anim),
        (700.0, 200.0),
        1.0,
        &interaction,
        &mut text,
        &mut dl,
    );
    let cells = power_cells(&dl);
    assert!(
        (cells[0].dst[2] - 300.0).abs() < EPS,
        "reduce_motion expands immediately: {}",
        cells[0].dst[2]
    );
}
