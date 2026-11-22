//! Headless `DrawList` integration tests for triton-kit Phase 1A.4 text-entry
//! widgets: `text_input`, `search_field`, `password_field`. Everything is
//! exercised through public APIs: `layout_and_paint` at 1.0 and 1.75 scale,
//! `BuiltFrame::dispatch_event` (pointer + keyboard), deterministic
//! `Animator::begin_frame`, dark and light `Theme`.

use std::time::{Duration, Instant};

use triton_design::icon::Icon;
use triton_engine::anim::Animator;
use triton_engine::gpu::{DrawBatch, DrawList, GlyphInstance, KIND_FILL, KIND_WAVE, QuadInstance};
use triton_engine::image::ImageStore;
use triton_engine::style::{FontSpec, Rect, Rgba, Theme};
use triton_engine::text::TextEngine;
use triton_engine::ui::{
    BuiltFrame, Element, InputEvent, InteractionState, Key, NodeKey, layout_and_paint,
};
use triton_kit::{
    PasswordField, SearchField, SearchMode, TextInput, TextInputAction, TextInputState,
    password_field, search_field, text_input, trigger_password_shake,
};

const EPS: f32 = 1e-3;

#[derive(Debug, Clone, PartialEq)]
enum Msg {
    Input(InputEvent),
    Clear,
    Reveal,
    Submit,
}

fn rgba_eq(a: [f32; 4], b: [f32; 4]) -> bool {
    a.iter().zip(b.iter()).all(|(x, y)| (x - y).abs() <= EPS)
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

fn glyphs(dl: &DrawList) -> Vec<GlyphInstance> {
    let mut out = Vec::new();
    for b in &dl.batches {
        if let DrawBatch::Glyphs { instances, .. } = b {
            out.extend(instances.iter().copied());
        }
    }
    out
}

/// First quad whose buffer-px size is within `tol` of `(w * scale, h * scale)`.
fn quad_sized(dl: &DrawList, w: f32, h: f32, scale: f32, tol: f32) -> Option<QuadInstance> {
    quads(dl)
        .into_iter()
        .find(|q| (q.dst[2] - w * scale).abs() <= tol && (q.dst[3] - h * scale).abs() <= tol)
}

/// The 2px-wide caret quad carries the field's `primary` caret color.
fn caret_quad(dl: &DrawList, scale: f32) -> Option<QuadInstance> {
    quads(dl).into_iter().find(|q| {
        (q.dst[2] - 2.0 * scale).abs() <= 0.01 && q.misc[0] == KIND_FILL && q.fill[3] > 0.9
    })
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

fn key_event(key: Key, utf8: Option<&str>, caps_lock: bool) -> InputEvent {
    InputEvent::KeyDown {
        key,
        utf8: utf8.map(str::to_owned),
        ctrl: false,
        shift: false,
        alt: false,
        caps_lock,
    }
}

/// Field rect of the single widget in the frame, found by hit key.
fn hit_rect<M>(frame: &BuiltFrame<M>, key: &str) -> Rect {
    frame
        .hits
        .iter()
        .find(|h| h.key == Some(NodeKey::named(key)))
        .unwrap_or_else(|| panic!("hit entry for {key}"))
        .rect
}

/// The full-width field quad of a `h`-tall pill painted at origin.
fn field_quad(dl: &DrawList, w: f32, h: f32, scale: f32) -> QuadInstance {
    quad_sized(dl, w, h, scale, 0.5).expect("field pill quad")
}

// ── text_input ───────────────────────────────────────────────────────────────

fn ti_spec() -> TextInput<Msg> {
    TextInput {
        key: NodeKey::named("ti"),
        placeholder: "Search apps…".to_owned(),
        lead_icon: None,
        on_event: Some(Box::new(|ev| Some(Msg::Input(ev.clone())))),
        ..Default::default()
    }
}

#[test]
fn text_input_shows_placeholder_dimmed_and_value_solid() {
    let mut text = TextEngine::new();
    let interaction = InteractionState::default();
    let mut dl = DrawList::default();
    let theme = Theme::dark();

    // Empty: placeholder paints `on_surface_variant` at reduced alpha.
    let state = TextInputState::default();
    paint(
        text_input(&state, ti_spec(), &theme, &interaction, &mut text),
        (400.0, 80.0),
        1.0,
        &interaction,
        &mut text,
        &mut dl,
    );
    let variant = Rgba::from_color(theme.scheme.on_surface_variant).to_array();
    let ghost: Vec<GlyphInstance> = glyphs(&dl)
        .into_iter()
        .filter(|g| (g.color[3] - 0.72).abs() < 0.05)
        .collect();
    assert!(ghost.len() >= 8, "placeholder glyphs painted dimmed");
    for g in &ghost {
        assert!(
            (g.color[0] - variant[0]).abs() < EPS
                && (g.color[1] - variant[1]).abs() < EPS
                && (g.color[2] - variant[2]).abs() < EPS,
            "placeholder uses on_surface_variant"
        );
    }

    // Typed value: full-alpha `on_surface` glyphs.
    let state = TextInputState::new("hello");
    let mut dl = DrawList::default();
    paint(
        text_input(&state, ti_spec(), &theme, &interaction, &mut text),
        (400.0, 80.0),
        1.0,
        &interaction,
        &mut text,
        &mut dl,
    );
    let on_surface = Rgba::from_color(theme.scheme.on_surface).to_array();
    let value: Vec<GlyphInstance> = glyphs(&dl)
        .into_iter()
        .filter(|g| (g.color[3] - 1.0).abs() < 0.01)
        .collect();
    assert_eq!(value.len(), 5, "value glyphs at full alpha");
    for g in &value {
        assert_rgba(g.color, on_surface, "value glyph");
    }
}

#[test]
fn text_input_focus_ring_swaps_hairline_for_primary_2px() {
    let mut text = TextEngine::new();
    let theme = Theme::dark();

    for scale in [1.0_f32, 1.75] {
        let mut interaction = InteractionState::default();
        let mut dl = DrawList::default();
        let state = TextInputState::new("query");
        let frame = paint(
            text_input(&state, ti_spec(), &theme, &interaction, &mut text),
            (400.0, 80.0),
            scale,
            &interaction,
            &mut text,
            &mut dl,
        );

        // Resting hairline: 1px `outline_variant` at low alpha.
        let field = field_quad(&dl, 400.0, 52.0, scale);
        assert!(
            (field.misc[2] - 1.0 * scale).abs() < EPS,
            "resting border 1px @ {scale}x"
        );
        let expected = Rgba::from_color(theme.scheme.outline_variant)
            .with_alpha(0.55)
            .to_array();
        assert_rgba(field.border, expected, "resting hairline");
        assert!(caret_quad(&dl, scale).is_none(), "no caret while unfocused");

        // Click into the field: 2px primary ring + live caret.
        let msgs = click(&frame, &mut interaction, 200.0, 26.0);
        assert!(msgs.is_empty(), "field itself emits no click");
        assert!(interaction.is_focused(NodeKey::named("ti")));

        let state = TextInputState::new("query");
        let mut dl = DrawList::default();
        paint(
            text_input(&state, ti_spec(), &theme, &interaction, &mut text),
            (400.0, 80.0),
            scale,
            &interaction,
            &mut text,
            &mut dl,
        );
        let field = field_quad(&dl, 400.0, 52.0, scale);
        assert!(
            (field.misc[2] - 2.0 * scale).abs() < EPS,
            "focused border 2px @ {scale}x"
        );
        assert_rgba(
            field.border,
            Rgba::from_color(theme.scheme.primary).to_array(),
            "focused ring",
        );
        let caret = caret_quad(&dl, scale).expect("caret while focused");
        assert_rgba(
            caret.fill,
            Rgba::from_color(theme.scheme.primary).to_array(),
            "caret color",
        );
    }
}

#[test]
fn text_input_keyboard_routes_events_and_caret_tracks_cursor() {
    let mut text = TextEngine::new();
    let mut interaction = InteractionState::default();
    let mut dl = DrawList::default();
    let theme = Theme::dark();
    let mut state = TextInputState::default();

    // Focus the field, then type a multibyte char.
    let frame = paint(
        text_input(&state, ti_spec(), &theme, &interaction, &mut text),
        (400.0, 80.0),
        1.0,
        &interaction,
        &mut text,
        &mut dl,
    );
    click(&frame, &mut interaction, 200.0, 26.0);

    let ev = key_event(Key::Char('é'), Some("é"), false);
    let (msgs, _dirty) = frame.dispatch_event(&ev, &mut interaction);
    assert_eq!(msgs, vec![Msg::Input(ev.clone())]);

    // The app applies the event (showcase pattern), then repaints.
    assert_eq!(state.apply_key(&ev), Some(TextInputAction::Changed));
    assert_eq!(state.value, "é");
    assert_eq!(state.cursor, "é".len());

    let mut dl = DrawList::default();
    paint(
        text_input(&state, ti_spec(), &theme, &interaction, &mut text),
        (400.0, 80.0),
        1.0,
        &interaction,
        &mut text,
        &mut dl,
    );
    let caret = caret_quad(&dl, 1.0).expect("caret");
    // Field px(16) → text box at x=16; caret offset = measured 'é' advance + 1.
    let font = FontSpec::ui(17.0).weight(500);
    let expected = 16.0 + text.measure_caret_x(&state.value, state.cursor, &font) + 1.0;
    assert!(
        (caret.dst[0] - expected).abs() < 0.5,
        "caret lands at measured x: got {}, want {expected}",
        caret.dst[0]
    );

    // Enter and Escape surface as actions, not edits.
    let frame = paint(
        text_input(&state, ti_spec(), &theme, &interaction, &mut text),
        (400.0, 80.0),
        1.0,
        &interaction,
        &mut text,
        &mut dl,
    );
    let enter = key_event(Key::Enter, None, false);
    let (msgs, _) = frame.dispatch_event(&enter, &mut interaction);
    assert_eq!(msgs, vec![Msg::Input(enter.clone())]);
    assert_eq!(state.apply_key(&enter), Some(TextInputAction::Submit));
    let esc = key_event(Key::Escape, None, false);
    let (msgs, _) = frame.dispatch_event(&esc, &mut interaction);
    assert_eq!(msgs, vec![Msg::Input(esc.clone())]);
    assert_eq!(state.apply_key(&esc), Some(TextInputAction::Escape));
}

#[test]
fn text_input_error_paints_error_border_and_inline_text() {
    let mut text = TextEngine::new();
    let interaction = InteractionState::default();
    let mut dl = DrawList::default();

    for theme in [Theme::dark(), Theme::light()] {
        let state = TextInputState::new("bad");
        let frame = paint(
            text_input(
                &state,
                TextInput::<Msg> {
                    key: NodeKey::named("ti"),
                    placeholder: "…".to_owned(),
                    error: Some("Value is required".to_owned()),
                    ..Default::default()
                },
                &theme,
                &interaction,
                &mut text,
            ),
            (400.0, 110.0),
            1.0,
            &interaction,
            &mut text,
            &mut dl,
        );
        // Error copy lives below the 52px field.
        assert!(frame.content_size.1 > 52.0, "error row extends height");
        let err = Rgba::from_color(theme.scheme.error).to_array();
        let err_glyphs: Vec<GlyphInstance> = glyphs(&dl)
            .into_iter()
            .filter(|g| rgba_eq(g.color, err))
            .collect();
        assert_eq!(
            err_glyphs.len(),
            "Value is required"
                .chars()
                .filter(|c| !c.is_whitespace())
                .count(),
            "error text painted in scheme.error"
        );

        // Unfocused error state still carries the error border.
        let field = field_quad(&dl, 400.0, 52.0, 1.0);
        assert_rgba(field.border, err, "error border");
        dl.clear();
    }
}

// ── search_field ─────────────────────────────────────────────────────────────

fn sf_spec() -> SearchField<Msg> {
    SearchField {
        key: NodeKey::named("sf"),
        placeholder: "Search…".to_owned(),
        on_event: Some(Box::new(|ev| Some(Msg::Input(ev.clone())))),
        ..Default::default()
    }
}

fn wave_disc(dl: &DrawList, scale: f32) -> Option<QuadInstance> {
    quads(dl).into_iter().find(|q| {
        q.misc[0] == KIND_WAVE
            && (q.dst[2] - 40.0 * scale).abs() < 0.5
            && (q.dst[3] - 40.0 * scale).abs() < 0.5
    })
}

#[test]
fn search_field_paints_mode_aware_wave_disc() {
    let mut text = TextEngine::new();
    let interaction = InteractionState::default();
    let theme = Theme::dark();

    // Mode → glyph mapping is part of the launcher grammar.
    assert_eq!(SearchMode::Search.icon(), Icon::Search);
    assert_eq!(SearchMode::Math.icon(), Icon::Calculate);
    assert_eq!(SearchMode::Command.icon(), Icon::Terminal);
    assert_eq!(SearchMode::File.icon(), Icon::Folder);

    for scale in [1.0_f32, 1.75] {
        for (mode, expected) in [
            (SearchMode::Search, theme.scheme.primary_container),
            (SearchMode::Math, theme.scheme.tertiary_container),
            (SearchMode::Command, theme.scheme.secondary_container),
            (SearchMode::File, theme.scheme.surface_container_highest),
        ] {
            let state = TextInputState::default();
            let mut dl = DrawList::default();
            paint(
                search_field(
                    &state,
                    SearchField::<Msg> { mode, ..sf_spec() },
                    &theme,
                    &interaction,
                    &mut text,
                ),
                (420.0, 80.0),
                scale,
                &interaction,
                &mut text,
                &mut dl,
            );
            let disc = wave_disc(&dl, scale).unwrap_or_else(|| panic!("wave disc {mode:?}"));
            // 9-lobe wave: radii slot 2 carries the lobe count.
            assert_eq!(disc.radii[2], 9.0, "9 lobes @ {scale}x");
            assert_rgba(
                disc.fill,
                Rgba::from_color(expected).to_array(),
                "disc tint",
            );
        }
    }
}

#[test]
fn search_field_is_a_52px_pill_with_primary_focus_ring() {
    let mut text = TextEngine::new();
    let theme = Theme::dark();
    let state = TextInputState::default();

    for scale in [1.0_f32, 1.75] {
        let mut interaction = InteractionState::default();
        let mut dl = DrawList::default();
        let frame = paint(
            search_field(&state, sf_spec(), &theme, &interaction, &mut text),
            (420.0, 80.0),
            scale,
            &interaction,
            &mut text,
            &mut dl,
        );
        let field = field_quad(&dl, 420.0, 52.0, scale);
        // True pill: radius clamped to h/2, zero corner smoothing.
        assert!((field.radii[0] - 26.0 * scale).abs() < EPS, "pill radius");
        assert!(field.misc[1].abs() < EPS, "pill smoothing 0");
        assert_rgba(field.fill, theme.veil().to_array(), "veil bg");

        click(&frame, &mut interaction, 210.0, 26.0);
        assert!(interaction.is_focused(NodeKey::named("sf")));

        let mut dl = DrawList::default();
        paint(
            search_field(&state, sf_spec(), &theme, &interaction, &mut text),
            (420.0, 80.0),
            scale,
            &interaction,
            &mut text,
            &mut dl,
        );
        let field = field_quad(&dl, 420.0, 52.0, scale);
        assert!((field.misc[2] - 2.0 * scale).abs() < EPS, "2px ring");
        assert_rgba(
            field.border,
            Rgba::from_color(theme.scheme.primary).to_array(),
            "focus ring",
        );
        assert!(caret_quad(&dl, scale).is_some(), "live caret when focused");
    }
}

#[test]
fn search_field_keycap_hint_and_clear_button() {
    let mut text = TextEngine::new();
    let mut interaction = InteractionState::default();
    let theme = Theme::dark();

    // Empty value: hint shows but no clear affordance.
    let state = TextInputState::default();
    let mut dl = DrawList::default();
    let frame = paint(
        search_field(
            &state,
            SearchField {
                keycap_hint: Some("Tab".to_owned()),
                on_clear: Some(Box::new(|| Msg::Clear)),
                ..sf_spec()
            },
            &theme,
            &interaction,
            &mut text,
        ),
        (420.0, 80.0),
        1.0,
        &interaction,
        &mut text,
        &mut dl,
    );
    // Flat keycap: 24px-tall bordered cap with on_surface@.35 hairline.
    let cap = quads(&dl)
        .into_iter()
        .find(|q| (q.dst[3] - 24.0).abs() < 0.5 && q.border[3] > 0.2)
        .expect("flat keycap");
    let cap_border = Rgba::from_color(theme.scheme.on_surface)
        .with_alpha(0.35)
        .to_array();
    assert_rgba(cap.border, cap_border, "keycap hairline");
    assert!(
        click(
            &frame,
            &mut interaction,
            cap.dst[0] + 4.0,
            cap.dst[1] + 12.0
        )
        .is_empty(),
        "keycap is inert"
    );

    // Non-empty: clear button appears at the trailing edge and emits.
    let state = TextInputState::new("firefox");
    let mut dl = DrawList::default();
    let frame = paint(
        search_field(
            &state,
            SearchField {
                keycap_hint: Some("Tab".to_owned()),
                on_clear: Some(Box::new(|| Msg::Clear)),
                ..sf_spec()
            },
            &theme,
            &interaction,
            &mut text,
        ),
        (420.0, 80.0),
        1.0,
        &interaction,
        &mut text,
        &mut dl,
    );
    let clear_hit = frame
        .hits
        .iter()
        .find(|h| h.on_click.is_some())
        .expect("clear hit entry");
    let msgs = click(
        &frame,
        &mut interaction,
        clear_hit.rect.x + clear_hit.rect.w * 0.5,
        clear_hit.rect.y + clear_hit.rect.h * 0.5,
    );
    assert_eq!(msgs, vec![Msg::Clear]);
}

#[test]
fn search_field_edits_utf8_through_event_routing() {
    let mut text = TextEngine::new();
    let mut interaction = InteractionState::default();
    let theme = Theme::dark();
    let mut state = TextInputState::default();
    let mut dl = DrawList::default();

    let frame = paint(
        search_field(&state, sf_spec(), &theme, &interaction, &mut text),
        (420.0, 80.0),
        1.0,
        &interaction,
        &mut text,
        &mut dl,
    );
    click(&frame, &mut interaction, 210.0, 26.0);

    for ch in ["2", "+", "2"] {
        let ev = key_event(Key::Char(ch.chars().next().unwrap()), Some(ch), false);
        let (msgs, _) = frame.dispatch_event(&ev, &mut interaction);
        assert_eq!(msgs.len(), 1);
        assert_eq!(state.apply_key(&ev), Some(TextInputAction::Changed));
    }
    assert_eq!(state.value, "2+2");

    // Multibyte: caret stays on char boundaries after left moves.
    let ev = key_event(Key::Char('√'), Some("√"), false);
    frame.dispatch_event(&ev, &mut interaction);
    state.apply_key(&ev);
    assert_eq!(state.value, "2+2√");
    assert!(state.value.is_char_boundary(state.cursor));
}

// ── password_field ───────────────────────────────────────────────────────────

fn pw_spec() -> PasswordField<Msg> {
    PasswordField {
        key: NodeKey::named("pw"),
        placeholder: "Password".to_owned(),
        on_toggle_reveal: Some(Box::new(|| Msg::Reveal)),
        on_submit: Some(Box::new(|| Msg::Submit)),
        on_event: Some(Box::new(|ev| Some(Msg::Input(ev.clone())))),
        ..Default::default()
    }
}

#[allow(clippy::too_many_arguments)]
fn pw_frame<'a>(
    state: &'a TextInputState,
    spec: PasswordField<Msg>,
    theme: &Theme,
    interaction: &InteractionState,
    anim: &'a mut Animator,
    text: &'a mut TextEngine,
    dl: &'a mut DrawList,
    size: (f32, f32),
) -> BuiltFrame<Msg> {
    paint(
        password_field(state, spec, theme, interaction, anim, text),
        size,
        1.0,
        interaction,
        text,
        dl,
    )
}

/// Hit entries carrying a click handler, ordered left-to-right.
fn click_hits(frame: &BuiltFrame<Msg>) -> Vec<Rect> {
    let mut rects: Vec<Rect> = frame
        .hits
        .iter()
        .filter(|h| h.on_click.is_some())
        .map(|h| h.rect)
        .collect();
    rects.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal));
    rects
}

/// Glyph positions+uvs of the three leftmost glyphs — the value strip (icon
/// glyphs sit at the trailing edge). Two different values must paint the same
/// shapes masked (all `•`) and different shapes revealed.
fn value_glyphs(dl: &DrawList, n: usize) -> Vec<(f32, [f32; 4])> {
    let mut gs = glyphs(dl);
    gs.sort_by(|a, b| {
        a.dst[0]
            .partial_cmp(&b.dst[0])
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    gs.iter().take(n).map(|g| (g.dst[0], g.uv)).collect()
}

/// Epsilon comparison of two `(x, uv)` glyph signature lists.
fn glyph_sigs_eq(a: &[(f32, [f32; 4])], b: &[(f32, [f32; 4])]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b.iter())
            .all(|((ax, auv), (bx, buv))| (ax - bx).abs() <= EPS && rgba_eq(*auv, *buv))
}

#[test]
fn password_field_masks_with_bullets_and_reveals_cleanly() {
    let mut text = TextEngine::new();
    let mut anim = Animator::new();
    let interaction = InteractionState::default();
    let theme = Theme::dark();
    anim.begin_frame(Instant::now(), false);
    let state = TextInputState::new("abc");

    // Masked: three identical bullet glyphs, caret measured on the dots.
    let mut dl = DrawList::default();
    let frame = pw_frame(
        &state,
        pw_spec(),
        &theme,
        &interaction,
        &mut anim,
        &mut text,
        &mut dl,
        (440.0, 90.0),
    );
    let rect = hit_rect(&frame, "pw");
    assert!((rect.h - 56.0).abs() < EPS, "Lock-board 56px pill");
    let masked_abc = value_glyphs(&dl, 3);

    // A different value paints the same bullet shapes when masked.
    let other = TextInputState::new("xyz");
    let mut dl = DrawList::default();
    pw_frame(
        &other,
        pw_spec(),
        &theme,
        &interaction,
        &mut anim,
        &mut text,
        &mut dl,
        (440.0, 90.0),
    );
    assert!(
        glyph_sigs_eq(&masked_abc, &value_glyphs(&dl, 3)),
        "masked values paint identical ••• dots"
    );

    // Focus and check the caret lands at the end of the masked strip.
    let mut interaction = InteractionState::default();
    click(&frame, &mut interaction, 120.0, 28.0);
    let mut dl = DrawList::default();
    pw_frame(
        &state,
        pw_spec(),
        &theme,
        &interaction,
        &mut anim,
        &mut text,
        &mut dl,
        (440.0, 90.0),
    );
    let caret = caret_quad(&dl, 1.0).expect("caret");
    let font = FontSpec::ui(17.0).weight(500);
    let dots = "•".repeat(3);
    let expected = 18.0 + text.measure_caret_x(&dots, dots.len(), &font) + 1.0;
    assert!(
        (caret.dst[0] - expected).abs() < 0.5,
        "caret measured on masked dots: got {}, want {expected}",
        caret.dst[0]
    );

    // Revealed: the same chars paint distinct glyphs.
    let mut dl = DrawList::default();
    pw_frame(
        &state,
        PasswordField {
            revealed: true,
            ..pw_spec()
        },
        &theme,
        &interaction,
        &mut anim,
        &mut text,
        &mut dl,
        (440.0, 90.0),
    );
    let revealed_abc = value_glyphs(&dl, 3);
    // Revealed "xyz" differs — masking was the only thing making them identical.
    let mut dl = DrawList::default();
    pw_frame(
        &TextInputState::new("xyz"),
        PasswordField {
            revealed: true,
            ..pw_spec()
        },
        &theme,
        &interaction,
        &mut anim,
        &mut text,
        &mut dl,
        (440.0, 90.0),
    );
    assert!(
        !glyph_sigs_eq(&revealed_abc, &value_glyphs(&dl, 3)),
        "revealed plaintext keeps per-char glyphs"
    );
}

#[test]
fn password_field_reveal_toggle_and_submit_disc() {
    let mut text = TextEngine::new();
    let mut anim = Animator::new();
    let mut interaction = InteractionState::default();
    let theme = Theme::dark();
    anim.begin_frame(Instant::now(), false);

    // Empty password: submit disc is disabled — muted fill, no click emission.
    let state = TextInputState::default();
    let mut dl = DrawList::default();
    let frame = pw_frame(
        &state,
        pw_spec(),
        &theme,
        &interaction,
        &mut anim,
        &mut text,
        &mut dl,
        (440.0, 90.0),
    );
    let disc = quads(&dl)
        .into_iter()
        .find(|q| {
            (q.dst[2] - 40.0).abs() < 0.01 && (q.dst[3] - 40.0).abs() < 0.01 && q.fill[3] > 0.05
        })
        .expect("40px submit disc");
    let expected = Rgba::from_color(theme.scheme.on_surface)
        .with_alpha(0.12)
        .to_array();
    assert_rgba(disc.fill, expected, "disabled disc container");

    let hits = click_hits(&frame);
    assert_eq!(hits.len(), 1, "only the eye toggle is clickable when empty");
    let msgs = click(
        &frame,
        &mut interaction,
        hits[0].x + hits[0].w * 0.5,
        hits[0].y + hits[0].h * 0.5,
    );
    assert_eq!(msgs, vec![Msg::Reveal]);
    // A click where the disabled disc sits emits nothing.
    let msgs = click(&frame, &mut interaction, 430.0, 28.0);
    assert!(msgs.is_empty(), "disabled submit emits nothing");

    // Non-empty: the disc goes primary and submits on click.
    let state = TextInputState::new("hunter2");
    let mut dl = DrawList::default();
    let frame = pw_frame(
        &state,
        pw_spec(),
        &theme,
        &interaction,
        &mut anim,
        &mut text,
        &mut dl,
        (440.0, 90.0),
    );
    let disc = quads(&dl)
        .into_iter()
        .find(|q| {
            (q.dst[2] - 40.0).abs() < 0.01 && (q.dst[3] - 40.0).abs() < 0.01 && q.fill[3] > 0.05
        })
        .expect("40px submit disc");
    assert_rgba(
        disc.fill,
        Rgba::from_color(theme.scheme.primary).to_array(),
        "enabled disc",
    );
    let hits = click_hits(&frame);
    assert_eq!(hits.len(), 2, "eye + submit are both clickable");
    let submit = hits[1];
    let msgs = click(
        &frame,
        &mut interaction,
        submit.x + submit.w * 0.5,
        submit.y + submit.h * 0.5,
    );
    assert_eq!(msgs, vec![Msg::Submit]);
}

#[test]
fn password_field_caps_lock_chip() {
    let mut text = TextEngine::new();
    let mut anim = Animator::new();
    let mut interaction = InteractionState::default();
    let theme = Theme::dark();
    anim.begin_frame(Instant::now(), false);
    let state = TextInputState::new("hunter2");

    // No caps lock → no warm chip.
    let mut dl = DrawList::default();
    let frame = pw_frame(
        &state,
        pw_spec(),
        &theme,
        &interaction,
        &mut anim,
        &mut text,
        &mut dl,
        (440.0, 110.0),
    );
    let warm = Rgba::from_color(theme.scheme.tertiary_container).to_array();
    assert!(
        !quads(&dl).iter().any(|q| rgba_eq(q.fill, warm)),
        "no caps chip without caps lock"
    );

    // A KeyDown carrying caps_lock syncs InteractionState; next frame shows the chip.
    let ev = key_event(Key::Char('x'), Some("x"), true);
    frame.dispatch_event(&ev, &mut interaction);
    assert!(interaction.caps_lock());

    let mut dl = DrawList::default();
    pw_frame(
        &state,
        pw_spec(),
        &theme,
        &interaction,
        &mut anim,
        &mut text,
        &mut dl,
        (440.0, 110.0),
    );
    let chip_quad = quads(&dl)
        .into_iter()
        .find(|q| rgba_eq(q.fill, warm) && (q.dst[3] - 36.0).abs() < 0.5)
        .expect("StatusWarm caps-lock chip");
    assert!(chip_quad.dst[1] > 56.0, "chip sits below the field");

    // Explicit override forces the chip even when caps lock is off.
    interaction.caps_lock = false;
    let mut dl = DrawList::default();
    pw_frame(
        &state,
        PasswordField {
            caps_lock_override: Some(true),
            ..pw_spec()
        },
        &theme,
        &interaction,
        &mut anim,
        &mut text,
        &mut dl,
        (440.0, 110.0),
    );
    assert!(
        quads(&dl)
            .iter()
            .any(|q| rgba_eq(q.fill, warm) && (q.dst[3] - 36.0).abs() < 0.5),
        "override shows the chip"
    );
}

#[test]
fn password_field_shake_oscillates_and_respects_reduce_motion() {
    let mut text = TextEngine::new();
    let mut anim = Animator::new();
    let interaction = InteractionState::default();
    let theme = Theme::dark();
    let state = TextInputState::new("hunter2");
    let t0 = Instant::now();

    anim.begin_frame(t0, false);
    let mut dl = DrawList::default();
    pw_frame(
        &state,
        pw_spec(),
        &theme,
        &interaction,
        &mut anim,
        &mut text,
        &mut dl,
        (440.0, 90.0),
    );
    let rest_x = field_quad(&dl, 440.0, 56.0, 1.0).dst[0];
    assert!(rest_x.abs() < EPS, "field rests at x=0");

    // Wrong password: trigger the shake, then repaint at two phases.
    trigger_password_shake(&mut anim, NodeKey::named("pw"));

    anim.begin_frame(t0 + Duration::from_millis(25), false);
    let mut dl = DrawList::default();
    pw_frame(
        &state,
        pw_spec(),
        &theme,
        &interaction,
        &mut anim,
        &mut text,
        &mut dl,
        (440.0, 90.0),
    );
    let early = field_quad(&dl, 440.0, 56.0, 1.0).dst[0];

    anim.begin_frame(t0 + Duration::from_millis(75), false);
    let mut dl = DrawList::default();
    pw_frame(
        &state,
        pw_spec(),
        &theme,
        &interaction,
        &mut anim,
        &mut text,
        &mut dl,
        (440.0, 90.0),
    );
    let late = field_quad(&dl, 440.0, 56.0, 1.0).dst[0];

    assert!(early.abs() > 0.5, "shake displaces the field: {early}");
    assert!(
        early.signum() != late.signum() && late.abs() > 0.5,
        "shake oscillates: {early} vs {late}"
    );

    // After ~300ms the field settles back at rest.
    anim.begin_frame(t0 + Duration::from_millis(400), false);
    let mut dl = DrawList::default();
    pw_frame(
        &state,
        pw_spec(),
        &theme,
        &interaction,
        &mut anim,
        &mut text,
        &mut dl,
        (440.0, 90.0),
    );
    let settled = field_quad(&dl, 440.0, 56.0, 1.0).dst[0];
    assert!(settled.abs() < EPS, "shake settles back to 0: {settled}");

    // reduce_motion: the trigger is a no-op and the field never moves.
    anim.begin_frame(t0, true);
    trigger_password_shake(&mut anim, NodeKey::named("pw"));
    for dt in [25, 75, 200] {
        anim.begin_frame(t0 + Duration::from_millis(dt), true);
        let mut dl = DrawList::default();
        pw_frame(
            &state,
            pw_spec(),
            &theme,
            &interaction,
            &mut anim,
            &mut text,
            &mut dl,
            (440.0, 90.0),
        );
        let x = field_quad(&dl, 440.0, 56.0, 1.0).dst[0];
        assert!(x.abs() < EPS, "reduce_motion never shakes: {x} @ {dt}ms");
    }
}

#[test]
fn password_field_error_ring_and_text() {
    let mut text = TextEngine::new();
    let mut anim = Animator::new();
    let interaction = InteractionState::default();
    let theme = Theme::light();
    anim.begin_frame(Instant::now(), false);
    let state = TextInputState::new("short");

    let mut dl = DrawList::default();
    pw_frame(
        &state,
        PasswordField {
            error: Some("Wrong password".to_owned()),
            ..pw_spec()
        },
        &theme,
        &interaction,
        &mut anim,
        &mut text,
        &mut dl,
        (440.0, 110.0),
    );
    let err = Rgba::from_color(theme.scheme.error).to_array();
    let field = field_quad(&dl, 440.0, 56.0, 1.0);
    assert_rgba(field.border, err, "error border on lock pill");
    assert!(
        glyphs(&dl).iter().filter(|g| rgba_eq(g.color, err)).count() >= 10,
        "error copy below the field"
    );
}
