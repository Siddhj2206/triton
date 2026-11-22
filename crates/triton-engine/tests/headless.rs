//! Integration tests for `triton-engine`'s Rune-style headless dev loop:
//! - Phase 1: semantic `NodeInfo` tree recording and `BuiltFrame::tree_json()`
//! - Phase 2: `HeadlessSession`, key-addressed input, and JSON `Script` runner
//! - Phase 3: `Egl::headless()`, `Gpu::render_to_rgba()`, `HeadlessRenderer`,
//!   and zero-dependency PNG encoding (with real GPU pixel-level assertions).

use std::time::{Duration, Instant};

use triton_design::material::Material;
use triton_design::shape::Point;
use triton_engine::{
    AppCommand, DropShadow, Edges, Element, FontSpec, HeadlessRenderer, HeadlessSession, ImageData,
    ImageStore, ImageStyle, InputEvent, Key, NodeKey, NodeKind, Rect, Rgba, Script, ShellApp,
    Spring, SubtreeTransform, SurfaceSize, SurfaceSpec, Theme, ViewContext, box_, col, encode_png,
    image, polyline, row, text, write_png,
};

#[derive(Debug, Clone, PartialEq)]
enum Msg {
    Tap(&'static str),
    Drag(i32),
    KeyPressed(Key),
    Typed(String),
}

// ── Phase 1: Semantic element tree & tree_json() ─────────────────────────────

#[test]
fn node_info_records_all_kinds_paths_clips_opacity_and_tree_json() {
    let mut session = HeadlessSession::<Msg>::new((320.0, 200.0), 1.5);
    let img_id = session
        .images_mut()
        .insert(ImageData::from_rgba(4, 4, vec![255; 64]).unwrap());

    let frame = session.render(|_ctx| {
        col().id("root").w(320.0).h(200.0).p(10.0).gap(8.0).child(
            col()
                .id("panel")
                .w(200.0)
                .h(120.0)
                .clip()
                .scroll_y()
                .input_opaque()
                .hoverable()
                .focusable()
                .shadow(DropShadow {
                    offset_x: 0.0,
                    offset_y: 8.0,
                    blur: 16.0,
                    spread: 0.0,
                    color: Rgba::new(0.0, 0.0, 0.0, 0.4),
                })
                .child(
                    // Unkeyed wrapper between "panel" and "label" to verify key paths skip unkeyed nodes.
                    box_().child(
                        text("Hello Tree", FontSpec::ui(14.0), Rgba::WHITE)
                            .id("label")
                            .opacity(0.6),
                    ),
                )
                .child(
                    image(img_id, ImageStyle::default())
                        .id("thumb")
                        .size(24.0, 24.0),
                )
                .child(
                    polyline(
                        vec![Point::new(0.0, 0.0), Point::new(20.0, 10.0)],
                        2.0,
                        Rgba::WHITE,
                    )
                    .id("wave")
                    .size(20.0, 10.0),
                ),
        )
    });

    // Every laid-out element is recorded (root, panel, wrapper, label, thumb, wave = 6).
    assert_eq!(frame.nodes.len(), 6);

    let panel_key = NodeKey::named("panel");
    let label_key = NodeKey::named("label");
    let root_key = NodeKey::named("root");

    let panel = frame.node_by_key(panel_key).expect("panel node recorded");
    assert_eq!(panel.kind, NodeKind::Container);
    assert!(panel.flags.scrollable);
    assert!(panel.flags.focusable);
    assert!(panel.flags.hoverable);
    assert!(panel.flags.input_opaque);
    assert!(panel.flags.has_shadow);

    let label = frame
        .node_by_path(&[panel_key, label_key])
        .expect("label found via panel/label path");
    assert_eq!(label.path, vec![root_key, panel_key, label_key]);
    assert_eq!(label.kind, NodeKind::Text("Hello Tree".to_owned()));
    assert!((label.opacity - 0.6).abs() < 1e-4);
    assert!(
        label.clip.is_some(),
        "clipped child should record clip rect"
    );

    let thumb = frame.node_by_key(NodeKey::named("thumb")).unwrap();
    assert_eq!(thumb.kind, NodeKind::Image);
    let wave = frame.node_by_key(NodeKey::named("wave")).unwrap();
    assert_eq!(wave.kind, NodeKind::Polyline);

    // JSON output parses cleanly and preserves content_size, nodes, and text.
    let json = frame.tree_json();
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid tree JSON");
    assert_eq!(parsed["content_size"][0].as_f64(), Some(320.0));
    assert_eq!(parsed["content_size"][1].as_f64(), Some(200.0));
    let nodes = parsed["nodes"].as_array().expect("nodes array");
    assert_eq!(nodes.len(), 6);
    assert!(
        nodes
            .iter()
            .any(|n| n["kind"]["text"].as_str() == Some("Hello Tree")),
        "tree_json should include text node content"
    );
}

#[test]
fn node_info_remaps_coords_and_opacity_across_cached_layer() {
    let mut session = HeadlessSession::<()>::new((200.0, 200.0), 1.0);
    let frame = session.render(|_ctx| {
        box_().w(200.0).h(200.0).p(20.0).child(
            box_()
                .id("cached")
                .layer("cached-layer")
                .w(100.0)
                .h(60.0)
                .transform(SubtreeTransform {
                    translate_x: 10.0,
                    translate_y: 5.0,
                    scale: 1.0,
                    opacity: 0.5,
                })
                .child(box_().id("inner").w(40.0).h(20.0).bg(Rgba::WHITE)),
        )
    });

    let inner = frame
        .node_by_path(&[NodeKey::named("cached"), NodeKey::named("inner")])
        .expect("inner node remapped out of layer_key");
    assert!((inner.rect.x - 30.0).abs() < 0.5, "got {:?}", inner.rect);
    assert!((inner.rect.y - 25.0).abs() < 0.5, "got {:?}", inner.rect);
    assert!((inner.opacity - 0.5).abs() < 1e-4);
}

// ── Phase 2: HeadlessSession, key-addressed input & JSON Script runner ───────

#[test]
fn headless_session_key_input_helpers_and_virtual_clock() {
    let mut session = HeadlessSession::<Msg>::new((300.0, 240.0), 1.0);
    let mut tapped = false;

    let build = |tapped: bool| {
        move |ctx: &mut ViewContext<'_>| -> Element<Msg> {
            let w = ctx.anim.spring_px(
                "morph-pill",
                if tapped { 180.0 } else { 80.0 },
                Spring::SPATIAL,
            );
            col()
                .w(300.0)
                .h(240.0)
                .p(10.0)
                .gap(8.0)
                .child(
                    box_()
                        .id("morph-pill")
                        .w(w)
                        .h(36.0)
                        .bg(Rgba::WHITE)
                        .on_click(|| Msg::Tap("pill")),
                )
                .child(
                    box_()
                        .id("slider")
                        .w(200.0)
                        .h(28.0)
                        .bg(Rgba::BLACK)
                        .on_drag(|x, _y, r| {
                            let pct = ((x - r.x) / r.w).clamp(0.0, 1.0);
                            Msg::Drag((pct * 100.0).round() as i32)
                        }),
                )
                .child(
                    col()
                        .id("scroller")
                        .w(200.0)
                        .h(60.0)
                        .scroll_y()
                        .children((0..5).map(|i| {
                            box_()
                                .id(NodeKey::from_pair("item", i))
                                .w(200.0)
                                .h(30.0)
                                .bg(Rgba::WHITE)
                        })),
                )
                .child(
                    box_()
                        .id("field")
                        .w(200.0)
                        .h(30.0)
                        .focusable()
                        .on_key(|ev| match ev {
                            InputEvent::KeyDown {
                                key, utf8: Some(u), ..
                            } => Some(Msg::Typed(format!("{key:?}:{u}"))),
                            InputEvent::KeyDown { key, .. } => Some(Msg::KeyPressed(*key)),
                            _ => None,
                        }),
                )
        }
    };

    session.render(build(tapped));
    assert!((session.hit_rect("morph-pill").unwrap().w - 80.0).abs() < 0.5);

    // Hover & tap by key
    session.hover_key("morph-pill").unwrap();
    assert!(
        session
            .interaction()
            .is_hovered(NodeKey::named("morph-pill"))
    );
    let msgs = session.tap_key("morph-pill").unwrap();
    assert_eq!(msgs, vec![Msg::Tap("pill")]);
    tapped = true;

    // Advance virtual clock — spring animates toward 180px deterministically.
    session.render(build(tapped));
    assert!(session.is_animating());
    session.advance(Duration::from_millis(80));
    session.render(build(tapped));
    let mid_w = session.hit_rect("morph-pill").unwrap().w;
    assert!(mid_w > 90.0, "expected animated width > 90, got {mid_w}");

    session.advance(Duration::from_secs(2));
    session.render(build(tapped));
    assert!((session.hit_rect("morph-pill").unwrap().w - 180.0).abs() < 1.0);
    assert!(!session.is_animating());

    // Drag by key
    let drag_msgs = session.drag_key("slider", (0.2, 0.5), (0.8, 0.5)).unwrap();
    assert_eq!(drag_msgs, vec![Msg::Drag(20), Msg::Drag(80)]);

    // Scroll by key
    session.scroll_key("scroller", 45.0).unwrap();
    assert!(
        (session
            .interaction()
            .scroll_offset(NodeKey::named("scroller"))
            - 45.0)
            .abs()
            < 0.5
    );

    // Focus + send_key
    session.tap_key("field").unwrap();
    let typed = session.send_key(Key::Char('a'), Some("a"));
    assert_eq!(typed, vec![Msg::Typed("Char('a'):a".into())]);
    let esc = session.send_key(Key::Escape, None);
    assert_eq!(esc, vec![Msg::KeyPressed(Key::Escape)]);
}

#[test]
fn json_script_runner_executes_key_addressed_events_and_marks() {
    let script = Script::from_json(
        r#"{
            "events": [
                {"t": 0.05, "type": "mark", "name": "start"},
                {"t": 0.10, "type": "tap", "key": "group/action"},
                {"t": 0.25, "type": "scroll", "key": "list", "delta_y": 50.0},
                {"t": 0.40, "type": "key", "key": "Escape"},
                {"t": 0.50, "type": "mark", "name": "end"}
            ]
        }"#,
    )
    .expect("valid script");

    let mut session = HeadlessSession::<Msg>::new((240.0, 200.0), 1.0);
    let mut log = Vec::new();
    let t0 = session.now();

    let outcome = session
        .run_script(
            &script,
            |_ctx| {
                col()
                    .w(240.0)
                    .h(200.0)
                    .gap(8.0)
                    .on_key(|ev| match ev {
                        InputEvent::KeyDown { key, .. } => Some(Msg::KeyPressed(*key)),
                        _ => None,
                    })
                    .child(
                        row().id("group").child(
                            box_()
                                .id("action")
                                .w(100.0)
                                .h(36.0)
                                .bg(Rgba::WHITE)
                                .on_click(|| Msg::Tap("action")),
                        ),
                    )
                    .child(
                        col()
                            .id("list")
                            .w(200.0)
                            .h(60.0)
                            .scroll_y()
                            .children((0..5).map(|_| box_().w(200.0).h(30.0).bg(Rgba::WHITE))),
                    )
            },
            |msg, now| log.push((msg, now.saturating_duration_since(t0))),
        )
        .expect("script execution succeeds");

    assert_eq!(outcome.events_dispatched, 3);
    assert_eq!(outcome.marks.get("start"), Some(&0.05));
    assert_eq!(outcome.marks.get("end"), Some(&0.50));
    assert_eq!(log.len(), 2);
    assert_eq!(log[0].0, Msg::Tap("action"));
    assert_eq!(log[1].0, Msg::KeyPressed(Key::Escape));
    assert!((session.interaction().scroll_offset(NodeKey::named("list")) - 50.0).abs() < 0.5);
}

#[test]
fn headless_session_renders_shell_app_surface_directly() {
    struct MiniApp;
    impl ShellApp for MiniApp {
        type SurfaceId = u8;
        type Message = &'static str;

        fn surfaces(&self) -> Vec<(u8, SurfaceSpec)> {
            vec![]
        }

        fn view(&self, _id: u8, _ctx: &mut ViewContext<'_>) -> Element<&'static str> {
            box_()
                .id("mini-card")
                .w(160.0)
                .h(90.0)
                .bg(Rgba::WHITE)
                .on_click(|| "clicked")
        }

        fn update(&mut self, _id: u8, _msg: &'static str, _now: Instant) -> Vec<AppCommand<u8>> {
            vec![]
        }

        fn is_animating(&self, _id: u8, _now: Instant) -> bool {
            false
        }
    }

    let mut app = MiniApp;
    let mut session = HeadlessSession::new((100.0, 100.0), 1.0);
    session.render_surface(
        &mut app,
        0,
        SurfaceSize::FitContent {
            max_w: Some(400),
            max_h: Some(400),
        },
    );
    assert_eq!(session.size(), (160.0, 90.0));
    assert_eq!(session.tap_key("mini-card").unwrap(), vec!["clicked"]);
}

// ── Phase 3: Offscreen GPU render_to_rgba() + zero-dep PNG encoder ───────────

#[test]
fn offscreen_gpu_renders_sdf_quad_shadow_text_and_valid_png() {
    let mut renderer = HeadlessRenderer::new().expect("headless EGL/GLES3 must initialize");
    let mut session = HeadlessSession::<()>::new((200.0, 140.0), 1.0);

    session.render(|_ctx| {
        box_().w(200.0).h(140.0).p(30.0).child(
            box_()
                .id("card")
                .w(140.0)
                .h(80.0)
                .rounded(16.0)
                .bg(Rgba::new(0.2, 0.5, 0.9, 1.0))
                .shadow(DropShadow {
                    offset_x: 0.0,
                    offset_y: 8.0,
                    blur: 12.0,
                    spread: 0.0,
                    color: Rgba::new(0.0, 0.0, 0.0, 0.6),
                })
                .items_center()
                .justify_center()
                .child(text("HI", FontSpec::ui(24.0).weight(700), Rgba::WHITE)),
        )
    });

    let (w, h, rgba) = renderer
        .render_rgba(&mut session)
        .expect("offscreen render_to_rgba succeeds");
    assert_eq!((w, h), (200, 140));
    assert_eq!(rgba.len(), (w * h * 4) as usize);

    let px = |x: u32, y: u32| -> [u8; 4] {
        let i = ((y * w + x) * 4) as usize;
        [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]
    };

    // 1. Far top-left corner is transparent (outside shadow & card).
    assert_eq!(px(0, 0)[3], 0, "far corner must be transparent");

    // 2. Interior of the blue card (away from text) matches fill ~(51, 128, 230, 255).
    let card_px = px(45, 50);
    assert_eq!(card_px[3], 255, "opaque card interior must have alpha 255");
    assert!(
        card_px[2] > 210 && card_px[0] < 70,
        "expected blue card fill, got {card_px:?}"
    );

    // 3. Rounded corner outside the card's 16px corner arc is NOT solid blue.
    let corner_outside = px(31, 31);
    assert!(
        corner_outside[2] < 150,
        "rounded corner should clip outside arc, got {corner_outside:?}"
    );

    // 4. Below the card (y=118, x=100), the SDF drop shadow produces dark semi-transparent pixels.
    let shadow_px = px(100, 118);
    assert!(
        shadow_px[3] > 10 && shadow_px[3] < 200,
        "expected soft drop shadow below card, got {shadow_px:?}"
    );
    assert!(
        shadow_px[0] < 20 && shadow_px[1] < 20 && shadow_px[2] < 20,
        "expected black shadow color, got {shadow_px:?}"
    );

    // 5. White text "HI" rendered bright pixels near the center of the card.
    let mut bright_glyph_pixels = 0usize;
    for y in 50..90 {
        for x in 70..130 {
            let p = px(x, y);
            if p[0] > 200 && p[1] > 200 && p[2] > 200 && p[3] == 255 {
                bright_glyph_pixels += 1;
            }
        }
    }
    assert!(
        bright_glyph_pixels >= 20,
        "expected crisp white glyph pixels inside card, got {bright_glyph_pixels}"
    );

    // 6. PNG encoding produces valid PNG signature + IHDR + IDAT + IEND chunks.
    let png = encode_png(w, h, &rgba).expect("encode_png succeeds");
    assert_eq!(&png[0..8], &[137, 80, 78, 71, 13, 10, 26, 10]);
    assert_eq!(&png[12..16], b"IHDR");
    assert!(png.windows(4).any(|w| w == b"IDAT"));
    assert!(png.ends_with(&[0, 0, 0, 0, b'I', b'E', b'N', b'D', 0xAE, 0x42, 0x60, 0x82]));

    let tmp_png = std::env::temp_dir().join("triton-engine-headless-test.png");
    write_png(&tmp_png, w, h, &rgba).expect("write_png succeeds");
    let written = std::fs::read(&tmp_png).expect("read back PNG");
    assert_eq!(written, png);
    let _ = std::fs::remove_file(&tmp_png);
}

#[test]
fn headless_session_handles_missing_key_and_bad_script_cleanly() {
    let mut session = HeadlessSession::<()>::new((100.0, 100.0), 1.0);
    session.render(|_| box_().w(100.0).h(100.0));
    assert!(session.tap_key("nonexistent").is_err());
    assert!(Script::from_json("{}").is_err());
    assert!(Script::from_json(r#"{"events":[{"t":0,"type":"bogus"}]}"#).is_err());
    // Ensure ImageStore public import works cleanly too.
    let _store = ImageStore::new();
    let _rect = Rect::new(0.0, 0.0, 1.0, 1.0);
}

// ── Part B: Blur gating/clipping/dedup, scroll-aware edge_fade, layout & shadow ──

#[test]
fn b1_blur_rects_gated_on_opacity_and_clipped_to_viewport() {
    let theme = Theme::dark();
    let mut session = HeadlessSession::<()>::new((300.0, 200.0), 1.0);

    // 1. Mid-fade (opacity 0.5 < 0.85) emits ZERO blur rects (both direct and layer_key).
    let fading = session.render(|_| {
        row()
            .w(300.0)
            .h(200.0)
            .gap(10.0)
            .child(
                box_()
                    .w(120.0)
                    .h(80.0)
                    .material(Material::Thick, &theme)
                    .opacity(0.5),
            )
            .child(
                box_()
                    .w(120.0)
                    .h(80.0)
                    .layer("fading-layer")
                    .material(Material::Glass, &theme)
                    .opacity(0.84),
            )
    });
    assert!(
        fading.blur_rects.is_empty(),
        "fading surfaces (< 0.85 opacity) must not emit compositor blur rects, got {:?}",
        fading.blur_rects
    );

    // 2. Settled (>= 0.85) emits blur rects cleanly.
    let settled = session.render(|_| {
        box_()
            .w(120.0)
            .h(80.0)
            .material(Material::Thick, &theme)
            .opacity(0.9)
    });
    assert_eq!(settled.blur_rects.len(), 1);

    // 3. Clipped child inside a scroll viewport has its blur_rect (and input_rect)
    //    clipped to the viewport; completely scrolled-off children emit no blur_rect.
    let clipped = session.render(|_| {
        col()
            .id("viewport")
            .w(200.0)
            .h(100.0)
            .clip()
            .scroll_y()
            .child(box_().w(200.0).h(80.0))
            .child(box_().w(200.0).h(80.0).material(Material::Glass, &theme))
            .child(box_().w(200.0).h(80.0).material(Material::Thick, &theme))
    });
    assert_eq!(
        clipped.blur_rects.len(),
        1,
        "completely scrolled-off Thick tile must not emit a blur_rect; got {:?}",
        clipped.blur_rects
    );
    let br = clipped.blur_rects[0].rect;
    assert!(
        (br.y - 80.0).abs() < 0.5 && (br.h - 20.0).abs() < 0.5,
        "got {br:?}"
    );
}

#[test]
fn b2_blur_ancestor_deduplicates_nested_materials() {
    let theme = Theme::dark();
    let mut session = HeadlessSession::<()>::new((300.0, 200.0), 1.0);

    let frame = session.render(|_| {
        col()
            .w(300.0)
            .h(200.0)
            .p(16.0)
            .gap(12.0)
            .material(Material::Thick, &theme)
            .child(box_().w(120.0).h(60.0).material(Material::Glass, &theme))
            .child(
                box_()
                    .w(120.0)
                    .h(60.0)
                    .layer("nested-cached")
                    .material(Material::Mica, &theme),
            )
    });

    assert_eq!(
        frame.blur_rects.len(),
        1,
        "nested blurred panels inside a blurred ancestor must deduplicate to 1 blur rect, got {:?}",
        frame.blur_rects
    );
    let r = frame.blur_rects[0].rect;
    assert!(
        (r.w - 300.0).abs() < 0.5 && (r.h - 200.0).abs() < 0.5,
        "got {r:?}"
    );
}

#[test]
fn l1_scroll_aware_and_nested_edge_fade() {
    let mut session = HeadlessSession::<()>::new((240.0, 120.0), 1.0);
    let build = |_ctx: &mut ViewContext<'_>| -> Element<()> {
        col()
            .id("outer-scroll")
            .w(240.0)
            .h(120.0)
            .clip()
            .scroll_y()
            .edge_fade(Edges::xy(0.0, 24.0))
            .child(box_().w(240.0).h(90.0).bg(Rgba::WHITE))
            .child(
                col()
                    .id("inner-scroll")
                    .w(240.0)
                    .h(80.0)
                    .clip()
                    .scroll_y()
                    .edge_fade(Edges::xy(0.0, 28.0))
                    .child(box_().id("inner-top").w(240.0).h(50.0).bg(Rgba::WHITE))
                    .child(box_().w(240.0).h(120.0).bg(Rgba::WHITE)),
            )
    };

    // At scroll_y = 0 for both outer & inner: top fade is 0 (no dimming band at top!),
    // while bottom fade is active and intersected to the visible outer bottom (y = 120).
    session.render(build);
    let mut saw_inner_fade = None;
    for batch in &session.draw_list().batches {
        if let triton_engine::gpu::DrawBatch::Quads {
            edge_fade,
            instances,
            ..
        } = batch
        {
            for q in instances {
                if (q.dst[1] - 90.0).abs() < 1.0 && (q.dst[3] - 50.0).abs() < 1.0 {
                    saw_inner_fade = *edge_fade;
                }
            }
        }
    }
    let ef = saw_inner_fade.expect("inner-top quad should carry intersected edge_fade");
    assert!(
        ef.widths[2].abs() < 0.01,
        "top edge_fade must be 0 when neither scroller is scrolled, got {ef:?}"
    );
    assert!(
        ef.widths[3] > 0.0 && (ef.rect[1] + ef.rect[3] - 120.0).abs() < 1.0,
        "nested bottom edge_fade must stay clamped to outer viewport bottom (120), got {ef:?}"
    );

    // Scroll outer down by 40px: top fade becomes active (> 0).
    session.scroll_key("outer-scroll", 40.0).unwrap();
    session.render(build);
    let top_quad_fade = session
        .draw_list()
        .batches
        .iter()
        .find_map(|b| match b {
            triton_engine::gpu::DrawBatch::Quads { edge_fade, .. } if edge_fade.is_some() => {
                *edge_fade
            }
            _ => None,
        })
        .expect("scrolled container emits edge_fade");
    assert!(
        top_quad_fade.widths[2] >= 23.5,
        "scrolling down 40px should activate full 24px top edge_fade, got {top_quad_fade:?}"
    );
}

#[test]
fn absolute_inset_all_resolves_against_direct_parent_not_root() {
    let mut session = HeadlessSession::<()>::new((400.0, 300.0), 1.0);
    let frame = session.render(|_| {
        box_().w(400.0).h(300.0).p(40.0).child(
            box_().id("host").size(20.0, 20.0).child(
                box_()
                    .id("overlay")
                    .absolute()
                    .inset_all(0.0)
                    .bg(Rgba::WHITE),
            ),
        )
    });

    let host = frame.node_by_key(NodeKey::named("host")).unwrap();
    let overlay = frame.node_by_key(NodeKey::named("overlay")).unwrap();
    assert!((host.rect.w - 20.0).abs() < 0.5 && (host.rect.h - 20.0).abs() < 0.5);
    assert!(
        (overlay.rect.w - 20.0).abs() < 0.5 && (overlay.rect.h - 20.0).abs() < 0.5,
        "absolute inset_all(0) must size to direct parent (20x20), not root (400x300): got {:?}",
        overlay.rect
    );
}

#[test]
fn smooth_corner_shadow_decays_cleanly_without_clamped_square_halo() {
    let mut renderer = HeadlessRenderer::new().expect("headless EGL/GLES3 must initialize");
    let mut session = HeadlessSession::<()>::new((320.0, 240.0), 1.0);

    session.render(|_| {
        box_()
            .w(320.0)
            .h(240.0)
            .p(Edges::new(50.0, 0.0, 0.0, 40.0))
            .child(
                box_()
                    .w(120.0)
                    .h(100.0)
                    .rounded(24.0)
                    .bg(Rgba::WHITE)
                    .shadow(DropShadow {
                        offset_x: 0.0,
                        offset_y: 12.0,
                        blur: 32.0,
                        spread: 0.0,
                        color: Rgba::new(0.0, 0.0, 0.0, 0.55),
                    }),
            )
    });

    let (w, _h, rgba) = renderer
        .render_rgba(&mut session)
        .expect("offscreen render succeeds");
    let alpha_at = |x: u32, y: u32| -> u8 { rgba[((y * w + x) * 4 + 3) as usize] };

    // Card occupies x = 40..160, y = 50..150.
    // Right beside the upper-right corner (y = 55, well inside LUT_EXTENT * r = 38.4 of top edge):
    // shadow alpha must decay monotonically with distance outside x = 160 (NOT stay stuck high!).
    let near = alpha_at(168, 55);
    let mid = alpha_at(186, 55);
    let far = alpha_at(212, 55);
    let outer_corner = alpha_at(214, 214);
    assert!(
        near > mid && mid > far && far <= 2 && outer_corner == 0,
        "smooth-corner Gaussian drop shadow must decay smoothly to zero without a rectangular floor box \
         (near={near}, mid={mid}, far={far}, outer_corner={outer_corner})"
    );
}

#[test]
fn input_rects_deduplicate_inside_scrollable_card_and_cull_offscreen_draw_work() {
    let mut session = HeadlessSession::<()>::new((400.0, 260.0), 1.0);
    let scroll_key = NodeKey::named("perf-scroll");

    let build_tree = || {
        let mut sc = col().id("perf-scroll").w(300.0).h(120.0).gap(8.0);
        for i in 0..30 {
            sc = sc.child(
                row()
                    .w(280.0)
                    .h(40.0)
                    .bg(Rgba::new(0.2, 0.2, 0.25, 1.0))
                    .child(text(format!("Row {i}"), FontSpec::ui(14.0), Rgba::WHITE)),
            );
        }
        col()
            .w(340.0)
            .h(200.0)
            .p(Edges::all(16.0))
            .bg(Rgba::new(0.1, 0.1, 0.14, 1.0))
            .child(sc.scroll_y())
    };

    let frame0 = session.render(|_| build_tree());
    let inputs0 = frame0.input_rects.clone();
    let quads0 = session.draw_list().quad_count();
    let glyphs0 = session.draw_list().glyph_count();
    assert_eq!(
        inputs0.len(),
        1,
        "all 30 scrollable child rows are contained inside the outer card's input_rect"
    );
    // Only the outer card + ~3 visible rows + scrollbar thumb should emit quads (not all 30 rows!).
    assert!(
        quads0 < 12,
        "off-screen scroll rows must be culled from DrawList (got {quads0} quads)"
    );
    assert!(
        glyphs0 < 40,
        "off-screen scroll labels must be culled from glyph emission (got {glyphs0} glyphs)"
    );

    // Scrolling must keep `input_rects` bit-identical so Wayland never rebuilds wl_region mid-scroll.
    session
        .interaction_mut()
        .set_scroll_offset(scroll_key, 240.0);
    let frame1 = session.render(|_| build_tree());
    assert_eq!(
        frame1.input_rects, inputs0,
        "scrolling inside a card must not churn Wayland input_rects"
    );
}
