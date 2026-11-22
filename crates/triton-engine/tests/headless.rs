//! Integration tests for `triton-engine`'s Rune-style headless dev loop:
//! - Phase 1: semantic `NodeInfo` tree recording and `BuiltFrame::tree_json()`
//! - Phase 2: `HeadlessSession`, key-addressed input, and JSON `Script` runner
//! - Phase 3: `Egl::headless()`, `Gpu::render_to_rgba()`, `HeadlessRenderer`,
//!   and zero-dependency PNG encoding (with real GPU pixel-level assertions).

use std::time::{Duration, Instant};

use triton_design::shape::Point;
use triton_engine::{
    AppCommand, DropShadow, Element, FontSpec, HeadlessRenderer, HeadlessSession, ImageData,
    ImageStore, ImageStyle, InputEvent, Key, NodeKey, NodeKind, Rect, Rgba, Script, ShellApp,
    Spring, SubtreeTransform, SurfaceSize, SurfaceSpec, ViewContext, box_, col, encode_png, image,
    polyline, row, text, write_png,
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
