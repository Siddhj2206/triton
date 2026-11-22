//! Text entry: the [`TextInputState`] editing model, the [`text_input`] field,
//! the launcher [`search_field`] with its mode-aware Wave disc, and the
//! [`password_field`] with masked dots, reveal toggle, submit disc, Caps Lock
//! warning, and the wrong-password [`trigger_password_shake`].
//!
//! Metrics come from `triton-design` tokens and the board reference
//! (`references/Triton Shell-project/triton/components.css`, `.t-field`;
//! `Lock.dc.html`; the Launcher `.hx` mode disc). Every field follows the same
//! focus grammar: a `2px` `primary` ring while focused (`error` when an error
//! is present), and a subtle `outline_variant` hairline at rest.

use triton_design::color::STATE_LAYER;
use triton_design::icon::{Icon, IconStyle};
use triton_design::shape::{CORNER_SMOOTHING, RADII, SIZE};
use triton_engine::anim::Animator;
use triton_engine::style::{Edges, FontSpec, Rgba, SubtreeTransform, Theme};
use triton_engine::text::TextEngine;
use triton_engine::ui::{
    ClickHandler, Element, InputEvent, InteractionState, Key, KeyHandler, NodeKey, box_, col, icon,
    row, text,
};

use crate::button::{ButtonSize, ButtonTone, Chip, ChipKind, IconButton, chip, icon_button};
use crate::chrome::{KeycapStyle, keycap};

// ── TextInputState ───────────────────────────────────────────────────────────

/// Single-line text input state with byte-safe UTF-8 cursor editing.
///
/// IME hook point: when `zwp_text_input_v3` is wired into
/// `triton_engine::platform`, preedit strings attach to `preedit` and
/// committed strings call [`Self::insert_str`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextInputState {
    pub value: String,
    /// Byte offset in `value` (always on a valid UTF-8 char boundary).
    pub cursor: usize,
    /// Optional IME preedit composition string (hook point for `text-input-v3`).
    pub preedit: Option<String>,
    pub caret_visible: bool,
}

impl Default for TextInputState {
    fn default() -> Self {
        Self::new("")
    }
}

/// Actions produced by [`TextInputState::apply_key`] keyboard handling.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TextInputAction {
    Changed,
    Submit,
    Escape,
}

impl TextInputState {
    pub fn new(initial: impl Into<String>) -> Self {
        let value = initial.into();
        let cursor = value.len();
        Self {
            value,
            cursor,
            preedit: None,
            caret_visible: true,
        }
    }

    pub fn insert_str(&mut self, s: &str) {
        let filtered: String = s.chars().filter(|c| !c.is_control()).collect();
        if filtered.is_empty() {
            return;
        }
        self.value.insert_str(self.cursor, &filtered);
        self.cursor += filtered.len();
        self.caret_visible = true;
    }

    pub fn backspace(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        let prev = prev_char_boundary(&self.value, self.cursor);
        self.value.drain(prev..self.cursor);
        self.cursor = prev;
        self.caret_visible = true;
        true
    }

    pub fn delete(&mut self) -> bool {
        if self.cursor >= self.value.len() {
            return false;
        }
        let next = next_char_boundary(&self.value, self.cursor);
        self.value.drain(self.cursor..next);
        self.caret_visible = true;
        true
    }

    pub fn move_left(&mut self) {
        self.cursor = prev_char_boundary(&self.value, self.cursor);
        self.caret_visible = true;
    }

    pub fn move_right(&mut self) {
        self.cursor = next_char_boundary(&self.value, self.cursor);
        self.caret_visible = true;
    }

    pub fn move_home(&mut self) {
        self.cursor = 0;
        self.caret_visible = true;
    }

    pub fn move_end(&mut self) {
        self.cursor = self.value.len();
        self.caret_visible = true;
    }

    pub fn clear(&mut self) {
        self.value.clear();
        self.cursor = 0;
        self.caret_visible = true;
    }

    /// Apply a keyboard [`InputEvent`], returning what happened if handled.
    pub fn apply_key(&mut self, event: &InputEvent) -> Option<TextInputAction> {
        let InputEvent::KeyDown {
            key, utf8, ctrl, ..
        } = event
        else {
            return None;
        };
        match key {
            Key::Escape => Some(TextInputAction::Escape),
            Key::Enter => Some(TextInputAction::Submit),
            Key::BackSpace => {
                if *ctrl {
                    self.clear();
                    Some(TextInputAction::Changed)
                } else if self.backspace() {
                    Some(TextInputAction::Changed)
                } else {
                    None
                }
            }
            Key::Delete => self.delete().then_some(TextInputAction::Changed),
            Key::Left => {
                self.move_left();
                Some(TextInputAction::Changed)
            }
            Key::Right => {
                self.move_right();
                Some(TextInputAction::Changed)
            }
            Key::Home => {
                self.move_home();
                Some(TextInputAction::Changed)
            }
            Key::End => {
                self.move_end();
                Some(TextInputAction::Changed)
            }
            _ => {
                if *ctrl {
                    return None;
                }
                if let Some(text) = utf8
                    && !text.is_empty()
                {
                    let before = self.value.len();
                    self.insert_str(text);
                    if self.value.len() != before {
                        return Some(TextInputAction::Changed);
                    }
                }
                None
            }
        }
    }
}

fn prev_char_boundary(s: &str, mut idx: usize) -> usize {
    idx = idx.min(s.len());
    if idx == 0 {
        return 0;
    }
    idx -= 1;
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

fn next_char_boundary(s: &str, mut idx: usize) -> usize {
    if idx >= s.len() {
        return s.len();
    }
    idx += 1;
    while idx < s.len() && !s.is_char_boundary(idx) {
        idx += 1;
    }
    idx
}

// ── Shared field pieces ──────────────────────────────────────────────────────

/// Field body type: 17px/500 UI text (the `.t-field` query grammar).
fn field_font() -> FontSpec {
    FontSpec::ui(17.0).weight(500).line_height(22.0)
}

/// The field border grammar: `2px` `primary` while focused, `error` whenever
/// an error is present, otherwise a subtle `outline_variant` hairline.
fn field_border(focused: bool, has_error: bool, theme: &Theme) -> (f32, Rgba) {
    let s = &theme.scheme;
    if has_error {
        (if focused { 2.0 } else { 1.0 }, Rgba::from_color(s.error))
    } else if focused {
        (2.0, Rgba::from_color(s.primary))
    } else {
        (1.0, Rgba::from_color(s.outline_variant).with_alpha(0.55))
    }
}

/// The measured-caret text strip: placeholder ghost text when the value is
/// empty, otherwise the shown string, with a `2px` `primary` caret at the
/// measured cursor position while focused.
fn field_text<M>(
    state: &TextInputState,
    shown: &str,
    caret_byte: usize,
    focused: bool,
    theme: &Theme,
    text_engine: &mut TextEngine,
) -> Element<M> {
    let font = field_font();
    let text_color = if state.value.is_empty() {
        Rgba::from_color(theme.scheme.on_surface_variant).with_alpha(0.72)
    } else {
        Rgba::from_color(theme.scheme.on_surface)
    };

    let caret_x = (focused && state.caret_visible)
        .then(|| text_engine.measure_caret_x(shown, caret_byte, &font));

    let mut strip = box_().flex_1().h(26.0).items_center().clip().child(text(
        shown.to_owned(),
        font,
        text_color,
    ));

    if let Some(caret_x) = caret_x {
        strip = strip.caret(caret_x + 1.0, 22.0, Rgba::from_color(theme.scheme.primary));
    }
    strip
}

/// 13px/500 supporting line under a field (`scheme.error` for errors).
fn error_text<M>(message: &str, theme: &Theme) -> Element<M> {
    text(
        message.to_owned(),
        FontSpec::ui(13.0).weight(500),
        Rgba::from_color(theme.scheme.error),
    )
}

// ── text_input ───────────────────────────────────────────────────────────────

/// Content and behavior of a [`text_input`] field.
pub struct TextInput<M> {
    /// Hit-test, focus, and key-routing key.
    pub key: NodeKey,
    /// Ghost text shown while the value is empty.
    pub placeholder: String,
    /// Optional leading glyph, tinted `primary` while focused.
    pub lead_icon: Option<Icon>,
    /// Corner radius in logical px. `RADII.full` (the default) renders the
    /// `52px` `.t-field` pill; pass a smaller concentric radius when the field
    /// sits inside a rounded panel.
    pub radius: f32,
    /// Inline error text below the field; also swaps the border to `error`.
    pub error: Option<String>,
    /// Keyboard events routed while the field is focused — feed them to
    /// [`TextInputState::apply_key`] in `update`.
    pub on_event: Option<KeyHandler<M>>,
}

impl<M> Default for TextInput<M> {
    fn default() -> Self {
        Self {
            key: NodeKey::named("triton-text-input"),
            placeholder: String::new(),
            lead_icon: None,
            radius: RADII.full,
            error: None,
            on_event: None,
        }
    }
}

impl<M> std::fmt::Debug for TextInput<M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextInput")
            .field("key", &self.key)
            .field("placeholder", &self.placeholder)
            .field("lead_icon", &self.lead_icon)
            .field("radius", &self.radius)
            .field("error", &self.error)
            .field("on_event", &self.on_event.is_some())
            .finish()
    }
}

/// `.t-field` single-line text input: a `52px` veil pill with a `2px`
/// `primary` focus ring (`error` when [`TextInput::error`] is set), optional
/// leading icon, live measured caret, and optional inline error text below.
///
/// The field only *routes* [`InputEvent`]s through [`TextInput::on_event`];
/// the caller applies them to its [`TextInputState`] (via
/// [`TextInputState::apply_key`]) in `update`, keeping all editing state in
/// app-land.
pub fn text_input<M: 'static>(
    state: &TextInputState,
    spec: TextInput<M>,
    theme: &Theme,
    interaction: &InteractionState,
    text_engine: &mut TextEngine,
) -> Element<M> {
    let focused = interaction.is_focused(spec.key);
    let (border_w, border_col) = field_border(focused, spec.error.is_some(), theme);

    // Pills take true semicircle endcaps (smoothing 0); smaller radii keep the
    // squircle smoothing that matches surrounding panels.
    let smoothing = if spec.radius >= SIZE.search_field * 0.5 {
        0.0
    } else {
        CORNER_SMOOTHING
    };

    let shown = if state.value.is_empty() {
        spec.placeholder.clone()
    } else {
        state.value.clone()
    };

    let mut field = row()
        .id(spec.key)
        .w_full()
        .h(SIZE.search_field)
        .px(16.0)
        .gap(12.0)
        .items_center()
        .rounded(spec.radius)
        .smoothing(smoothing)
        .bg(theme.veil())
        .hover_bg(theme.veil_hi())
        .border(border_w, border_col)
        .focusable()
        .child_opt(spec.lead_icon.map(|ic| {
            icon(
                ic,
                IconStyle::Outlined,
                22.0,
                if focused {
                    Rgba::from_color(theme.scheme.primary)
                } else {
                    Rgba::from_color(theme.scheme.on_surface_variant)
                },
            )
        }))
        .child(field_text(
            state,
            &shown,
            state.cursor,
            focused,
            theme,
            text_engine,
        ));

    if let Some(on_event) = spec.on_event {
        field = field.on_key(on_event);
    }

    col()
        .w_full()
        .gap(6.0)
        .child(field)
        .child_opt(spec.error.map(|e| error_text(&e, theme).px(16.0)))
}

// ── search_field ─────────────────────────────────────────────────────────────

/// Which launcher grammar a [`search_field`] is querying — drives the leading
/// Wave disc glyph and tint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum SearchMode {
    /// Free-text app/setting search.
    #[default]
    Search,
    /// `=` math evaluation.
    Math,
    /// `>` shell command.
    Command,
    /// File-browser query.
    File,
}

impl SearchMode {
    /// The Wave-disc glyph for this mode (Launcher `.hx` grammar).
    #[must_use]
    pub fn icon(self) -> Icon {
        match self {
            Self::Search => Icon::Search,
            Self::Math => Icon::Calculate,
            Self::Command => Icon::Terminal,
            Self::File => Icon::Folder,
        }
    }

    /// Mode-tinted `(container, content)` pair for the Wave disc.
    fn colors(self, theme: &Theme) -> (Rgba, Rgba) {
        let s = &theme.scheme;
        let (bg, fg) = match self {
            Self::Search => (s.primary_container, s.on_primary_container),
            Self::Math => (s.tertiary_container, s.on_tertiary_container),
            Self::Command => (s.secondary_container, s.on_secondary_container),
            Self::File => (s.surface_container_highest, s.on_surface_variant),
        };
        (Rgba::from_color(bg), Rgba::from_color(fg))
    }
}

/// Content and behavior of a [`search_field`].
pub struct SearchField<M> {
    /// Hit-test, focus, and key-routing key.
    pub key: NodeKey,
    /// Ghost text shown while the value is empty.
    pub placeholder: String,
    /// Mode driving the leading Wave disc glyph + tint.
    pub mode: SearchMode,
    /// Right-aligned [`keycap`] hint label (e.g. `"Tab"`), `KeycapStyle::Flat`.
    pub keycap_hint: Option<String>,
    /// Clear affordance shown only while the value is non-empty.
    pub on_clear: Option<ClickHandler<M>>,
    /// Keyboard events routed while the field is focused — feed them to
    /// [`TextInputState::apply_key`] in `update`.
    pub on_event: Option<KeyHandler<M>>,
}

impl<M> Default for SearchField<M> {
    fn default() -> Self {
        Self {
            key: NodeKey::named("triton-search-field"),
            placeholder: String::new(),
            mode: SearchMode::Search,
            keycap_hint: None,
            on_clear: None,
            on_event: None,
        }
    }
}

impl<M> std::fmt::Debug for SearchField<M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SearchField")
            .field("key", &self.key)
            .field("placeholder", &self.placeholder)
            .field("mode", &self.mode)
            .field("keycap_hint", &self.keycap_hint)
            .field("on_clear", &self.on_clear.is_some())
            .field("on_event", &self.on_event.is_some())
            .finish()
    }
}

/// Key under which the clear button registers in hit-testing (derived from the
/// field key, so two search fields on one surface never collide).
const SEARCH_CLEAR_SALT: u64 = 0x5ea2_c11e_a700_0001;

/// The `52px` launcher search pill: mode-aware `40px` Wave icon disc (the
/// `.hx` grammar — 9-lobe SDF, never a circle), veil background, `primary`
/// focus ring, live caret, optional trailing clear button and [`keycap`] hint.
pub fn search_field<M: 'static>(
    state: &TextInputState,
    spec: SearchField<M>,
    theme: &Theme,
    interaction: &InteractionState,
    text_engine: &mut TextEngine,
) -> Element<M> {
    let focused = interaction.is_focused(spec.key);
    let (border_w, border_col) = field_border(focused, false, theme);
    let (disc_bg, disc_fg) = spec.mode.colors(theme);

    let disc = box_()
        .size(SIZE.control_m, SIZE.control_m)
        .items_center()
        .justify_center()
        .wave()
        .bg(disc_bg)
        .child(icon(
            spec.mode.icon(),
            IconStyle::Outlined,
            SIZE.icon_l,
            disc_fg,
        ));

    let shown = if state.value.is_empty() {
        spec.placeholder.clone()
    } else {
        state.value.clone()
    };

    let mut field = row()
        .id(spec.key)
        .w_full()
        .h(SIZE.search_field)
        .p(Edges::new(0.0, 12.0, 0.0, 8.0))
        .gap(12.0)
        .items_center()
        .rounded(RADII.full)
        .smoothing(0.0)
        .bg(theme.veil())
        .hover_bg(theme.veil_hi())
        .border(border_w, border_col)
        .focusable()
        .child(disc)
        .child(field_text(
            state,
            &shown,
            state.cursor,
            focused,
            theme,
            text_engine,
        ));

    if !state.value.is_empty()
        && let Some(on_clear) = spec.on_clear
    {
        let fg = Rgba::from_color(theme.scheme.on_surface_variant);
        field = field.child(
            box_()
                .id(NodeKey(spec.key.0 ^ SEARCH_CLEAR_SALT))
                .size(36.0, 36.0)
                .items_center()
                .justify_center()
                .rounded(18.0)
                .smoothing(0.0)
                .hover_bg(fg.with_alpha(STATE_LAYER.hover))
                .press_bg(fg.with_alpha(STATE_LAYER.press))
                .on_click(on_clear)
                .child(icon(Icon::Close, IconStyle::Outlined, SIZE.icon_s, fg)),
        );
    }
    if let Some(hint) = spec.keycap_hint {
        field = field.child(keycap(hint, KeycapStyle::Flat, theme));
    }
    if let Some(on_event) = spec.on_event {
        field = field.on_key(on_event);
    }
    field
}

// ── password_field ───────────────────────────────────────────────────────────

/// Wrong-password horizontal shake amplitude (a brisk ~8px nudge, ~300ms).
pub const PASSWORD_SHAKE_AMPLITUDE: f32 = 8.0;

/// Bullet used to mask each character of a [`password_field`] value.
const MASK: char = '•';

/// Content and behavior of a [`password_field`].
pub struct PasswordField<M> {
    /// Hit-test, focus, key-routing, and shake key.
    pub key: NodeKey,
    /// Ghost text shown while the value is empty.
    pub placeholder: String,
    /// `true` reveals the plaintext; `false` (default) masks every character
    /// as `•` and measures the caret against the dot string so it still lands
    /// at the exact visual position.
    pub revealed: bool,
    /// Inline error text below the field; also swaps the border to `error`.
    pub error: Option<String>,
    /// Overrides [`InteractionState::caps_lock`] for the Caps Lock chip (e.g.
    /// when the warning should be forced in tests or simulated sessions).
    pub caps_lock_override: Option<bool>,
    /// Eye icon-button emitting the reveal toggle — the caller flips
    /// [`PasswordField::revealed`] in `update`.
    pub on_toggle_reveal: Option<ClickHandler<M>>,
    /// `arrow_forward` submit disc: enabled only while the password is
    /// non-empty (disabled paints M3 disabled alphas and emits nothing).
    pub on_submit: Option<ClickHandler<M>>,
    /// Keyboard events routed while the field is focused — feed them to
    /// [`TextInputState::apply_key`] in `update`.
    pub on_event: Option<KeyHandler<M>>,
}

impl<M> Default for PasswordField<M> {
    fn default() -> Self {
        Self {
            key: NodeKey::named("triton-password-field"),
            placeholder: String::new(),
            revealed: false,
            error: None,
            caps_lock_override: None,
            on_toggle_reveal: None,
            on_submit: None,
            on_event: None,
        }
    }
}

impl<M> std::fmt::Debug for PasswordField<M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PasswordField")
            .field("key", &self.key)
            .field("placeholder", &self.placeholder)
            .field("revealed", &self.revealed)
            .field("error", &self.error)
            .field("caps_lock_override", &self.caps_lock_override)
            .field("on_toggle_reveal", &self.on_toggle_reveal.is_some())
            .field("on_submit", &self.on_submit.is_some())
            .field("on_event", &self.on_event.is_some())
            .finish()
    }
}

/// Channel salt for the shake animation (XOR keeps it unique per field key).
const SHAKE_SALT: u64 = 0x5a1e_5a1e_0000_0002;

/// Trigger the wrong-password horizontal shake on the [`password_field`]
/// keyed `key`. Reads back via [`Animator::shake_offset`] inside
/// `password_field`; a no-op under `reduce_motion`.
pub fn trigger_password_shake(anim: &mut Animator, key: impl Into<NodeKey>) {
    anim.trigger_shake(NodeKey(key.into().0 ^ SHAKE_SALT), PASSWORD_SHAKE_AMPLITUDE);
}

/// Masked display string and the byte offset of `cursor` within it: one `•`
/// per character keeps the caret measurement at the exact visual position.
fn masked_view(value: &str, cursor: usize) -> (String, usize) {
    let cursor = cursor.min(value.len());
    let head = value.get(..cursor).unwrap_or_default();
    let chars_before = head.chars().count();
    (
        MASK.to_string().repeat(value.chars().count()),
        chars_before * MASK.len_utf8(),
    )
}

/// Trailing controls for [`password_field`]: the eye reveal toggle (depicts
/// the action — `visibility` while masked, `visibility_off` once revealed)
/// and the `arrow_forward` submit disc, disabled at M3 alphas while the
/// password is empty.
#[expect(
    clippy::too_many_arguments,
    reason = "helper packs the field key, two state flags, both click handlers, \
              theme, interaction and animator — each a distinct caller-owned input"
)]
fn password_trailing<M: 'static>(
    key: NodeKey,
    masked: bool,
    empty: bool,
    on_toggle_reveal: Option<ClickHandler<M>>,
    on_submit: Option<ClickHandler<M>>,
    theme: &Theme,
    interaction: &InteractionState,
    anim: &mut Animator,
) -> (Option<Element<M>>, Option<Element<M>>) {
    let eye = on_toggle_reveal.map(|on_click| {
        icon_button(
            theme,
            interaction,
            anim,
            IconButton {
                key: NodeKey(key.0 ^ EYE_SALT),
                icon: if masked {
                    Icon::Visibility
                } else {
                    Icon::VisibilityOff
                },
                tone: ButtonTone::Text,
                size: ButtonSize::M,
                on_click: Some(on_click),
                ..Default::default()
            },
        )
    });
    let submit = on_submit.map(|on_click| {
        icon_button(
            theme,
            interaction,
            anim,
            IconButton {
                key: NodeKey(key.0 ^ SUBMIT_SALT),
                icon: Icon::ArrowForward,
                tone: ButtonTone::Filled,
                size: ButtonSize::M,
                disabled: empty,
                on_click: Some(on_click),
                ..Default::default()
            },
        )
    });
    (eye, submit)
}

/// The Lock-board `56px` password pill: veil background, `2px` `primary`
/// focus ring (`error` on error), `•`-masked dots with a live caret, trailing
/// eye reveal toggle + `arrow_forward` submit disc, a Caps Lock warning
/// [`chip`], optional inline error text, and the wrong-password shake driven
/// through `anim` (see [`trigger_password_shake`]).
pub fn password_field<M: 'static>(
    state: &TextInputState,
    spec: PasswordField<M>,
    theme: &Theme,
    interaction: &InteractionState,
    anim: &mut Animator,
    text_engine: &mut TextEngine,
) -> Element<M> {
    let focused = interaction.is_focused(spec.key);
    let (border_w, border_col) = field_border(focused, spec.error.is_some(), theme);

    let masked = !spec.revealed;
    let (shown, caret_byte) = if masked && !state.value.is_empty() {
        masked_view(&state.value, state.cursor)
    } else if state.value.is_empty() {
        (spec.placeholder.clone(), state.cursor)
    } else {
        (state.value.clone(), state.cursor)
    };

    let shake = anim.shake_offset(NodeKey(spec.key.0 ^ SHAKE_SALT));
    let (eye, submit) = password_trailing(
        spec.key,
        masked,
        state.value.is_empty(),
        spec.on_toggle_reveal,
        spec.on_submit,
        theme,
        interaction,
        anim,
    );

    let mut field = row()
        .id(spec.key)
        .w_full()
        .h(SIZE.control_xl)
        .p(Edges::new(0.0, 8.0, 0.0, 18.0))
        .gap(12.0)
        .items_center()
        .rounded(RADII.full)
        .smoothing(0.0)
        .bg(theme.veil())
        .hover_bg(theme.veil_hi())
        .border(border_w, border_col)
        .transform(SubtreeTransform {
            translate_x: shake,
            ..SubtreeTransform::IDENTITY
        })
        .focusable()
        .child(field_text(
            state,
            &shown,
            caret_byte,
            focused,
            theme,
            text_engine,
        ))
        .child_opt(eye)
        .child_opt(submit);

    if let Some(on_event) = spec.on_event {
        field = field.on_key(on_event);
    }

    // Under-field row: Caps Lock chip (warm status) and/or inline error text.
    let caps_on = spec
        .caps_lock_override
        .unwrap_or_else(|| interaction.caps_lock());
    let chip_el = caps_on.then(|| {
        chip(
            theme,
            Chip::<M> {
                key: NodeKey(spec.key.0 ^ CAPS_SALT),
                label: "Caps Lock".to_owned(),
                icon: Some(Icon::KeyboardCapslock),
                kind: ChipKind::StatusWarm,
                ..Default::default()
            },
        )
    });
    let error_el = spec.error.map(|e| error_text(&e, theme));

    let mut root = col().w_full().gap(6.0).child(field);
    if chip_el.is_some() || error_el.is_some() {
        root = root.child(
            row()
                .px(18.0)
                .gap(8.0)
                .items_center()
                .child_opt(chip_el)
                .child_opt(error_el),
        );
    }
    root
}

/// Key salts for the reveal toggle, submit disc, and Caps Lock chip.
const EYE_SALT: u64 = 0x5a1e_5a1e_0000_0003;
const SUBMIT_SALT: u64 = 0x5a1e_5a1e_0000_0004;
const CAPS_SALT: u64 = 0x5a1e_5a1e_0000_0005;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_input_editing_and_cursor_movement() {
        let mut ti = TextInputState::new("hel");
        assert_eq!(ti.cursor, 3);
        ti.insert_str("lo");
        assert_eq!(ti.value, "hello");
        ti.move_left();
        ti.move_left();
        ti.insert_str("X");
        assert_eq!(ti.value, "helXlo");
        assert!(ti.backspace());
        assert_eq!(ti.value, "hello");
        ti.move_home();
        assert!(ti.delete());
        assert_eq!(ti.value, "ello");
        ti.move_end();
        ti.insert_str("!");
        assert_eq!(ti.value, "ello!");
    }

    #[test]
    fn text_input_handles_utf8_boundaries() {
        let mut ti = TextInputState::new("café");
        ti.move_left();
        assert!(ti.value.is_char_boundary(ti.cursor));
        ti.insert_str("e");
        assert_eq!(ti.value, "cafeé");
        assert!(ti.backspace());
        assert_eq!(ti.value, "café");
    }
}
