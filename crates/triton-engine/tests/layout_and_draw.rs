//! Headless integration tests for Taffy layout, `DrawList` / `QuadInstance`
//! emission, clipping/scissoring, subtree transforms, hit-testing, and
//! compositor blur/input region collection.
//!
//! Runs without Wayland, EGL, or a GLES3 context.

use triton_design::material::Material;
use triton_engine::gpu::{
    DrawBatch, DrawList, EdgeFade, KIND_FILL, KIND_SHADOW, KIND_TOP_HIGHLIGHT, KIND_WAVE,
    QuadInstance, ScissorRect,
};
use triton_engine::style::{DropShadow, Edges, FontSpec, Rect, Rgba, SubtreeTransform, Theme};
use triton_engine::text::TextEngine;
use triton_engine::ui::{
    Element, InputEvent, InteractionState, NodeKey, box_, col, layout_and_paint, row, text,
};
use triton_engine::widgets::{panel, scroll_area};

const EPS: f32 = 1e-3;

fn assert_approx(actual: f32, expected: f32, label: &str) {
    assert!(
        (actual - expected).abs() <= EPS,
        "{label}: expected {expected}, got {actual}"
    );
}

fn assert_rect_approx(actual: [f32; 4], expected: [f32; 4], label: &str) {
    for i in 0..4 {
        assert!(
            (actual[i] - expected[i]).abs() <= EPS,
            "{label}[{i}]: expected {}, got {} (full actual={actual:?}, expected={expected:?})",
            expected[i],
            actual[i]
        );
    }
}

fn collect_quads(draw_list: &DrawList) -> Vec<(Option<ScissorRect>, QuadInstance)> {
    let mut out = Vec::new();
    for batch in &draw_list.batches {
        if let DrawBatch::Quads {
            scissor, instances, ..
        } = batch
        {
            for &q in instances {
                out.push((*scissor, q));
            }
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Msg {
    Click(&'static str),
}

// ── 1. Row/Col Taffy layout coordinates and physical-scale conversion ────────

#[test]
fn row_and_col_layout_scales_from_logical_to_physical_pixels() {
    let build_tree = || -> Element<()> {
        col()
            .w(240.0)
            .h(120.0)
            .p(10.0)
            .gap(8.0)
            .child(
                row()
                    .w(220.0)
                    .h(40.0)
                    .gap(12.0)
                    .child(box_().w(60.0).h(40.0).bg(Rgba::new(1.0, 0.0, 0.0, 1.0)))
                    .child(box_().w(80.0).h(40.0).bg(Rgba::new(0.0, 1.0, 0.0, 1.0))),
            )
            .child(box_().w(220.0).h(50.0).bg(Rgba::new(0.0, 0.0, 1.0, 1.0)))
    };

    // Expected logical rectangles:
    //   child 0 (row item 0): x=10, y=10, w=60, h=40
    //   child 1 (row item 1): x=10+60+12=82, y=10, w=80, h=40
    //   child 2 (col item 1): x=10, y=10+40+8=58, w=220, h=50
    let expected_logical = [
        [10.0, 10.0, 60.0, 40.0],
        [82.0, 10.0, 80.0, 40.0],
        [10.0, 58.0, 220.0, 50.0],
    ];

    for scale in [1.0_f32, 1.75_f32] {
        let mut text = TextEngine::new();
        let mut draw_list = DrawList::default();
        let interaction = InteractionState::default();

        let frame = layout_and_paint(
            build_tree(),
            (Some(240.0), Some(120.0)),
            scale,
            &interaction,
            &mut text,
            &mut draw_list,
        );

        assert_approx(frame.content_size.0, 240.0, "content_size.w");
        assert_approx(frame.content_size.1, 120.0, "content_size.h");

        let quads = collect_quads(&draw_list);
        assert_eq!(quads.len(), 3, "three filled leaf quads at scale {scale}");

        for (i, &(_, quad)) in quads.iter().enumerate() {
            let exp = expected_logical[i].map(|v| v * scale);
            assert_rect_approx(quad.dst, exp, &format!("quad[{i}] at scale {scale}"));
            assert_approx(quad.misc[0], KIND_FILL, "quad kind");
            assert_approx(quad.misc[3], 1.0, "quad opacity");
        }
    }
}

// ── 2. DropShadow quad emission (kind 2) and bounding expansion ──────────────

#[test]
fn drop_shadow_emits_kind_2_quad_with_expanded_bounds_before_fill() {
    let mut text = TextEngine::new();
    let mut draw_list = DrawList::default();
    let interaction = InteractionState::default();

    let shadow = DropShadow {
        offset_x: 4.0,
        offset_y: 8.0,
        blur: 12.0,
        spread: 2.0,
        color: Rgba::new(0.0, 0.0, 0.0, 0.35),
    };
    let scale = 1.5_f32;

    let root: Element<()> = col().w(200.0).h(200.0).p(30.0).child(
        box_()
            .w(100.0)
            .h(60.0)
            .rounded(10.0)
            .bg(Rgba::WHITE)
            .shadow(shadow),
    );

    layout_and_paint(
        root,
        (Some(200.0), Some(200.0)),
        scale,
        &interaction,
        &mut text,
        &mut draw_list,
    );

    let quads = collect_quads(&draw_list);
    assert_eq!(quads.len(), 2, "shadow quad followed by fill quad");

    let (shadow_sc, shadow_q) = quads[0];
    let (fill_sc, fill_q) = quads[1];
    assert_eq!(shadow_sc, None);
    assert_eq!(fill_sc, None);

    // Shadow quad must be emitted behind fill with KIND_SHADOW (2.0).
    assert_approx(shadow_q.misc[0], KIND_SHADOW, "shadow kind");
    assert_approx(shadow_q.misc[2], shadow.blur * scale, "shadow blur px");
    assert_approx(shadow_q.misc[3], 1.0, "shadow opacity");
    assert_approx(shadow_q.fill[3], 0.35, "shadow alpha");

    // Bounding expansion: pad = blur * 2 = 24.0; spread = 2.0.
    // Logical box is (30, 30, 100, 60).
    // sx = 30 + 4 - 2 - 24 = 8; sy = 30 + 8 - 2 - 24 = 12;
    // sw = 100 + (2 + 24) * 2 = 152; sh = 60 + (2 + 24) * 2 = 112.
    assert_rect_approx(
        shadow_q.dst,
        [8.0 * scale, 12.0 * scale, 152.0 * scale, 112.0 * scale],
        "shadow expanded dst",
    );
    // Radii expand by spread and scale to physical px: (10 + 2) * 1.5 = 18.
    assert_rect_approx(shadow_q.radii, [18.0; 4], "shadow expanded radii");

    // Fill quad sits on top at the exact unexpanded box.
    assert_approx(fill_q.misc[0], KIND_FILL, "fill kind");
    assert_rect_approx(
        fill_q.dst,
        [30.0 * scale, 30.0 * scale, 100.0 * scale, 60.0 * scale],
        "fill dst",
    );
}

// ── 3. SubtreeTransform (translate + scale + opacity) propagation ────────────

#[test]
fn subtree_transform_propagates_translate_scale_and_opacity_to_children() {
    let mut text = TextEngine::new();
    let mut draw_list = DrawList::default();
    let interaction = InteractionState::default();

    let parent_tf = SubtreeTransform {
        translate_x: 10.0,
        translate_y: -4.0,
        scale: 0.5,
        opacity: 0.8,
    };
    let child_tf = SubtreeTransform {
        translate_x: 6.0,
        translate_y: 2.0,
        scale: 1.0,
        opacity: 0.5,
    };

    // Parent is (0, 0, 100, 80), center = (50, 40).
    // Parent composed about (50, 40):
    //   s = 0.5, tx = 50*(1-0.5) + 10 = 35, ty = 40*(1-0.5) - 4 = 16, opacity = 0.8
    // Parent screen_rect = (35, 16, 50, 40).
    // Child local rect (with padding 10) = (10, 10, 40, 20).
    // Child composed inside parent (child.scale = 1.0):
    //   s = 0.5, tx = 35 + 0.5*6 = 38, ty = 16 + 0.5*2 = 17, opacity = 0.8*0.5 = 0.4
    // Child screen_rect = (38 + 10*0.5, 17 + 10*0.5, 40*0.5, 20*0.5) = (43, 22, 20, 10).
    let root: Element<()> = col()
        .w(100.0)
        .h(80.0)
        .p(10.0)
        .bg(Rgba::WHITE)
        .rounded(8.0)
        .transform(parent_tf)
        .child(
            box_()
                .w(40.0)
                .h(20.0)
                .bg(Rgba::BLACK)
                .rounded(4.0)
                .transform(child_tf),
        );

    let scale = 2.0_f32;
    layout_and_paint(
        root,
        (Some(100.0), Some(80.0)),
        scale,
        &interaction,
        &mut text,
        &mut draw_list,
    );

    let quads = collect_quads(&draw_list);
    assert_eq!(quads.len(), 2, "parent + child quads");

    let (_, parent_q) = quads[0];
    let (_, child_q) = quads[1];

    assert_rect_approx(
        parent_q.dst,
        [35.0 * scale, 16.0 * scale, 50.0 * scale, 40.0 * scale],
        "parent transformed dst",
    );
    assert_approx(parent_q.misc[3], 0.8, "parent opacity");
    // Radii scale by scale * tf.scale = 2.0 * 0.5 = 1.0.
    assert_rect_approx(parent_q.radii, [8.0; 4], "parent scaled radii");

    assert_rect_approx(
        child_q.dst,
        [43.0 * scale, 22.0 * scale, 20.0 * scale, 10.0 * scale],
        "child transformed dst",
    );
    assert_approx(child_q.misc[3], 0.4, "child compounded opacity");
    assert_rect_approx(child_q.radii, [4.0; 4], "child scaled radii");
}

#[test]
fn zero_opacity_subtree_culls_parent_and_children() {
    let mut text = TextEngine::new();
    let mut draw_list = DrawList::default();
    let interaction = InteractionState::default();

    let root: Element<()> = col()
        .w(100.0)
        .h(100.0)
        .bg(Rgba::WHITE)
        .opacity(0.0)
        .child(box_().w(50.0).h(50.0).bg(Rgba::BLACK));

    let frame = layout_and_paint(
        root,
        (Some(100.0), Some(100.0)),
        1.0,
        &interaction,
        &mut text,
        &mut draw_list,
    );

    assert_eq!(draw_list.quad_count(), 0);
    assert_eq!(frame.blur_rects, Vec::new());
    assert_eq!(frame.input_rects, Vec::new());
}

// ── 4. Clipping / ScrollArea scissor rect emission ───────────────────────────

#[test]
fn scroll_area_emits_scissor_rect_and_offsets_children_by_scroll() {
    let mut text = TextEngine::new();
    let mut draw_list = DrawList::default();
    let mut interaction = InteractionState::default();
    let scroller_key = NodeKey::named("list");
    interaction.set_scroll_offset(scroller_key, 25.0);

    let scale = 1.5_f32;
    let root: Element<()> = col().w(200.0).h(140.0).p(20.0).child(
        scroll_area(
            scroller_key,
            100.0,
            0.0,
            0.0,
            (0..4).map(|_| box_().w(160.0).h(40.0).bg(Rgba::WHITE)),
        )
        .w(160.0)
        .bg(Rgba::new(0.1, 0.1, 0.1, 1.0)),
    );

    layout_and_paint(
        root,
        (Some(200.0), Some(140.0)),
        scale,
        &interaction,
        &mut text,
        &mut draw_list,
    );

    let quads = collect_quads(&draw_list);
    // 1 background quad for scroll_area (unclipped) + 4 child quads + 1 scrollbar thumb quad.
    assert_eq!(quads.len(), 6);

    // Container background itself is outside its own clip.
    assert_eq!(quads[0].0, None);

    // Expected scissor for scroll_area logical rect (20, 20, 160, 100) at 1.5x:
    // physical (30, 30, 240, 150).
    let expected_scissor = ScissorRect {
        x: 30,
        y: 30,
        w: 240,
        h: 150,
    };

    for (idx, &(scissor, quad)) in quads[1..5].iter().enumerate() {
        assert_eq!(scissor, Some(expected_scissor), "child {idx} scissor");
        // Child logical y = 20 - 25 (scroll) + idx * 40
        let expected_y = (20.0 - 25.0 + idx as f32 * 40.0) * scale;
        assert_approx(quad.dst[1], expected_y, &format!("child {idx} scrolled y"));
    }

    // Overflowing scroll area also emits a scrollbar thumb clipped to the same scissor.
    assert_eq!(
        quads[5].0,
        Some(expected_scissor),
        "scrollbar thumb scissor"
    );
}

#[test]
fn nested_clip_intersects_parent_and_child_scissors() {
    let mut text = TextEngine::new();
    let mut draw_list = DrawList::default();
    let interaction = InteractionState::default();

    let root: Element<()> = col().w(200.0).h(200.0).p(10.0).clip().child(
        col()
            .w(100.0)
            .h(100.0)
            .absolute()
            .inset_xy(150.0, 50.0)
            .clip()
            .child(box_().w(80.0).h(80.0).bg(Rgba::WHITE)),
    );

    layout_and_paint(
        root,
        (Some(200.0), Some(200.0)),
        1.0,
        &interaction,
        &mut text,
        &mut draw_list,
    );

    let quads = collect_quads(&draw_list);
    assert_eq!(quads.len(), 1);
    // Outer clip: (0, 0, 200, 200). Inner clip: (150, 50, 100, 100).
    // Intersection: (150, 50, 50, 100).
    assert_eq!(
        quads[0].0,
        Some(ScissorRect {
            x: 150,
            y: 50,
            w: 50,
            h: 100,
        })
    );
}

// ── 5. Hit-testing: hover, press, click, leave, and visual state fills ───────

#[test]
fn hit_testing_hover_press_state_transitions_and_state_fills() {
    let mut text = TextEngine::new();
    let mut draw_list = DrawList::default();
    let mut interaction = InteractionState::default();

    let idle_col = Rgba::new(0.2, 0.2, 0.2, 1.0);
    let hover_col = Rgba::new(0.4, 0.4, 0.4, 1.0);
    let press_col = Rgba::new(0.8, 0.2, 0.2, 1.0);
    let btn_key = NodeKey::named("action");

    let build = || -> Element<Msg> {
        col().w(200.0).h(100.0).p(20.0).child(
            box_()
                .id(btn_key)
                .w(100.0)
                .h(40.0)
                .bg(idle_col)
                .hover_bg(hover_col)
                .press_bg(press_col)
                .on_click(|| Msg::Click("action")),
        )
    };

    // Initial idle render.
    let frame = layout_and_paint(
        build(),
        (Some(200.0), Some(100.0)),
        1.0,
        &interaction,
        &mut text,
        &mut draw_list,
    );
    assert_eq!(collect_quads(&draw_list)[0].1.fill, idle_col.to_array());
    assert!(!interaction.is_hovered(btn_key));
    assert!(!interaction.is_pressed(btn_key));

    // Move inside button (rect is 20..120, 20..60) -> hovered.
    let (msgs, dirty) = frame.dispatch_event(
        &InputEvent::PointerMove { x: 50.0, y: 35.0 },
        &mut interaction,
    );
    assert!(msgs.is_empty() && dirty);
    assert!(interaction.is_hovered(btn_key));
    assert!(!interaction.is_pressed(btn_key));

    // Repaint reflects hover_bg.
    let frame = layout_and_paint(
        build(),
        (Some(200.0), Some(100.0)),
        1.0,
        &interaction,
        &mut text,
        &mut draw_list,
    );
    assert_eq!(collect_quads(&draw_list)[0].1.fill, hover_col.to_array());

    // Press inside button -> pressed, no click emitted yet.
    let (msgs, dirty) = frame.dispatch_event(
        &InputEvent::PointerDown {
            x: 50.0,
            y: 35.0,
            button: 272,
        },
        &mut interaction,
    );
    assert!(msgs.is_empty() && dirty);
    assert!(interaction.is_pressed(btn_key));

    // Repaint reflects press_bg.
    let frame = layout_and_paint(
        build(),
        (Some(200.0), Some(100.0)),
        1.0,
        &interaction,
        &mut text,
        &mut draw_list,
    );
    assert_eq!(collect_quads(&draw_list)[0].1.fill, press_col.to_array());

    // Release outside button -> clears pressed, does NOT trigger click.
    let (msgs, dirty) = frame.dispatch_event(
        &InputEvent::PointerUp {
            x: 180.0,
            y: 90.0,
            button: 272,
        },
        &mut interaction,
    );
    assert!(msgs.is_empty() && dirty);
    assert!(!interaction.is_pressed(btn_key));

    // Press and release inside button -> triggers click.
    let _ = frame.dispatch_event(
        &InputEvent::PointerDown {
            x: 50.0,
            y: 35.0,
            button: 272,
        },
        &mut interaction,
    );
    let (msgs, dirty) = frame.dispatch_event(
        &InputEvent::PointerUp {
            x: 50.0,
            y: 35.0,
            button: 272,
        },
        &mut interaction,
    );
    assert!(dirty);
    assert_eq!(msgs, vec![Msg::Click("action")]);
    assert!(!interaction.is_pressed(btn_key));

    // PointerLeave clears hover.
    let (msgs, dirty) = frame.dispatch_event(&InputEvent::PointerLeave, &mut interaction);
    assert!(msgs.is_empty() && dirty);
    assert!(!interaction.is_hovered(btn_key));
    assert_eq!(interaction.pointer_pos, None);
}

// ── 6. blur_rects and input_rects collection for Glass / Thick / Solid ───────

#[test]
fn glass_and_thick_panels_collect_blur_and_input_rects_while_solid_only_collects_input() {
    let mut text = TextEngine::new();
    let mut draw_list = DrawList::default();
    let interaction = InteractionState::default();
    let theme = Theme::dark();

    let root: Element<()> = row()
        .w(400.0)
        .h(120.0)
        .p(12.0)
        .gap(16.0)
        .child(panel(Material::Glass, 20.0, &theme).w(100.0).h(80.0))
        .child(panel(Material::Thick, 24.0, &theme).w(120.0).h(80.0))
        .child(panel(Material::Solid, 12.0, &theme).w(80.0).h(80.0));

    let frame = layout_and_paint(
        root,
        (Some(400.0), Some(120.0)),
        1.75,
        &interaction,
        &mut text,
        &mut draw_list,
    );

    // Only Glass and Thick request compositor blur (Solid has no blur spec).
    assert_eq!(frame.blur_rects.len(), 2);
    assert_eq!(frame.blur_rects[0].rect, Rect::new(12.0, 12.0, 100.0, 80.0));
    assert_approx(frame.blur_rects[0].radius, 20.0, "glass blur radius");

    assert_eq!(
        frame.blur_rects[1].rect,
        Rect::new(128.0, 12.0, 120.0, 80.0)
    );
    assert_approx(frame.blur_rects[1].radius, 24.0, "thick blur radius");

    // All three panels capture Wayland input regions (logical surface px).
    assert_eq!(frame.input_rects.len(), 3);
    assert_eq!(
        frame.input_rects[2].rect,
        Rect::new(264.0, 12.0, 80.0, 80.0)
    );
    assert_approx(frame.input_rects[2].radius, 12.0, "solid input radius");

    // Glass and Thick also emit KIND_TOP_HIGHLIGHT quads; Solid emits KIND_FILL.
    let quads = collect_quads(&draw_list);
    let highlight_count = quads
        .iter()
        .filter(|(_, q)| (q.misc[0] - KIND_TOP_HIGHLIGHT).abs() < EPS)
        .count();
    let shadow_count = quads
        .iter()
        .filter(|(_, q)| (q.misc[0] - KIND_SHADOW).abs() < EPS)
        .count();
    let fill_count = quads
        .iter()
        .filter(|(_, q)| (q.misc[0] - KIND_FILL).abs() < EPS)
        .count();

    assert_eq!(highlight_count, 2, "Glass + Thick top highlight quads");
    assert_eq!(shadow_count, 2, "Glass + Thick floating shadows");
    assert_eq!(fill_count, 1, "Solid fill quad");
}

// ── 7. DropShadow cutout flag and gutter() calculation ───────────────────────

#[test]
fn drop_shadow_cutout_flag_and_gutter_margins() {
    let shadow = DropShadow {
        offset_x: 2.0,
        offset_y: 8.0,
        blur: 14.0,
        spread: 1.0,
        color: Rgba::new(0.0, 0.0, 0.0, 0.35),
    };

    // Gutter covers blur*2 + spread plus directional offsets, never negative.
    let g = shadow.gutter();
    // pad = 14*2 + 1 = 29; top = 29-8 = 21, right = 29+2 = 31, bottom = 29+8 = 37, left = 29-2 = 27
    assert_approx(g.top, 21.0, "gutter top");
    assert_approx(g.right, 31.0, "gutter right");
    assert_approx(g.bottom, 37.0, "gutter bottom");
    assert_approx(g.left, 27.0, "gutter left");
    assert!(g.top >= 0.0 && g.right >= 0.0 && g.bottom >= 0.0 && g.left >= 0.0);

    // Extreme offset clamps opposite side to 0.0 rather than going negative.
    let extreme = DropShadow {
        offset_x: 100.0,
        offset_y: -100.0,
        blur: 4.0,
        spread: 0.0,
        color: Rgba::new(0.0, 0.0, 0.0, 0.5),
    };
    let eg = extreme.gutter();
    assert_approx(eg.left, 0.0, "extreme gutter left clamped");
    assert_approx(eg.bottom, 0.0, "extreme gutter bottom clamped");
    assert!(eg.right >= 108.0 && eg.top >= 108.0);

    // Translucent fill (a < 0.99) enables interior shadow cutout (border[3] == 1.0);
    // opaque fill (a == 1.0) disables cutout (border[3] == 0.0).
    let mut text = TextEngine::new();
    let mut draw_list = DrawList::default();
    let interaction = InteractionState::default();

    let root: Element<()> = row()
        .w(300.0)
        .h(120.0)
        .p(20.0)
        .gap(20.0)
        .child(
            box_()
                .w(80.0)
                .h(60.0)
                .rounded(12.0)
                .bg(Rgba::new(0.2, 0.2, 0.2, 0.72))
                .shadow(shadow),
        )
        .child(
            box_()
                .w(80.0)
                .h(60.0)
                .rounded(12.0)
                .bg(Rgba::new(0.2, 0.2, 0.2, 1.0))
                .shadow(shadow),
        );

    layout_and_paint(
        root,
        (Some(300.0), Some(120.0)),
        1.0,
        &interaction,
        &mut text,
        &mut draw_list,
    );

    let shadow_quads: Vec<QuadInstance> = collect_quads(&draw_list)
        .into_iter()
        .map(|(_, q)| q)
        .filter(|q| (q.misc[0] - KIND_SHADOW).abs() < EPS)
        .collect();
    assert_eq!(shadow_quads.len(), 2, "two shadow quads emitted");
    assert_approx(
        shadow_quads[0].border[3],
        1.0,
        "translucent fill enables cutout",
    );
    assert_approx(
        shadow_quads[1].border[3],
        0.0,
        "opaque fill disables cutout",
    );
}

// ── 8. Wave element emits KIND_WAVE quad ─────────────────────────────────────

#[test]
fn wave_element_emits_kind_wave_quad() {
    let mut text_engine = TextEngine::new();
    let mut draw_list = DrawList::default();
    let interaction = InteractionState::default();

    let root: Element<()> = row()
        .w(200.0)
        .h(100.0)
        .gap(16.0)
        .child(box_().w(48.0).h(48.0).bg(Rgba::WHITE).wave())
        .child(
            box_()
                .w(48.0)
                .h(48.0)
                .bg(Rgba::new(0.2, 0.5, 1.0, 1.0))
                .wave_animated(1.25, 0.04),
        );

    layout_and_paint(
        root,
        (Some(200.0), Some(100.0)),
        2.0,
        &interaction,
        &mut text_engine,
        &mut draw_list,
    );

    let quads = collect_quads(&draw_list);
    assert_eq!(quads.len(), 2, "two wave quads emitted");

    let (_, default_wave) = quads[0];
    assert_approx(default_wave.misc[0], KIND_WAVE, "default wave kind");
    assert_rect_approx(
        default_wave.radii,
        [0.0, 0.075, 9.0, 0.0],
        "default wave params",
    );

    let (_, anim_wave) = quads[1];
    assert_approx(anim_wave.misc[0], KIND_WAVE, "animated wave kind");
    assert_rect_approx(
        anim_wave.radii,
        [1.25, 0.04, 9.0, 0.0],
        "animated wave params",
    );
}

// ── 9. EdgeFade propagates to child quads and glyphs ─────────────────────────

#[test]
fn edge_fade_propagates_to_child_quads_and_glyphs() {
    let mut text_engine = TextEngine::new();
    let mut draw_list = DrawList::default();
    let interaction = InteractionState::default();

    let scale = 1.5_f32;
    let fade_edges = Edges {
        top: 6.0,
        right: 12.0,
        bottom: 8.0,
        left: 10.0,
    };

    let root: Element<()> = col().w(240.0).h(120.0).p(20.0).child(
        col()
            .w(200.0)
            .h(80.0)
            .bg(Rgba::new(0.1, 0.1, 0.1, 1.0))
            .edge_fade(fade_edges)
            .child(box_().w(100.0).h(20.0).bg(Rgba::WHITE))
            .child(text("Hello", FontSpec::ui(14.0), Rgba::WHITE)),
    );

    layout_and_paint(
        root,
        (Some(240.0), Some(120.0)),
        scale,
        &interaction,
        &mut text_engine,
        &mut draw_list,
    );

    let expected_fade = EdgeFade {
        rect: [20.0 * scale, 20.0 * scale, 200.0 * scale, 80.0 * scale],
        widths: [
            fade_edges.left * scale,
            fade_edges.right * scale,
            fade_edges.top * scale,
            fade_edges.bottom * scale,
        ],
    };

    let mut quad_batches = Vec::new();
    let mut glyph_batches = Vec::new();
    for batch in &draw_list.batches {
        match batch {
            DrawBatch::Quads {
                edge_fade,
                instances,
                ..
            } => quad_batches.push((*edge_fade, instances.len())),
            DrawBatch::Glyphs {
                edge_fade,
                instances,
                ..
            } => glyph_batches.push((*edge_fade, instances.len())),
        }
    }

    // First quad batch is the parent container itself (no edge_fade),
    // second quad batch is the child box (with expected_fade).
    assert_eq!(
        quad_batches.len(),
        2,
        "parent and child should be in separate quad batches"
    );
    assert_eq!(quad_batches[0], (None, 1));
    assert_eq!(quad_batches[1], (Some(expected_fade), 1));

    // Glyph batch for the child text inherits the same edge_fade.
    assert_eq!(glyph_batches.len(), 1, "one faded glyph batch");
    assert_eq!(glyph_batches[0].0, Some(expected_fade));
    assert!(glyph_batches[0].1 > 0, "glyphs emitted for 'Hello'");
}
