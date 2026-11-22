//! Headless `DrawList` integration tests for triton-kit Phase 1A.1 surface
//! primitives. Everything is exercised through public APIs: `layout_and_paint`
//! at 1.0 and 1.75 scale, `BuiltFrame::dispatch_event`, deterministic
//! `Animator::begin_frame`, dark and light `Theme`.

use std::time::{Duration, Instant};

use triton_design::color::STATE_LAYER;
use triton_design::material::{Material, ShadowToken};
use triton_design::shape::CORNER_SMOOTHING;
use triton_engine::anim::Animator;
use triton_engine::gpu::{
    DrawBatch, DrawList, EdgeFade, KIND_FILL, KIND_SHADOW, KIND_TOP_HIGHLIGHT, QuadInstance,
    ScissorRect,
};
use triton_engine::image::ImageStore;
use triton_engine::style::{DropShadow, Edges, Rgba, Theme};
use triton_engine::text::TextEngine;
use triton_engine::ui::{
    BuiltFrame, Element, InputEvent, InteractionState, NodeKey, box_, col, layout_and_paint,
};
use triton_kit::{
    Dialog, DividerStyle, ListFill, Orientation, ScrollAreaOptions, card, dialog, divider,
    empty_state, island, list_group, list_row, list_row_text, panel, scrim, scroll_area,
    shadow_gutter,
};

const EPS: f32 = 1e-3;

#[derive(Debug, Clone, PartialEq)]
enum Msg {
    Dismiss,
    Action(&'static str),
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

// ── panel: materials, highlight, shadow, blur regions ─────────────────────────

#[test]
fn panel_reports_blur_for_translucent_materials_only() {
    let mut text = TextEngine::new();
    for (mat, blurs) in [
        (Material::Solid, 0usize),
        (Material::Mica, 1),
        (Material::Glass, 1),
        (Material::Thick, 1),
    ] {
        for scale in [1.0_f32, 1.75] {
            let mut dl = DrawList::default();
            let frame = paint(
                panel::<Msg>(mat, 20.0, &Theme::dark()).w(120.0).h(80.0),
                (120.0, 80.0),
                scale,
                &InteractionState::default(),
                &mut text,
                &mut dl,
            );
            assert_eq!(frame.blur_rects.len(), blurs, "{mat:?} at {scale}x");
        }
    }
}

#[test]
fn panel_paints_highlight_shadow_and_border_per_material() {
    let mut text = TextEngine::new();
    for theme in [Theme::dark(), Theme::light()] {
        let mut dl = DrawList::default();
        paint::<Msg>(
            col().w(300.0).h(300.0).children([
                panel(Material::Solid, 12.0, &theme).w(100.0).h(40.0),
                panel(Material::Glass, 12.0, &theme).w(100.0).h(40.0),
                panel(Material::Thick, 12.0, &theme).w(100.0).h(40.0),
            ]),
            (300.0, 300.0),
            1.75,
            &InteractionState::default(),
            &mut text,
            &mut dl,
        );
        let qs = quads(&dl);
        let fills: Vec<_> = qs
            .iter()
            .filter(|q| q.2.misc[0] == KIND_FILL || q.2.misc[0] == KIND_TOP_HIGHLIGHT)
            .collect();
        assert_eq!(fills.len(), 3);
        // Solid → plain fill; Glass & Thick → top-highlight kind + hairline border.
        assert_eq!(fills[0].2.misc[0], KIND_FILL);
        assert_eq!(fills[1].2.misc[0], KIND_TOP_HIGHLIGHT);
        assert_eq!(fills[2].2.misc[0], KIND_TOP_HIGHLIGHT);
        assert!(
            (fills[1].2.misc[2] - 1.75).abs() < EPS,
            "1px border at 1.75x"
        );
        // Shadows only on floating materials (Glass, Thick).
        let shadows: Vec<_> = qs.iter().filter(|q| q.2.misc[0] == KIND_SHADOW).collect();
        assert_eq!(shadows.len(), 2);
        // Translucent fills request the interior cutout; FLOATING blur 40 vs FLOATING_LARGE 80.
        assert!((shadows[0].2.border[3] - 1.0).abs() < EPS);
        assert!((shadows[0].2.misc[2] - 40.0 * 1.75).abs() < 0.5);
        assert!((shadows[1].2.misc[2] - 80.0 * 1.75).abs() < 0.5);
    }
}

#[test]
fn panel_smoothing_matches_material_float_state() {
    // Floating materials (Glass, Thick) stay circular to match niri's window
    // corners; grounded materials (Solid, Mica) keep CORNER_SMOOTHING.
    let mut text = TextEngine::new();
    for (mat, smoothing) in [
        (Material::Solid, CORNER_SMOOTHING),
        (Material::Mica, CORNER_SMOOTHING),
        (Material::Glass, 0.0),
        (Material::Thick, 0.0),
    ] {
        let mut dl = DrawList::default();
        paint::<Msg>(
            panel(mat, 16.0, &Theme::dark()).w(80.0).h(50.0),
            (80.0, 50.0),
            1.0,
            &InteractionState::default(),
            &mut text,
            &mut dl,
        );
        let body = quads(&dl)
            .into_iter()
            .map(|q| q.2)
            .find(|q| q.misc[0] == KIND_FILL || q.misc[0] == KIND_TOP_HIGHLIGHT)
            .expect("panel body quad");
        assert!(
            (body.misc[1] - smoothing).abs() < EPS,
            "{mat:?} smoothing {} (expected {smoothing})",
            body.misc[1]
        );
    }
}

#[test]
fn material_fill_matches_theme_tints() {
    let mut text = TextEngine::new();
    let mut dl = DrawList::default();
    let theme = Theme::dark();
    paint::<Msg>(
        col().children([
            panel(Material::Solid, 0.0, &theme).w(10.0).h(10.0),
            panel(Material::Glass, 0.0, &theme).w(10.0).h(10.0),
        ]),
        (100.0, 100.0),
        1.0,
        &InteractionState::default(),
        &mut text,
        &mut dl,
    );
    let qs = quads(&dl);
    let solid = theme.material_fill(Material::Solid).to_array();
    let glass = theme.material_fill(Material::Glass).to_array();
    assert_rgba(qs[0].2.fill, solid, "solid fill");
    assert_eq!(qs[1].2.misc[0], KIND_SHADOW, "Glass shadow before fill");
    assert_rgba(qs[2].2.fill, glass, "glass fill");
}

// ── island + shadow_gutter ────────────────────────────────────────────────────

#[test]
fn island_is_glass_pill_with_floating_shadow_and_gutter() {
    let mut text = TextEngine::new();
    let mut dl = DrawList::default();
    let frame = paint::<Msg>(
        shadow_gutter(island(&Theme::dark()).child(box_().w(80.0).h(30.0))),
        (400.0, 200.0),
        1.0,
        &InteractionState::default(),
        &mut text,
        &mut dl,
    );
    // gutter: FLOATING blur40 → pad 80; y offset 16 → top 64, bottom 96.
    let g = DropShadow::from(ShadowToken::FLOATING).gutter();
    let h = 56.0 + g.top + g.bottom;
    assert!(
        (frame.content_size.1 - h).abs() < EPS,
        "island+gutter height"
    );
    assert_eq!(frame.blur_rects.len(), 1, "island reports Glass blur");
    let qs = quads(&dl);
    let island_fill = qs
        .iter()
        .find(|q| q.2.misc[0] == KIND_TOP_HIGHLIGHT)
        .expect("island quad");
    // Pill: radii clamp to h/2 = 28, smoothing 0.
    assert!((island_fill.2.radii[0] - 28.0).abs() < EPS);
    assert!(
        (island_fill.2.misc[1] - 0.0).abs() < EPS,
        "pill smoothing 0"
    );
}

// ── card ──────────────────────────────────────────────────────────────────────

#[test]
fn card_uses_container_low_r24_and_20x22_padding() {
    let mut text = TextEngine::new();
    let mut dl = DrawList::default();
    let theme = Theme::dark();
    paint::<Msg>(
        card(&theme).child(box_().w(30.0).h(10.0).bg(Rgba::WHITE)),
        (300.0, 200.0),
        1.0,
        &InteractionState::default(),
        &mut text,
        &mut dl,
    );
    let qs = quads(&dl);
    let card_q = qs[0].2;
    assert_eq!(
        card_q.fill,
        Rgba::from_color(theme.scheme.surface_container_low).to_array()
    );
    assert!((card_q.radii[0] - 24.0).abs() < EPS);
    // Child sits at padding (22, 20).
    assert!((qs[1].2.dst[0] - 22.0).abs() < EPS && (qs[1].2.dst[1] - 20.0).abs() < EPS);
}

// ── scrim + dialog ────────────────────────────────────────────────────────────

#[test]
fn scrim_covers_surface_at_45_percent_and_captures_input() {
    let mut text = TextEngine::new();
    let mut dl = DrawList::default();
    let frame = paint::<Msg>(
        scrim(),
        (300.0, 200.0),
        1.75,
        &InteractionState::default(),
        &mut text,
        &mut dl,
    );
    let qs = quads(&dl);
    assert_rgba(qs[0].2.fill, [0.0, 0.0, 0.0, 0.45], "scrim");
    assert_eq!(qs[0].2.dst, [0.0, 0.0, 525.0, 350.0]);
    assert_eq!(frame.input_rects.len(), 1, "scrim captures all input");
}

fn action_button(
    key: &str,
    label: &str,
    tone: triton_kit::ButtonTone,
    tag: &'static str,
) -> Element<Msg> {
    triton_kit::button(
        &Theme::dark(),
        &InteractionState::default(),
        &mut Animator::new(),
        triton_kit::Button {
            key: key.into(),
            label: label.into(),
            tone,
            on_click: Some(Box::new(move || Msg::Action(tag))),
            ..Default::default()
        },
    )
}

#[test]
fn scrim_blocks_clicks_scrolls_and_hover_below() {
    let mut text = TextEngine::new();
    let mut dl = DrawList::default();
    let mut interaction = InteractionState::default();
    let mut anim = Animator::new();
    let theme = Theme::dark();
    let t0 = Instant::now();

    // A clickable button and a scrollable column sit *below* the scrim.
    let below = triton_kit::button(
        &theme,
        &interaction,
        &mut anim,
        triton_kit::Button {
            key: "below".into(),
            label: "Below".into(),
            on_click: Some(Box::new(|| Msg::Action("below"))),
            ..Default::default()
        },
    );
    let scroller = scroll_area::<Msg>(
        "sa-below",
        120.0,
        ScrollAreaOptions::default(),
        &theme,
        &interaction,
        &mut anim,
        (0..8).map(|_| box_().w(160.0).h(40.0).bg(Rgba::WHITE)),
    );
    anim.begin_frame(t0, false);
    let frame = paint(
        box_().w_full().h_full().children([
            col().children([below, scroller]),
            scrim().absolute().inset_all(0.0),
        ]),
        (300.0, 200.0),
        1.0,
        &interaction,
        &mut text,
        &mut dl,
    );

    // Hover: the covered button key is invisible to hover queries.
    frame.dispatch_event(
        &InputEvent::PointerMove { x: 40.0, y: 30.0 },
        &mut interaction,
    );
    assert!(
        !interaction.hovered_keys.contains(&NodeKey::named("below")),
        "hover must not reach through the scrim: {:?}",
        interaction.hovered_keys
    );
    // Click: press+release on the scrim never fires the button below.
    frame.dispatch_event(
        &InputEvent::PointerDown {
            x: 40.0,
            y: 30.0,
            button: 272,
        },
        &mut interaction,
    );
    let (msgs, _) = frame.dispatch_event(
        &InputEvent::PointerUp {
            x: 40.0,
            y: 30.0,
            button: 272,
        },
        &mut interaction,
    );
    assert!(msgs.is_empty(), "scrim swallows clicks: {msgs:?}");
    // Wheel: the scroll container below receives nothing.
    frame.dispatch_event(
        &InputEvent::Scroll {
            x: 40.0,
            y: 80.0,
            delta_y: 60.0,
        },
        &mut interaction,
    );
    assert!(
        interaction.scroll_offset(NodeKey::named("sa-below")).abs() < EPS,
        "wheel must not scroll content behind the scrim"
    );
}

#[test]
fn dialog_without_dismiss_still_blocks_input_below() {
    let mut text = TextEngine::new();
    let mut dl = DrawList::default();
    let mut interaction = InteractionState::default();
    let theme = Theme::dark();

    // Non-dismissable dialog (on_dismiss: None): the scrim still has to eat
    // every click/scroll so nothing beneath is reachable.
    let below = triton_kit::button(
        &theme,
        &interaction,
        &mut Animator::new(),
        triton_kit::Button {
            key: "below".into(),
            label: "Below".into(),
            on_click: Some(Box::new(|| Msg::Action("below"))),
            ..Default::default()
        },
    );
    let frame = paint(
        box_().w_full().h_full().children([
            box_().p(Edges::all(20.0)).child(below),
            dialog(
                &theme,
                Dialog {
                    title: Some("Working".into()),
                    on_dismiss: None,
                    ..Default::default()
                },
            )
            .absolute()
            .inset_all(0.0),
        ]),
        (600.0, 400.0),
        1.0,
        &interaction,
        &mut text,
        &mut dl,
    );

    frame.dispatch_event(
        &InputEvent::PointerMove { x: 30.0, y: 30.0 },
        &mut interaction,
    );
    assert!(
        !interaction.hovered_keys.contains(&NodeKey::named("below")),
        "hover must stop at the scrim: {:?}",
        interaction.hovered_keys
    );
    frame.dispatch_event(
        &InputEvent::PointerDown {
            x: 30.0,
            y: 30.0,
            button: 272,
        },
        &mut interaction,
    );
    let (msgs, _) = frame.dispatch_event(
        &InputEvent::PointerUp {
            x: 30.0,
            y: 30.0,
            button: 272,
        },
        &mut interaction,
    );
    assert!(
        msgs.is_empty(),
        "non-dismissable scrim still swallows clicks: {msgs:?}"
    );
}

#[test]
fn dialog_paints_scrim_thick_card_and_blocks_dismiss_on_card() {
    let mut text = TextEngine::new();
    let mut dl = DrawList::default();
    let mut interaction = InteractionState::default();
    let theme = Theme::dark();

    let frame = paint(
        dialog(
            &theme,
            Dialog {
                title: Some("Delete items?".into()),
                body: Some("They can be restored for 30 days.".into()),
                actions: vec![
                    action_button("cancel", "Cancel", triton_kit::ButtonTone::Text, "cancel"),
                    action_button(
                        "del",
                        "Delete",
                        triton_kit::ButtonTone::Destructive,
                        "delete",
                    ),
                ],
                on_dismiss: Some(Box::new(|| Msg::Dismiss)),
                ..Default::default()
            },
        ),
        (600.0, 400.0),
        1.0,
        &interaction,
        &mut text,
        &mut dl,
    );

    let qs = quads(&dl);
    assert!(
        qs.iter().any(|q| q
            .2
            .fill
            .iter()
            .zip([0.0, 0.0, 0.0, 0.45])
            .all(|(a, e)| (a - e).abs() < EPS)),
        "scrim quad"
    );
    assert_eq!(frame.blur_rects.len(), 1, "Thick card blur region");
    assert!(qs.iter().any(|q| q.2.misc[0] == KIND_TOP_HIGHLIGHT));
    assert!(
        qs.iter()
            .any(|q| (q.2.misc[0] - KIND_SHADOW).abs() < EPS && (q.2.misc[2] - 80.0).abs() < 0.5)
    );
    assert!(glyph_count(&dl) > 0, "title/body/action glyphs");

    // Click on the scrim (corner) → Dismiss; click on the Thick card dead area → nothing.
    let card_rect = frame.blur_rects[0].rect;
    frame.dispatch_event(
        &InputEvent::PointerDown {
            x: 5.0,
            y: 5.0,
            button: 272,
        },
        &mut interaction,
    );
    let (m, _) = frame.dispatch_event(
        &InputEvent::PointerUp {
            x: 5.0,
            y: 5.0,
            button: 272,
        },
        &mut interaction,
    );
    assert_eq!(m, vec![Msg::Dismiss]);
    let cx = card_rect.x + 4.0;
    let cy = card_rect.y + 4.0;
    frame.dispatch_event(
        &InputEvent::PointerDown {
            x: cx,
            y: cy,
            button: 272,
        },
        &mut interaction,
    );
    let (m2, _) = frame.dispatch_event(
        &InputEvent::PointerUp {
            x: cx,
            y: cy,
            button: 272,
        },
        &mut interaction,
    );
    assert!(
        m2.is_empty(),
        "card is input-opaque: dead card area must not dismiss"
    );
}

#[test]
fn destructive_button_tone_uses_error_container() {
    let mut text = TextEngine::new();
    let mut dl = DrawList::default();
    let theme = Theme::dark();
    paint::<Msg>(
        triton_kit::button(
            &theme,
            &InteractionState::default(),
            &mut Animator::new(),
            triton_kit::Button {
                key: "d".into(),
                label: "X".into(),
                tone: triton_kit::ButtonTone::Destructive,
                on_click: Some(Box::new(|| Msg::Dismiss)),
                ..Default::default()
            },
        ),
        (200.0, 100.0),
        1.0,
        &InteractionState::default(),
        &mut text,
        &mut dl,
    );
    let qs = quads(&dl);
    assert_eq!(
        qs[0].2.fill,
        Rgba::from_color(theme.scheme.error_container).to_array()
    );
}

// ── divider ──────────────────────────────────────────────────────────────────

#[test]
fn divider_is_1px_outline_variant_hairline() {
    let mut text = TextEngine::new();
    let mut dl = DrawList::default();
    let theme = Theme::dark();
    paint::<Msg>(
        col().w(200.0).h(50.0).children([
            divider(Orientation::Horizontal, DividerStyle::Standard, &theme),
            divider(Orientation::Vertical, DividerStyle::OnGlass, &theme).h(20.0),
        ]),
        (200.0, 50.0),
        1.75,
        &InteractionState::default(),
        &mut text,
        &mut dl,
    );
    let qs = quads(&dl);
    assert_eq!(
        qs[0].2.fill,
        Rgba::from_color(theme.scheme.outline_variant).to_array()
    );
    assert!((qs[0].2.dst[3] - 1.75).abs() < EPS, "1px @1.75x");
    assert!((qs[0].2.dst[2] - 350.0).abs() < EPS, "w_full");
    assert_eq!(
        qs[1].2.fill,
        Rgba::from_color(theme.scheme.on_surface)
            .with_alpha(STATE_LAYER.disabled_container)
            .to_array(),
        "on-glass dark hairline"
    );
    assert!((qs[1].2.dst[2] - 1.75).abs() < EPS);
}

// ── empty_state ───────────────────────────────────────────────────────────────

#[test]
fn empty_state_centers_title_subtitle_in_on_surface_variant() {
    let mut text = TextEngine::new();
    let mut dl = DrawList::default();
    let theme = Theme::dark();
    paint::<Msg>(
        col().w(400.0).h(200.0).child(empty_state(
            Some(triton_design::icon::Icon::Search),
            "No results",
            Some("Try a different query"),
            &theme,
        )),
        (400.0, 200.0),
        1.0,
        &InteractionState::default(),
        &mut text,
        &mut dl,
    );
    assert!(glyph_count(&dl) >= 3, "icon + two text runs");
    // Glyph extents should be roughly centered horizontally in the 400px box.
    let mut min_x = f32::MAX;
    let mut max_x = 0f32;
    for b in &dl.batches {
        if let DrawBatch::Glyphs { instances, .. } = b {
            for g in instances {
                min_x = min_x.min(g.dst[0]);
                max_x = max_x.max(g.dst[0] + g.dst[2]);
            }
        }
    }
    let center = min_x.midpoint(max_x);
    assert!(
        (center - 200.0).abs() < 60.0,
        "content roughly centered, got {center}"
    );
}

// ── list_group / list_row ─────────────────────────────────────────────────────

#[test]
fn list_group_assigns_outer_inner_and_single_corners() {
    let mut text = TextEngine::new();
    let theme = Theme::dark();
    let build = |n: usize| -> Element<Msg> {
        col().w(300.0).child(list_group(
            ListFill::Container,
            &theme,
            (0..n).map(|i| {
                list_row(
                    NodeKey::from_pair("row", i),
                    ListFill::Container,
                    false,
                    &theme,
                    None,
                    [list_row_text("Title", Some("Sub"), false, &theme)],
                )
            }),
        ))
    };
    for scale in [1.0_f32, 1.75] {
        let mut dl = DrawList::default();
        paint(
            build(3),
            (300.0, 400.0),
            scale,
            &InteractionState::default(),
            &mut text,
            &mut dl,
        );
        let row_quads: Vec<_> = quads(&dl)
            .into_iter()
            .map(|q| q.2)
            .filter(|q| q.misc[0] == KIND_FILL)
            .collect();
        assert_eq!(row_quads.len(), 3);
        let expect = [[24.0, 24.0, 6.0, 6.0], [6.0; 4], [6.0, 6.0, 24.0, 24.0]];
        for (i, q) in row_quads.iter().enumerate() {
            for c in 0..4 {
                assert!(
                    (q.radii[c] - expect[i][c] * scale).abs() < EPS,
                    "row {i} corner {c}"
                );
            }
            assert_eq!(
                q.fill,
                Rgba::from_color(theme.scheme.surface_container).to_array()
            );
        }
    }
    // Single-item group → [24;4].
    let mut dl = DrawList::default();
    paint(
        build(1),
        (300.0, 100.0),
        1.0,
        &InteractionState::default(),
        &mut text,
        &mut dl,
    );
    let q = quads(&dl)[0].2;
    assert!(q.radii.iter().all(|&r| (r - 24.0).abs() < EPS));
}

#[test]
fn list_group_veil_and_selected_fill() {
    let mut text = TextEngine::new();
    let mut dl = DrawList::default();
    let theme = Theme::dark();
    paint::<Msg>(
        col().w(300.0).child(list_group(
            ListFill::Veil,
            &theme,
            [
                list_row(
                    "a",
                    ListFill::Veil,
                    true,
                    &theme,
                    None,
                    [list_row_text("Sel", None, true, &theme)],
                ),
                list_row(
                    "b",
                    ListFill::Veil,
                    false,
                    &theme,
                    None,
                    [list_row_text("Row", None, false, &theme)],
                ),
            ],
        )),
        (300.0, 200.0),
        1.0,
        &InteractionState::default(),
        &mut text,
        &mut dl,
    );
    let qs = quads(&dl);
    assert_eq!(
        qs[0].2.fill,
        Rgba::from_color(theme.scheme.secondary_container).to_array(),
        "selected wins over Veil"
    );
    assert_rgba(qs[1].2.fill, theme.veil().to_array(), "Veil row fill");
}

// ── scroll_area ───────────────────────────────────────────────────────────────

fn scroll_frame(
    interaction: &InteractionState,
    anim: &mut Animator,
    theme: &Theme,
    text: &mut TextEngine,
    dl: &mut DrawList,
) -> BuiltFrame<Msg> {
    paint(
        scroll_area::<Msg>(
            "sa",
            100.0,
            ScrollAreaOptions {
                gap: 0.0,
                padding: Edges::ZERO,
                edge_fade: Some(Edges::xy(0.0, 12.0)),
            },
            theme,
            interaction,
            anim,
            (0..8).map(|_| box_().w(160.0).h(40.0).bg(Rgba::WHITE)),
        ),
        (160.0, 100.0),
        1.0,
        interaction,
        text,
        dl,
    )
}

#[test]
fn scroll_area_clips_fades_scrolls_and_morphs_thumb() {
    let mut text = TextEngine::new();
    let mut dl = DrawList::default();
    let mut interaction = InteractionState::default();
    let mut anim = Animator::new();
    let theme = Theme::dark();
    let t0 = Instant::now();

    anim.begin_frame(t0, false);
    let frame = scroll_frame(&interaction, &mut anim, &theme, &mut text, &mut dl);
    let qs = quads(&dl);
    let thumb = qs.last().expect("thumb").2;
    assert!((thumb.dst[2] - 4.0).abs() < EPS, "rest thumb width 4px");
    assert!((thumb.fill[3] - 0.5).abs() < EPS, "rest alpha .5");
    // Children are scissored to the scroll rect and carry the edge fade.
    assert!(
        qs.iter().any(|q| q.0
            == Some(ScissorRect {
                x: 0,
                y: 0,
                w: 160,
                h: 100
            })),
        "scissor"
    );
    assert!(
        qs.iter().any(|q| q.1.is_some_and(|f| f.widths[3] > 0.0)),
        "bottom edge fade on children when overflow exists below"
    );
    assert!(
        qs.iter()
            .all(|q| q.1.is_none_or(|f| f.widths[2].abs() < EPS)),
        "no top edge fade at scroll_y == 0"
    );

    // Wheel scroll registers the offset.
    frame.dispatch_event(
        &InputEvent::Scroll {
            x: 30.0,
            y: 30.0,
            delta_y: 60.0,
        },
        &mut interaction,
    );
    assert!((interaction.scroll_offset(NodeKey::named("sa")) - 60.0).abs() < EPS);

    // Hover inside → scroll_hovered → thumb springs toward 10px/@.75.
    frame.dispatch_event(
        &InputEvent::PointerMove { x: 30.0, y: 30.0 },
        &mut interaction,
    );
    assert!(interaction.is_scroll_hovered(NodeKey::named("sa")));

    // The retarget frame itself still reports the start value; the next
    // frame samples the spring mid-flight. Scrolling down also activates top fade.
    anim.begin_frame(t0 + Duration::from_millis(16), false);
    scroll_frame(&interaction, &mut anim, &theme, &mut text, &mut dl);
    let scrolled_qs = quads(&dl);
    assert!(
        scrolled_qs
            .iter()
            .any(|q| q.1.is_some_and(|f| f.widths[2] > 0.0)),
        "top edge fade activates once scrolled down"
    );
    anim.begin_frame(t0 + Duration::from_millis(64), false);
    scroll_frame(&interaction, &mut anim, &theme, &mut text, &mut dl);
    let w_mid = quads(&dl).last().unwrap().2.dst[2];
    assert!(
        w_mid > 4.0 && w_mid < 10.0,
        "thumb widening mid-flight: {w_mid}"
    );

    anim.begin_frame(t0 + Duration::from_millis(600), false);
    scroll_frame(&interaction, &mut anim, &theme, &mut text, &mut dl);
    let thumb2 = quads(&dl).last().unwrap().2;
    assert!(
        (thumb2.dst[2] - 10.0).abs() < 0.6,
        "hover thumb width ~10px: {}",
        thumb2.dst[2]
    );
    assert!(thumb2.fill[3] > 0.6, "hover alpha toward .75");

    // Children keep intrinsic height past the viewport (no flex-shrink squeeze).
    let child_h = quads(&dl)
        .iter()
        .filter(|q| q.2.fill.iter().all(|c| *c > 0.99))
        .count();
    assert!(child_h >= 8, "8 white children drawn (clipped)");
}

#[test]
fn scroll_area_thumb_respects_reduce_motion() {
    let mut text = TextEngine::new();
    let mut dl = DrawList::default();
    let mut interaction = InteractionState {
        scroll_hovered: Some(NodeKey::named("sa")),
        ..Default::default()
    };
    let mut anim = Animator::new();
    let theme = Theme::dark();
    let t0 = Instant::now();

    // Register keys at rest first (scroll_hovered was preset for clarity).
    interaction.scroll_hovered = None;
    anim.begin_frame(t0, true);
    scroll_frame(&interaction, &mut anim, &theme, &mut text, &mut dl);
    assert!((quads(&dl).last().unwrap().2.dst[2] - 4.0).abs() < EPS);

    // Hover under reduce_motion: 120ms fade completes deterministically.
    interaction.scroll_hovered = Some(NodeKey::named("sa"));
    anim.begin_frame(t0 + Duration::from_millis(200), true);
    scroll_frame(&interaction, &mut anim, &theme, &mut text, &mut dl);
    anim.begin_frame(t0 + Duration::from_millis(400), true);
    scroll_frame(&interaction, &mut anim, &theme, &mut text, &mut dl);
    let w = quads(&dl).last().unwrap().2.dst[2];
    assert!(
        (w - 10.0).abs() < EPS,
        "reduce_motion fade reaches 10px: {w}"
    );
}
