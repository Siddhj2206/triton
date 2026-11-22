//! Single-line text input: [`TextInputState`] editing model + [`text_input`] field.

use triton_design::icon::{Icon, IconStyle};
use triton_design::shape::{CORNER_SMOOTHING, SIZE};
use triton_engine::style::{FontSpec, Rgba, Theme};
use triton_engine::text::TextEngine;
use triton_engine::ui::{
    Element, InputEvent, InteractionState, Key, NodeKey, box_, icon, row, text,
};

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
