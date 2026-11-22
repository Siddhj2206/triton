//! Headless `DrawList` integration tests for triton-kit Phase 1A.6 overlay
//! components: `menu`, `tooltip`, `toast_card`. Everything is exercised through
//! public APIs: `layout_and_paint` at 1.0 and 1.75 scale,
//! `BuiltFrame::dispatch_event`, deterministic `Animator::begin_frame`, dark
//! and light `Theme`.

use std::time::Instant;

use triton_engine::anim::Animator;
use triton_engine::gpu::{
    DrawBatch, DrawList, KIND_CAPSULE, KIND_FILL, KIND_SHADOW, KIND_TOP_HIGHLIGHT, QuadInstance,
};
use triton_engine::image::ImageStore;
use triton_engine::style::{Rgba, Theme};
use triton_engine::text::TextEngine;
use triton_engine::ui::{
    BuiltFrame, Element, InputEvent, InteractionState, NodeKey, box_, layout_and_paint,
};
use triton_kit::{
    Menu, MenuEntry, MenuItem, ToastAction, ToastActionTone, ToastCard, ToastIdentity,
    ToastVariant, menu, toast_card, toast_is_hovered, tooltip,
};

const EPS: f32 = 1e-3;

fn fill_is(q: &QuadInstance, color: [f32; 4]) -> bool {
    q.fill
        .iter()
        .zip(color.iter())
        .all(|(a, b)| (a - b).abs() <= EPS)
}

fn color_is(a: [f32; 4], b: [f32; 4]) -> bool {
    a.iter().zip(b.iter()).all(|(x, y)| (x - y).abs() <= EPS)
}

#[derive(Debug, Clone, PartialEq)]
enum Msg {
    Activate(&'static str),
    Dismiss,
    Action(&'static str),
    Under,
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

fn click(frame: &BuiltFrame<Msg>, interaction: &mut InteractionState, x: f32, y: f32) -> Vec<Msg> {
    frame.dispatch_event(&InputEvent::PointerDown { x, y, button: 272 }, interaction);
    frame
        .dispatch_event(&InputEvent::PointerUp { x, y, button: 272 }, interaction)
        .0
}

fn hit_rect<M>(frame: &BuiltFrame<M>, key: NodeKey) -> triton_engine::style::Rect {
    frame
        .hits
        .iter()
        .find(|h| h.key == Some(key))
        .unwrap_or_else(|| panic!("hit for {key:?}"))
        .rect
}

fn item(key: &str, label: &str) -> MenuItem<Msg> {
    MenuItem {
        key: NodeKey::named(key),
        label: label.to_owned(),
        ..MenuItem::default()
    }
}

// ── menu ─────────────────────────────────────────────────────────────────────

#[test]
fn menu_paints_glass_panel_with_concentric_pill_rows() {
    let mut text = TextEngine::new();
    for theme in [Theme::dark(), Theme::light()] {
        for scale in [1.0_f32, 1.75] {
            let mut dl = DrawList::default();
            let mut interaction = InteractionState::default();
            let spec = Menu {
                key: NodeKey::named("ctx"),
                width: 260.0,
                open_submenu: None,
                entries: vec![
                    MenuEntry::Item(item("open", "Open")),
                    MenuEntry::Item(item("copy", "Copy path")),
                ],
            };
            let frame = paint(
                menu(&theme, spec),
                (400.0, 300.0),
                scale,
                &interaction,
                &mut text,
                &mut dl,
            );

            // One Glass blur region with the panel's 28px radius.
            assert_eq!(frame.blur_rects.len(), 1, "glass panel blur at {scale}x");
            assert!((frame.blur_rects[0].radius - 28.0).abs() < EPS);

            // Panel quad: material fill + top highlight, r28.
            let panel_q = quads(&dl)
                .into_iter()
                .find(|q| q.misc[0] == KIND_TOP_HIGHLIGHT)
                .expect("material panel quad");
            for r in panel_q.radii {
                assert!((r - 28.0 * scale).abs() < EPS, "panel r28 at {scale}x");
            }
            assert_eq!(
                panel_q.fill,
                theme
                    .material_fill(triton_design::material::Material::Glass)
                    .to_array()
            );

            // Hover the first row: the fill becomes `primary @ 16%` on a
            // concentric r24 (`nested(28, 4)`) full-pill row, h48.
            let row0 = hit_rect(&frame, NodeKey::named("open"));
            assert!((row0.h - 48.0).abs() < EPS, "row h48");
            frame.dispatch_event(
                &InputEvent::PointerMove {
                    x: row0.x + row0.w * 0.5,
                    y: row0.y + row0.h * 0.5,
                },
                &mut interaction,
            );
            paint::<Msg>(
                menu(
                    &theme,
                    Menu {
                        key: NodeKey::named("ctx"),
                        width: 260.0,
                        open_submenu: None,
                        entries: vec![
                            MenuEntry::Item(item("open", "Open")),
                            MenuEntry::Item(item("copy", "Copy path")),
                        ],
                    },
                ),
                (400.0, 300.0),
                scale,
                &interaction,
                &mut text,
                &mut dl,
            );
            let hover = Rgba::from_color(theme.scheme.primary).with_alpha(0.16);
            let row_q = quads(&dl)
                .into_iter()
                .find(|q| fill_is(q, hover.to_array()))
                .expect("hovered row paints primary @ 16%");
            assert!(
                (row_q.dst[3] - 48.0 * scale).abs() < EPS,
                "row h48 at {scale}x"
            );
            for r in row_q.radii {
                assert!((r - 24.0 * scale).abs() < EPS, "concentric r24 at {scale}x");
            }
            assert!(row_q.misc[1].abs() < EPS, "full-pill row smoothing 0");
        }
    }
}

#[test]
fn menu_item_click_activates_and_disabled_suppresses() {
    let mut text = TextEngine::new();
    let mut dl = DrawList::default();
    let mut interaction = InteractionState::default();
    let theme = Theme::dark();

    let spec = Menu {
        key: NodeKey::named("ctx"),
        width: 260.0,
        open_submenu: None,
        entries: vec![
            MenuEntry::Item(MenuItem {
                on_click: Some(Box::new(|| Msg::Activate("open"))),
                ..item("open", "Open")
            }),
            MenuEntry::Item(MenuItem {
                disabled: true,
                on_click: Some(Box::new(|| Msg::Activate("locked"))),
                ..item("locked", "Locked")
            }),
        ],
    };
    let frame = paint(
        menu(&theme, spec),
        (400.0, 300.0),
        1.0,
        &interaction,
        &mut text,
        &mut dl,
    );

    let r = hit_rect(&frame, NodeKey::named("open"));
    assert_eq!(
        click(&frame, &mut interaction, r.x + r.w * 0.5, r.y + r.h * 0.5),
        vec![Msg::Activate("open")]
    );

    // The disabled row is inert: it has no hit entry of its own, so clicks
    // land on the input-opaque panel and emit nothing.
    assert!(
        frame
            .hits
            .iter()
            .all(|h| h.key != Some(NodeKey::named("locked"))),
        "disabled row is not hit-testable"
    );
    assert!(
        click(&frame, &mut interaction, 130.0, 76.0).is_empty(),
        "disabled item emits no click"
    );
}

#[test]
fn menu_input_opaque_blocks_clicks_on_padded_area() {
    let mut text = TextEngine::new();
    let mut dl = DrawList::default();
    let mut interaction = InteractionState::default();
    let theme = Theme::dark();

    let root = box_()
        .w(400.0)
        .h(300.0)
        .child(
            box_()
                .absolute()
                .inset_all(0.0)
                .id(NodeKey::named("under"))
                .bg(Rgba::new(0.2, 0.4, 0.8, 1.0))
                .on_click(|| Msg::Under),
        )
        .child(
            menu(
                &theme,
                Menu {
                    key: NodeKey::named("ctx"),
                    width: 200.0,
                    open_submenu: None,
                    entries: vec![MenuEntry::Item(item("open", "Open"))],
                },
            )
            .absolute()
            .inset_xy(20.0, 20.0),
        );
    let frame = paint(root, (400.0, 300.0), 1.0, &interaction, &mut text, &mut dl);

    // A click landing on the menu's padding (inside the panel, outside every
    // row) must not fall through to the button painted underneath.
    assert!(
        click(&frame, &mut interaction, 30.0, 22.0).is_empty(),
        "menu padding swallows the click"
    );
    // Outside the panel the underlying button still receives clicks.
    assert_eq!(
        click(&frame, &mut interaction, 10.0, 10.0),
        vec![Msg::Under]
    );
}

#[test]
fn menu_paints_shortcut_column_icons_separator_and_submenu_indicator() {
    let mut text = TextEngine::new();
    let mut dl = DrawList::default();
    let theme = Theme::dark();
    let spec = Menu {
        key: NodeKey::named("ctx"),
        width: 260.0,
        open_submenu: None,
        entries: vec![
            MenuEntry::Item(MenuItem {
                icon: Some(triton_design::icon::Icon::FolderOpen),
                shortcut: Some("Enter".to_owned()),
                ..item("open", "Open")
            }),
            MenuEntry::Item(MenuItem {
                icon: Some(triton_design::icon::Icon::OpenInNew),
                submenu: Some(Box::new(Menu {
                    key: NodeKey::named("sub"),
                    width: 200.0,
                    open_submenu: None,
                    entries: vec![MenuEntry::Item(item("editor", "Text Editor"))],
                })),
                ..item("open_with", "Open with")
            }),
            MenuEntry::Separator,
            MenuEntry::Item(MenuItem {
                icon: Some(triton_design::icon::Icon::Delete),
                danger: true,
                shortcut: Some("Del".to_owned()),
                ..item("trash", "Move to Trash")
            }),
        ],
    };
    paint::<Msg>(
        menu(&theme, spec),
        (400.0, 300.0),
        1.0,
        &InteractionState::default(),
        &mut text,
        &mut dl,
    );

    // Labels (Open/Open with/Move to Trash), shortcut text (Enter/Del), four
    // icon glyphs (folder_open, open_in_new, chevron_right, delete).
    assert!(glyph_count(&dl) >= 10, "labels + shortcuts + icons paint");

    // The separator emits a 1px hairline in the on-glass divider tone.
    let hairline = quads(&dl)
        .into_iter()
        .find(|q| (q.dst[3] - 1.0).abs() < EPS && q.misc[0] == KIND_FILL)
        .expect("separator hairline");
    assert!(
        (hairline.fill[3] - 0.12).abs() < EPS,
        "on-glass divider alpha"
    );

    // Danger content paints in `error` (label glyph color).
    let error = Rgba::from_color(theme.scheme.error).to_array();
    let danger_glyph = dl
        .batches
        .iter()
        .find_map(|b| match b {
            DrawBatch::Glyphs { instances, .. } => {
                instances.iter().find(|g| color_is(g.color, error))
            }
            _ => None,
        })
        .expect("danger item uses error-colored glyphs");
    assert!(danger_glyph.dst[2] > 0.0);
}

#[test]
fn menu_open_submenu_renders_second_glass_panel_beside_it() {
    let mut text = TextEngine::new();
    for scale in [1.0_f32, 1.75] {
        let mut dl = DrawList::default();
        let theme = Theme::dark();
        let spec = Menu {
            key: NodeKey::named("ctx"),
            width: 260.0,
            open_submenu: Some(NodeKey::named("open_with")),
            entries: vec![
                MenuEntry::Item(item("open", "Open")),
                MenuEntry::Item(MenuItem {
                    submenu: Some(Box::new(Menu {
                        key: NodeKey::named("sub"),
                        width: 200.0,
                        open_submenu: None,
                        entries: vec![
                            MenuEntry::Item(item("editor", "Text Editor")),
                            MenuEntry::Item(item("console", "Console")),
                        ],
                    })),
                    ..item("open_with", "Open with")
                }),
            ],
        };
        let frame = paint::<Msg>(
            menu(&theme, spec),
            (600.0, 400.0),
            scale,
            &InteractionState::default(),
            &mut text,
            &mut dl,
        );

        // Two Glass panels → two blur regions and two material quads.
        assert_eq!(frame.blur_rects.len(), 2, "parent + submenu blur");
        let panels: Vec<QuadInstance> = quads(&dl)
            .into_iter()
            .filter(|q| q.misc[0] == KIND_TOP_HIGHLIGHT)
            .collect();
        assert_eq!(panels.len(), 2, "two glass panels");

        // The submenu floats just right of the parent panel's edge.
        let parent = frame.blur_rects[0].rect;
        let sub = frame.blur_rects[1].rect;
        assert!(
            sub.x >= parent.x + parent.w,
            "submenu right of parent: sub.x={} parent right={}",
            sub.x,
            parent.x + parent.w
        );
        assert!(glyph_count(&dl) >= 5, "submenu labels paint");
    }
}

// ── tooltip ──────────────────────────────────────────────────────────────────

#[test]
fn tooltip_is_32px_r12_glass_pill_with_blur_and_shadow() {
    let mut text = TextEngine::new();
    for theme in [Theme::dark(), Theme::light()] {
        for scale in [1.0_f32, 1.75] {
            let mut dl = DrawList::default();
            let frame = paint::<Msg>(
                tooltip("Copy path", &theme),
                (200.0, 100.0),
                scale,
                &InteractionState::default(),
                &mut text,
                &mut dl,
            );

            // Glass blur region with the pill's r12 corner.
            assert_eq!(frame.blur_rects.len(), 1, "tooltip blur at {scale}x");
            assert!((frame.blur_rects[0].radius - 12.0).abs() < EPS);

            // Material quad: fill + top highlight, h32 r12.
            let pill = quads(&dl)
                .into_iter()
                .find(|q| q.misc[0] == KIND_TOP_HIGHLIGHT)
                .expect("glass pill quad");
            assert!((pill.dst[3] - 32.0 * scale).abs() < EPS, "h32 at {scale}x");
            for r in pill.radii {
                assert!((r - 12.0 * scale).abs() < EPS, "r12 at {scale}x");
            }
            assert!(pill.misc[1].abs() < EPS, "pill smoothing 0");
            assert_eq!(
                pill.fill,
                theme
                    .material_fill(triton_design::material::Material::Glass)
                    .to_array()
            );

            // Subtle raised shadow underneath.
            assert!(
                quads(&dl).iter().any(|q| q.misc[0] == KIND_SHADOW),
                "floating shadow at {scale}x"
            );
            assert!(glyph_count(&dl) >= 1, "label glyphs paint");
        }
    }
}

// ── toast_card ───────────────────────────────────────────────────────────────

fn standard_toast(key: &str) -> ToastCard<Msg> {
    ToastCard {
        key: NodeKey::named(key),
        width: 360.0,
        identity: Some(ToastIdentity::Initials("MK".to_owned())),
        title: Some("Mia".to_owned()),
        timestamp: Some("now".to_owned()),
        body: Some("Thinking ramen at 1, you in?".to_owned()),
        ..ToastCard::default()
    }
}

#[test]
fn toast_standard_paints_asymmetric_glass_card_with_wave_identity() {
    let mut text = TextEngine::new();
    let mut anim = Animator::new();
    let t0 = Instant::now();
    for scale in [1.0_f32, 1.75] {
        let mut dl = DrawList::default();
        let theme = Theme::dark();
        anim.begin_frame(t0, false);
        let frame = paint::<Msg>(
            toast_card(&theme, &mut anim, standard_toast("toast")),
            (420.0, 200.0),
            scale,
            &InteractionState::default(),
            &mut text,
            &mut dl,
        );

        // Glass blur + the `.nc` asymmetric corners: rounded r24 everywhere
        // except a tight 10px bottom-right (`Corners` order tl,tr,br,bl).
        assert_eq!(frame.blur_rects.len(), 1, "toast blur at {scale}x");
        assert!((frame.blur_rects[0].radius - 24.0).abs() < EPS);
        let card = quads(&dl)
            .into_iter()
            .find(|q| q.misc[0] == KIND_TOP_HIGHLIGHT)
            .expect("glass card quad");
        let expected = [24.0, 24.0, 10.0, 24.0].map(|r| r * scale);
        for (got, want) in card.radii.iter().zip(expected.iter()) {
            assert!(
                (got - want).abs() < EPS,
                "corner {got} vs {want} at {scale}x"
            );
        }
        assert_eq!(
            card.fill,
            theme
                .material_fill(triton_design::material::Material::Glass)
                .to_array()
        );
        assert!(
            quads(&dl).iter().any(|q| q.misc[0] == KIND_SHADOW),
            "floating shadow"
        );

        // 48px wave avatar + title/timestamp/body/initials glyphs.
        assert!(
            quads(&dl)
                .iter()
                .any(|q| q.misc[0] == triton_engine::gpu::KIND_WAVE
                    && (q.dst[2] - 48.0 * scale).abs() < EPS),
            "48px wave avatar at {scale}x"
        );
        assert!(glyph_count(&dl) >= 10, "title + time + body + initials");
    }
}

#[test]
fn toast_critical_fill_and_content_differ_from_standard() {
    let mut text = TextEngine::new();
    for theme in [Theme::dark(), Theme::light()] {
        let mut anim = Animator::new();
        anim.begin_frame(Instant::now(), false);
        let mut dl = DrawList::default();
        let spec = ToastCard {
            variant: ToastVariant::Critical,
            ..standard_toast("toast")
        };
        paint::<Msg>(
            toast_card(&theme, &mut anim, spec),
            (420.0, 200.0),
            1.0,
            &InteractionState::default(),
            &mut text,
            &mut dl,
        );
        let card = quads(&dl)
            .into_iter()
            .find(|q| q.misc[0] == KIND_TOP_HIGHLIGHT)
            .expect("critical card quad");
        let want = Rgba::from_color(theme.scheme.error_container).with_alpha(0.78);
        assert_eq!(card.fill, want.to_array(), "error_container @ .78 fill");
        assert_ne!(
            card.fill,
            theme
                .material_fill(triton_design::material::Material::Glass)
                .to_array(),
            "critical is visually distinct from Standard"
        );
        // Content paints `on_error_container`.
        let oec = Rgba::from_color(theme.scheme.on_error_container).to_array();
        assert!(
            dl.batches.iter().any(|b| match b {
                DrawBatch::Glyphs { instances, .. } => {
                    instances.iter().any(|g| color_is(g.color, oec))
                }
                _ => false,
            }),
            "critical text uses on_error_container"
        );
    }
}

#[test]
fn toast_progress_variant_emits_wavy_capsules_and_percent() {
    let mut text = TextEngine::new();
    let mut anim = Animator::new();
    let theme = Theme::dark();
    anim.begin_frame(Instant::now(), false);
    let mut dl = DrawList::default();
    let spec = ToastCard {
        variant: ToastVariant::Progress {
            value: 0.58,
            label: None,
        },
        ..standard_toast("toast")
    };
    paint::<Msg>(
        toast_card(&theme, &mut anim, spec),
        (420.0, 200.0),
        1.0,
        &InteractionState::default(),
        &mut text,
        &mut dl,
    );
    // Flat track + wavy played part, both SDF capsules.
    assert!(
        quads(&dl)
            .iter()
            .filter(|q| q.misc[0] == KIND_CAPSULE)
            .count()
            >= 2,
        "wavy progress paints KIND_CAPSULE quads"
    );
    // "58%" percent readout next to the bar.
    assert!(glyph_count(&dl) >= 3, "percent text paints");
}

#[test]
fn toast_action_dismiss_and_activate_route_distinct_messages() {
    let mut text = TextEngine::new();
    let mut anim = Animator::new();
    let theme = Theme::dark();
    anim.begin_frame(Instant::now(), false);
    let mut dl = DrawList::default();
    let mut interaction = InteractionState::default();
    let spec = ToastCard {
        actions: vec![ToastAction {
            key: NodeKey::named("reply"),
            label: "Reply".to_owned(),
            icon: None,
            tone: ToastActionTone::Primary,
            on_click: Some(Box::new(|| Msg::Action("reply"))),
        }],
        on_dismiss: Some(Box::new(|| Msg::Dismiss)),
        on_activate: Some(Box::new(|| Msg::Activate("toast"))),
        ..standard_toast("toast")
    };
    let frame = paint::<Msg>(
        toast_card(&theme, &mut anim, spec),
        (420.0, 200.0),
        1.0,
        &interaction,
        &mut text,
        &mut dl,
    );

    // Action button: only its own message.
    let a = hit_rect(&frame, NodeKey::named("reply"));
    assert_eq!(
        click(&frame, &mut interaction, a.x + a.w * 0.5, a.y + a.h * 0.5),
        vec![Msg::Action("reply")]
    );

    // Dismiss (the only 24px-tall interactive hit): `on_dismiss` fires and
    // the card's `on_activate` must NOT.
    let d = frame
        .hits
        .iter()
        .find(|h| (h.rect.h - 24.0).abs() < EPS && h.on_click.is_some())
        .expect("dismiss button hit")
        .rect;
    assert_eq!(
        click(&frame, &mut interaction, d.x + d.w * 0.5, d.y + d.h * 0.5),
        vec![Msg::Dismiss],
        "dismiss emits on_dismiss only"
    );

    // Card body (its own hit rect, padding corner avoids all children):
    // `on_activate` fires.
    let c = hit_rect(&frame, NodeKey::named("toast"));
    assert_eq!(
        click(&frame, &mut interaction, c.x + 4.0, c.y + c.h * 0.5),
        vec![Msg::Activate("toast")]
    );
}

#[test]
fn toast_is_hovered_stays_true_over_dismiss_and_action_children() {
    let mut text = TextEngine::new();
    let mut anim = Animator::new();
    let theme = Theme::dark();
    anim.begin_frame(Instant::now(), false);
    let mut dl = DrawList::default();
    let mut interaction = InteractionState::default();
    let key = NodeKey::named("toast");
    let spec = ToastCard {
        actions: vec![ToastAction {
            key: NodeKey::named("reply"),
            label: "Reply".to_owned(),
            icon: None,
            tone: ToastActionTone::Primary,
            on_click: Some(Box::new(|| Msg::Action("reply"))),
        }],
        on_dismiss: Some(Box::new(|| Msg::Dismiss)),
        ..standard_toast("toast")
    };
    let frame = paint::<Msg>(
        toast_card(&theme, &mut anim, spec),
        (420.0, 200.0),
        1.0,
        &interaction,
        &mut text,
        &mut dl,
    );

    assert!(!toast_is_hovered(key, &interaction));

    // Over the card body → hovered within.
    let c = hit_rect(&frame, key);
    frame.dispatch_event(
        &InputEvent::PointerMove {
            x: c.x + 4.0,
            y: c.y + c.h * 0.5,
        },
        &mut interaction,
    );
    assert!(toast_is_hovered(key, &interaction), "over card body");

    // Over the dismiss button → still within the toast (timer stays paused).
    let d = frame
        .hits
        .iter()
        .find(|h| (h.rect.h - 24.0).abs() < EPS && h.on_click.is_some())
        .expect("dismiss button hit")
        .rect;
    frame.dispatch_event(
        &InputEvent::PointerMove {
            x: d.x + d.w * 0.5,
            y: d.y + d.h * 0.5,
        },
        &mut interaction,
    );
    assert!(toast_is_hovered(key, &interaction), "over dismiss button");

    // Over an action button → still within.
    let a = hit_rect(&frame, NodeKey::named("reply"));
    frame.dispatch_event(
        &InputEvent::PointerMove {
            x: a.x + a.w * 0.5,
            y: a.y + a.h * 0.5,
        },
        &mut interaction,
    );
    assert!(toast_is_hovered(key, &interaction), "over action button");

    // Outside the card rect → cleared.
    frame.dispatch_event(
        &InputEvent::PointerMove {
            x: c.x + c.w + 20.0,
            y: c.y + c.h + 20.0,
        },
        &mut interaction,
    );
    assert!(!toast_is_hovered(key, &interaction), "outside the card");
}
