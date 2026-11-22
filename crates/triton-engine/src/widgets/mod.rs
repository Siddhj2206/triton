//! Core Triton Material 3 Futura widgets built 100% in safe Rust on [`crate::ui`].
//!
//! Includes:
//! - [`panel`] & [`island`] (Glass / Thick / Solid containers with concentric radii + blur reporting)
//! - [`button`], [`icon_button`], [`pill`] (state layers + spring morphing)
//! - [`TextInputState`] & [`text_input`] (caret, movement, editing, placeholder, IME hook point)
//! - [`slider`] (M3 Expressive split-track horizontal slider with drag support)
//! - [`SwitchState`] & [`switch`] (spring-animated M3 toggle switch with check icon)
//! - [`scroll_area`] (clipped vertical scroll container with wheel support)

use std::time::Instant;

use triton_design::color::STATE_LAYER;
use triton_design::icon::{Icon, IconStyle};
use triton_design::material::{Material, ShadowToken};
use triton_design::motion::{Animated, Motion, Spring};
use triton_design::shape::{CORNER_SMOOTHING, RADII, SIZE, nested};
use triton_design::typography::TypeStyle;

use crate::style::{Corners, Edges, FontSpec, Rgba, Theme};
use crate::text::TextEngine;
use crate::ui::{
    Element, InputEvent, InteractionState, Key, NodeKey, TextAlign, box_, col, icon, row, text,
};

// ── Panels & Islands ─────────────────────────────────────────────────────────

/// Floating or solid Triton panel with automatic material tint, 1px top highlight,
/// continuous smooth corners, optional soft shadow, and compositor blur reporting.
pub fn panel<M>(material: Material, radius: f32, theme: &Theme) -> Element<M> {
    let mut el = col()
        .material(material, theme)
        .rounded(radius)
        .smoothing(CORNER_SMOOTHING);
    if let Some(sh) = material.shadow() {
        el = el.shadow(sh);
    }
    el
}

/// Top-bar floating Glass island (pill-shaped, 56px or custom height, circular endcaps).
pub fn island<M>(theme: &Theme) -> Element<M> {
    row()
        .h(SIZE.bar_height)
        .px(6.0)
        .gap(4.0)
        .items_center()
        .material(Material::Glass, theme)
        .rounded(RADII.full)
        .smoothing(0.0) // full pill uses true semicircular endcaps
        .shadow(ShadowToken::FLOATING)
}

/// Helper to compute concentric inner radius: `outer - inset` (never negative).
pub fn concentric_inner(outer_radius: f32, padding: f32) -> f32 {
    nested(outer_radius, padding)
}

// ── Buttons & Pills ──────────────────────────────────────────────────────────

/// Visual emphasis of a Triton [`button`] or [`icon_button`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonTone {
    Filled,
    Tonal,
    Veil,
    Ghost,
}

/// Triton M3 pill button with hover/press state layer and click callback.
pub fn button<M: 'static>(
    key: impl Into<NodeKey>,
    label: impl Into<String>,
    lead_icon: Option<Icon>,
    tone: ButtonTone,
    theme: &Theme,
    on_click: impl Fn() -> M + 'static,
) -> Element<M> {
    let (bg, fg) = tone_colors(tone, theme);
    let hover = bg.over(fg, STATE_LAYER.hover);
    let press = bg.over(fg, STATE_LAYER.press);

    let mut el = row()
        .id(key)
        .h(SIZE.control_m)
        .px(18.0)
        .gap(8.0)
        .items_center()
        .justify_center()
        .rounded(RADII.lg)
        .bg(bg)
        .hover_bg(hover)
        .press_bg(press)
        .on_click(on_click);

    if let Some(ic) = lead_icon {
        el = el.child(icon(ic, IconStyle::Outlined, SIZE.icon_m, fg));
    }
    el.child(text(label, TypeStyle::LABEL_L, fg))
}

/// Circular or squircle-morphing icon button (e.g., launcher button, settings gear, power).
#[allow(clippy::too_many_arguments)]
pub fn icon_button<M: 'static>(
    key: impl Into<NodeKey>,
    glyph: Icon,
    style: IconStyle,
    diameter: f32,
    toggled: bool,
    tone: ButtonTone,
    theme: &Theme,
    on_click: impl Fn() -> M + 'static,
) -> Element<M> {
    let (bg, fg) = if toggled {
        (
            Rgba::from_color(theme.scheme.primary),
            Rgba::from_color(theme.scheme.on_primary),
        )
    } else {
        tone_colors(tone, theme)
    };
    let hover = bg.over(fg, STATE_LAYER.hover);
    let press = bg.over(fg, STATE_LAYER.press);
    let radius = if toggled { RADII.sm } else { diameter * 0.5 };

    box_()
        .id(key)
        .size(diameter, diameter)
        .items_center()
        .justify_center()
        .rounded(radius)
        .smoothing(if toggled { CORNER_SMOOTHING } else { 0.0 })
        .bg(bg)
        .hover_bg(hover)
        .press_bg(press)
        .on_click(on_click)
        .child(icon(glyph, style, (diameter * 0.5).clamp(18.0, 24.0), fg))
}

/// Spring-morphing workspace/status pill (width, corner radius, and color smoothly
/// interpolate with `morph` in `0.0..=1.0`).
#[allow(clippy::too_many_arguments)]
pub fn pill<M: 'static>(
    key: impl Into<NodeKey>,
    label: impl Into<String>,
    morph: f32,
    idle_w: f32,
    active_w: f32,
    height: f32,
    theme: &Theme,
    on_click: impl Fn() -> M + 'static,
) -> Element<M> {
    let m = morph.clamp(0.0, 1.0);
    let w = idle_w + (active_w - idle_w) * m;
    let idle_bg = theme.veil();
    let active_bg = Rgba::from_color(theme.scheme.primary);
    let bg = idle_bg.lerp(active_bg, m);
    let fg = Rgba::from_color(theme.scheme.on_surface_variant)
        .lerp(Rgba::from_color(theme.scheme.on_primary), m);
    let hover = bg.over(fg, STATE_LAYER.hover);
    let press = bg.over(fg, STATE_LAYER.press);
    let radius = height * 0.5 - (height * 0.5 - 12.0) * m;

    let mut font = FontSpec::display(16.0);
    font.weight = if m > 0.5 { 600 } else { 500 };

    box_()
        .id(key)
        .size(w, height)
        .items_center()
        .justify_center()
        .rounded(radius)
        .smoothing(CORNER_SMOOTHING * m)
        .bg(bg)
        .hover_bg(hover)
        .press_bg(press)
        .on_click(on_click)
        .child(text(label, font, fg).text_align(TextAlign::Center))
}

fn tone_colors(tone: ButtonTone, theme: &Theme) -> (Rgba, Rgba) {
    match tone {
        ButtonTone::Filled => (
            Rgba::from_color(theme.scheme.primary),
            Rgba::from_color(theme.scheme.on_primary),
        ),
        ButtonTone::Tonal => (
            Rgba::from_color(theme.scheme.secondary_container),
            Rgba::from_color(theme.scheme.on_secondary_container),
        ),
        ButtonTone::Veil => (theme.veil(), Rgba::from_color(theme.scheme.on_surface)),
        ButtonTone::Ghost => (
            Rgba::TRANSPARENT,
            Rgba::from_color(theme.scheme.on_surface_variant),
        ),
    }
}

// ── TextInput ────────────────────────────────────────────────────────────────

/// Single-line text input state with byte-safe UTF-8 cursor editing.
///
/// IME hook point: when `zwp_text_input_v3` is wired into [`crate::platform`],
/// preedit strings attach to `preedit` and committed strings call [`Self::insert_str`].
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

/// Actions produced by [`text_input`] keyboard handling.
#[derive(Debug, Clone, PartialEq, Eq)]
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

/// Build a styled single-line Triton search/text field with live caret measurement.
#[allow(clippy::too_many_arguments)]
pub fn text_input<M: 'static>(
    key: impl Into<NodeKey>,
    state: &TextInputState,
    placeholder: &str,
    lead_icon: Option<Icon>,
    radius: f32,
    theme: &Theme,
    interaction: &InteractionState,
    text_engine: &mut TextEngine,
    on_event: impl Fn(InputEvent) -> M + 'static,
) -> Element<M> {
    let key = key.into();
    let focused = interaction.is_focused(key);
    let font = FontSpec::ui(17.0).weight(500).line_height(22.0);
    let caret_x = text_engine.measure_caret_x(&state.value, state.cursor, &font);

    let border_col = if focused {
        Rgba::from_color(theme.scheme.primary).with_alpha(0.75)
    } else {
        Rgba::from_color(theme.scheme.outline_variant).with_alpha(0.55)
    };

    let (shown, text_color) = if state.value.is_empty() {
        (
            placeholder.to_string(),
            Rgba::from_color(theme.scheme.on_surface_variant).with_alpha(0.72),
        )
    } else {
        (
            state.value.clone(),
            Rgba::from_color(theme.scheme.on_surface),
        )
    };

    let mut text_box = box_()
        .flex_1()
        .h(26.0)
        .items_center()
        .clip()
        .child(text(shown, font, text_color));

    if focused && state.caret_visible {
        text_box = text_box.caret(caret_x + 1.0, 22.0, Rgba::from_color(theme.scheme.primary));
    }

    let mut field = row()
        .id(key)
        .w_full()
        .h(SIZE.search_field)
        .px(16.0)
        .gap(12.0)
        .items_center()
        .rounded(radius)
        .smoothing(CORNER_SMOOTHING)
        .bg(theme.veil())
        .hover_bg(theme.veil_hi())
        .border(if focused { 1.5 } else { 1.0 }, border_col)
        .focusable()
        .on_key(move |ev| Some(on_event(ev.clone())));

    if let Some(ic) = lead_icon {
        field = field.child(icon(
            ic,
            IconStyle::Outlined,
            22.0,
            if focused {
                Rgba::from_color(theme.scheme.primary)
            } else {
                Rgba::from_color(theme.scheme.on_surface_variant)
            },
        ));
    }

    field.child(text_box)
}

// ── Slider ───────────────────────────────────────────────────────────────────

/// M3 Expressive split-track horizontal slider (active track, 4px vertical handle,
/// inactive track) matching `QuickSettings.dc.html` and `Controls.dc.html`.
#[allow(clippy::too_many_arguments)]
pub fn slider<M: 'static>(
    key: impl Into<NodeKey>,
    value: f32,
    width: f32,
    theme: &Theme,
    on_change: impl Fn(f32) -> M + 'static,
) -> Element<M> {
    let t = value.clamp(0.0, 1.0);
    let track_h = SIZE.slider_track; // 16px
    let (handle_w, handle_h) = SIZE.slider_handle; // 4x44px
    let gap = 6.0;
    let total_w = width.max(60.0);
    let handle_x = (t * (total_w - handle_w)).clamp(0.0, total_w - handle_w);
    let active_w = (handle_x - gap).max(0.0);
    let inactive_x = (handle_x + handle_w + gap).min(total_w);
    let inactive_w = (total_w - inactive_x).max(0.0);
    let track_y = (handle_h - track_h) * 0.5;

    let active_col = Rgba::from_color(theme.scheme.primary);
    let inactive_col = Rgba::new(1.0, 1.0, 1.0, 0.18);

    let mut root = box_()
        .id(key)
        .size(total_w, handle_h)
        .on_drag(move |px, _py, rect| {
            let v = ((px - rect.x) / rect.w.max(1.0)).clamp(0.0, 1.0);
            on_change(v)
        });

    if active_w > 1.0 {
        root = root.child(
            box_()
                .absolute()
                .inset_xy(0.0, track_y)
                .size(active_w, track_h)
                .rounded(Corners::each(8.0, 2.0, 2.0, 8.0))
                .smoothing(0.0)
                .bg(active_col),
        );
    }

    if inactive_w > 1.0 {
        root = root.child(
            box_()
                .absolute()
                .inset_xy(inactive_x, track_y)
                .size(inactive_w, track_h)
                .rounded(Corners::each(2.0, 8.0, 8.0, 2.0))
                .smoothing(0.0)
                .bg(inactive_col),
        );
    }

    root.child(
        box_()
            .absolute()
            .inset_xy(handle_x, 0.0)
            .size(handle_w, handle_h)
            .rounded(2.0)
            .smoothing(0.0)
            .bg(active_col),
    )
}

// ── Switch ───────────────────────────────────────────────────────────────────

/// Spring-backed state for a Material 3 toggle [`switch`].
#[derive(Debug, Clone)]
pub struct SwitchState {
    pub checked: bool,
    pub anim: Animated,
}

impl SwitchState {
    pub fn new(checked: bool, now: Instant) -> Self {
        let initial = f32::from(u8::from(checked));
        Self {
            checked,
            anim: Animated::new(initial, Motion::Spring(Spring::SPATIAL_FAST), 0.005, now),
        }
    }

    pub fn toggle(&mut self, now: Instant) {
        self.set(!self.checked, now);
    }

    pub fn set(&mut self, checked: bool, now: Instant) {
        self.checked = checked;
        self.anim.set_target(f32::from(u8::from(checked)), now);
    }

    pub fn is_animating(&self, now: Instant) -> bool {
        !self.anim.is_settled(now)
    }
}

/// Spring-animated Material 3 switch (`52×32` pill track, morphing thumb + check icon).
pub fn switch<M: 'static>(
    key: impl Into<NodeKey>,
    state: &SwitchState,
    now: Instant,
    theme: &Theme,
    on_toggle: impl Fn() -> M + 'static,
) -> Element<M> {
    let (track_w, track_h) = SIZE.switch; // 52x32
    let t = state.anim.value(now).clamp(0.0, 1.0);

    let off_bg = Rgba::from_color(theme.scheme.surface_container_highest);
    let on_bg = Rgba::from_color(theme.scheme.primary);
    let track_bg = off_bg.lerp(on_bg, t);
    let border_col = Rgba::from_color(theme.scheme.outline).lerp(on_bg, t);

    let thumb_d = 16.0 + 8.0 * t; // 16px off -> 24px on
    let thumb_x = 8.0 + (track_w - 4.0 - 24.0 - 8.0) * t;
    let thumb_y = (track_h - thumb_d) * 0.5;
    let thumb_col =
        Rgba::from_color(theme.scheme.outline).lerp(Rgba::from_color(theme.scheme.on_primary), t);

    let mut thumb = box_()
        .absolute()
        .inset_xy(thumb_x, thumb_y)
        .size(thumb_d, thumb_d)
        .items_center()
        .justify_center()
        .rounded(thumb_d * 0.5)
        .smoothing(0.0)
        .bg(thumb_col);

    if t > 0.55 {
        let check_alpha = ((t - 0.55) / 0.45).clamp(0.0, 1.0);
        thumb = thumb.child(icon(
            Icon::Check,
            IconStyle::Filled,
            15.0,
            Rgba::from_color(theme.scheme.primary).with_alpha(check_alpha),
        ));
    }

    box_()
        .id(key)
        .size(track_w, track_h)
        .rounded(track_h * 0.5)
        .smoothing(0.0)
        .bg(track_bg)
        .border(2.0 * (1.0 - t * 0.5), border_col)
        .on_click(on_toggle)
        .child(thumb)
}

// ── ScrollArea ───────────────────────────────────────────────────────────────

/// Clipped vertical scroll container with automatic wheel scrolling managed by
/// [`InteractionState`].
pub fn scroll_area<M>(
    key: impl Into<NodeKey>,
    height: f32,
    gap: f32,
    padding: impl Into<Edges>,
    children: impl IntoIterator<Item = Element<M>>,
) -> Element<M> {
    col()
        .id(key)
        .w_full()
        .h(height)
        .p(padding)
        .gap(gap)
        .scroll_y()
        .children(children)
}

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
