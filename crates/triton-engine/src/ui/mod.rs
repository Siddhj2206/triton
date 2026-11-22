//! Declarative UI node tree, Taffy layout solver, and automatic interaction/hit-testing.
//!
//! Shell authors build UI in 100% safe Rust using [`col`], [`row`], [`box_`],
//! [`text`], [`icon`], and widgets in [`crate::widgets`]. No manual `HitRect`
//! tables, no raw coordinates, no GL calls.

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use taffy::prelude::*;
use taffy::style_helpers::{auto, length, percent};
use taffy::{LayoutOutput, Overflow};
use triton_design::icon::{Icon, IconStyle};
use triton_design::material::Material;
use triton_design::shape::CORNER_SMOOTHING;

use crate::gpu::{DrawList, KIND_FILL, KIND_SHADOW, KIND_TOP_HIGHLIGHT, QuadInstance, ScissorRect};
use crate::style::{Corners, DropShadow, Edges, FontSpec, Rect, Rgba, SubtreeTransform, Theme};
use crate::text::TextEngine;

/// Stable widget identity used for hover, press, drag, focus, and scroll state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeKey(pub u64);

impl NodeKey {
    pub fn named(s: &str) -> Self {
        let mut h = DefaultHasher::new();
        s.hash(&mut h);
        Self(h.finish())
    }

    pub fn from_pair(prefix: &str, index: usize) -> Self {
        let mut h = DefaultHasher::new();
        prefix.hash(&mut h);
        index.hash(&mut h);
        Self(h.finish())
    }
}

impl From<&str> for NodeKey {
    fn from(s: &str) -> Self {
        Self::named(s)
    }
}

impl From<u64> for NodeKey {
    fn from(v: u64) -> Self {
        Self(v)
    }
}

/// Interactive event payload produced by pointer and keyboard interaction.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum InputEvent {
    PointerMove {
        x: f32,
        y: f32,
    },
    PointerLeave,
    PointerDown {
        x: f32,
        y: f32,
        button: u32,
    },
    PointerUp {
        x: f32,
        y: f32,
        button: u32,
    },
    Scroll {
        x: f32,
        y: f32,
        delta_y: f32,
    },
    KeyDown {
        key: Key,
        utf8: Option<String>,
        ctrl: bool,
        shift: bool,
        alt: bool,
    },
}

/// Normalized keyboard keys exposed to safe UI code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Key {
    Escape,
    Enter,
    Tab,
    BackSpace,
    Delete,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    Space,
    Char(char),
    Other,
}

/// Text alignment inside a text node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextAlign {
    #[default]
    Start,
    Center,
    End,
}

/// Persistent per-surface interaction state owned by the engine runtime.
#[derive(Debug, Default)]
pub struct InteractionState {
    pub hovered: Option<NodeKey>,
    pub pressed: Option<NodeKey>,
    pub dragging: Option<NodeKey>,
    pub focused: Option<NodeKey>,
    pub scroll_offsets: HashMap<NodeKey, f32>,
    pub pointer_pos: Option<(f32, f32)>,
}

impl InteractionState {
    pub fn is_hovered(&self, key: NodeKey) -> bool {
        self.hovered == Some(key)
    }

    pub fn is_pressed(&self, key: NodeKey) -> bool {
        self.pressed == Some(key)
    }

    pub fn is_focused(&self, key: NodeKey) -> bool {
        self.focused == Some(key)
    }

    pub fn scroll_offset(&self, key: NodeKey) -> f32 {
        self.scroll_offsets.get(&key).copied().unwrap_or(0.0)
    }

    pub fn set_scroll_offset(&mut self, key: NodeKey, offset: f32) {
        self.scroll_offsets.insert(key, offset.max(0.0));
    }
}

/// A blur region reported by a `Glass` / `Thick` / `Mica` element during paint.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlurRect {
    pub rect: Rect,
    pub radius: f32,
}

pub type ClickHandler<M> = Box<dyn Fn() -> M>;
pub type DragHandler<M> = Box<dyn Fn(f32, f32, Rect) -> M>;
pub type KeyHandler<M> = Box<dyn Fn(&InputEvent) -> Option<M>>;

/// Click / drag / scroll / key callbacks attached to a declarative [`Element`].
pub struct Handlers<M> {
    pub on_click: Option<ClickHandler<M>>,
    pub on_drag: Option<DragHandler<M>>,
    pub on_key: Option<KeyHandler<M>>,
}

impl<M> Default for Handlers<M> {
    fn default() -> Self {
        Self {
            on_click: None,
            on_drag: None,
            on_key: None,
        }
    }
}

impl<M> std::fmt::Debug for Handlers<M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Handlers")
            .field("on_click", &self.on_click.is_some())
            .field("on_drag", &self.on_drag.is_some())
            .field("on_key", &self.on_key.is_some())
            .finish()
    }
}

/// Visual paint specification of a single element.
#[derive(Debug, Clone)]
pub struct VisualStyle {
    pub fill: Option<Rgba>,
    pub hover_fill: Option<Rgba>,
    pub press_fill: Option<Rgba>,
    pub border_color: Option<Rgba>,
    pub border_width: f32,
    pub top_highlight: Option<Rgba>,
    pub shadow: Option<DropShadow>,
    pub radii: Corners,
    pub smoothing: f32,
    pub blur: bool,
    pub clip: bool,
    pub transform: SubtreeTransform,
    /// Optional caret drawn inside a `TextInput` node `(x_offset_from_pad, height, color)`.
    pub caret: Option<(f32, f32, Rgba)>,
}

impl Default for VisualStyle {
    fn default() -> Self {
        Self {
            fill: None,
            hover_fill: None,
            press_fill: None,
            border_color: None,
            border_width: 0.0,
            top_highlight: None,
            shadow: None,
            radii: Corners::ZERO,
            smoothing: CORNER_SMOOTHING,
            blur: false,
            clip: false,
            transform: SubtreeTransform::IDENTITY,
            caret: None,
        }
    }
}

#[derive(Debug, Clone)]
pub enum ElementContent {
    Container,
    Text {
        text: String,
        font: FontSpec,
        color: Rgba,
        align: TextAlign,
        wrap: bool,
    },
}

/// A declarative UI element producing layout, paint commands, blur regions, and
/// hit-test regions automatically.
#[must_use]
#[derive(Debug)]
pub struct Element<M> {
    pub key: Option<NodeKey>,
    pub layout: Style,
    pub visual: VisualStyle,
    pub content: ElementContent,
    pub children: Vec<Element<M>>,
    pub handlers: Handlers<M>,
    pub scrollable: bool,
    pub focusable: bool,
    /// Marks whether this element claims pointer input for Wayland input regions
    /// (defaults to true when it has a fill or interaction handler).
    pub captures_input: bool,
}

impl<M> Element<M> {
    fn container(dir: FlexDirection) -> Self {
        Self {
            key: None,
            layout: Style {
                display: Display::Flex,
                flex_direction: dir,
                ..Style::default()
            },
            visual: VisualStyle::default(),
            content: ElementContent::Container,
            children: Vec::new(),
            handlers: Handlers::default(),
            scrollable: false,
            focusable: false,
            captures_input: false,
        }
    }

    // ── identity & hierarchy ─────────────────────────────────────────────────

    pub fn id(mut self, key: impl Into<NodeKey>) -> Self {
        self.key = Some(key.into());
        self
    }

    pub fn child(mut self, mut child: Element<M>) -> Self {
        if self.scrollable {
            child.layout.flex_shrink = 0.0;
        }
        self.children.push(child);
        self
    }

    pub fn child_opt(mut self, child: Option<Element<M>>) -> Self {
        if let Some(c) = child {
            self.children.push(c);
        }
        self
    }

    pub fn children<I: IntoIterator<Item = Element<M>>>(mut self, iter: I) -> Self {
        for mut c in iter {
            if self.scrollable {
                c.layout.flex_shrink = 0.0;
            }
            self.children.push(c);
        }
        self
    }

    // ── layout (taffy) ───────────────────────────────────────────────────────

    pub fn w(mut self, px: f32) -> Self {
        self.layout.size.width = length(px);
        self
    }

    pub fn h(mut self, px: f32) -> Self {
        self.layout.size.height = length(px);
        self
    }

    pub fn size(self, w: f32, h: f32) -> Self {
        self.w(w).h(h)
    }

    pub fn min_w(mut self, px: f32) -> Self {
        self.layout.min_size.width = length(px);
        self
    }

    pub fn min_h(mut self, px: f32) -> Self {
        self.layout.min_size.height = length(px);
        self
    }

    pub fn max_w(mut self, px: f32) -> Self {
        self.layout.max_size.width = length(px);
        self
    }

    pub fn max_h(mut self, px: f32) -> Self {
        self.layout.max_size.height = length(px);
        self
    }

    pub fn w_full(mut self) -> Self {
        self.layout.size.width = percent(1.0);
        self
    }

    pub fn h_full(mut self) -> Self {
        self.layout.size.height = percent(1.0);
        self
    }

    pub fn flex_1(mut self) -> Self {
        self.layout.flex_grow = 1.0;
        self.layout.flex_shrink = 1.0;
        self.layout.flex_basis = length(0.0);
        self
    }

    pub fn flex_grow(mut self, grow: f32) -> Self {
        self.layout.flex_grow = grow;
        self
    }

    pub fn flex_shrink(mut self, shrink: f32) -> Self {
        self.layout.flex_shrink = shrink;
        self
    }

    pub fn wrap(mut self) -> Self {
        self.layout.flex_wrap = FlexWrap::Wrap;
        self
    }

    pub fn gap(mut self, px: f32) -> Self {
        self.layout.gap = Size {
            width: length(px),
            height: length(px),
        };
        self
    }

    pub fn gap_xy(mut self, x: f32, y: f32) -> Self {
        self.layout.gap = Size {
            width: length(x),
            height: length(y),
        };
        self
    }

    pub fn p(mut self, edges: impl Into<Edges>) -> Self {
        let e = edges.into();
        self.layout.padding = taffy::Rect {
            top: length(e.top),
            right: length(e.right),
            bottom: length(e.bottom),
            left: length(e.left),
        };
        self
    }

    pub fn px(mut self, x: f32) -> Self {
        self.layout.padding.left = length(x);
        self.layout.padding.right = length(x);
        self
    }

    pub fn py(mut self, y: f32) -> Self {
        self.layout.padding.top = length(y);
        self.layout.padding.bottom = length(y);
        self
    }

    pub fn m(mut self, edges: impl Into<Edges>) -> Self {
        let e = edges.into();
        self.layout.margin = taffy::Rect {
            top: length(e.top),
            right: length(e.right),
            bottom: length(e.bottom),
            left: length(e.left),
        };
        self
    }

    pub fn items_center(mut self) -> Self {
        self.layout.align_items = AlignItems::CENTER;
        self
    }

    pub fn items_start(mut self) -> Self {
        self.layout.align_items = AlignItems::START;
        self
    }

    pub fn items_end(mut self) -> Self {
        self.layout.align_items = AlignItems::END;
        self
    }

    pub fn items_stretch(mut self) -> Self {
        self.layout.align_items = AlignItems::STRETCH;
        self
    }

    pub fn justify_center(mut self) -> Self {
        self.layout.justify_content = JustifyContent::CENTER;
        self
    }

    pub fn justify_between(mut self) -> Self {
        self.layout.justify_content = JustifyContent::SPACE_BETWEEN;
        self
    }

    pub fn justify_end(mut self) -> Self {
        self.layout.justify_content = JustifyContent::END;
        self
    }

    pub fn align_self_center(mut self) -> Self {
        self.layout.align_self = Some(AlignSelf::CENTER);
        self
    }

    pub fn absolute(mut self) -> Self {
        self.layout.position = Position::Absolute;
        self
    }

    pub fn inset_xy(mut self, left: f32, top: f32) -> Self {
        self.layout.inset = taffy::Rect {
            left: length(left),
            top: length(top),
            right: auto(),
            bottom: auto(),
        };
        self
    }

    pub fn inset_all(mut self, v: f32) -> Self {
        self.layout.inset = taffy::Rect {
            left: length(v),
            top: length(v),
            right: length(v),
            bottom: length(v),
        };
        self
    }

    // ── visual styling ───────────────────────────────────────────────────────

    pub fn bg(mut self, color: impl Into<Rgba>) -> Self {
        self.visual.fill = Some(color.into());
        self.captures_input = true;
        self
    }

    pub fn hover_bg(mut self, color: impl Into<Rgba>) -> Self {
        self.visual.hover_fill = Some(color.into());
        self
    }

    pub fn press_bg(mut self, color: impl Into<Rgba>) -> Self {
        self.visual.press_fill = Some(color.into());
        self
    }

    pub fn border(mut self, width: f32, color: impl Into<Rgba>) -> Self {
        self.visual.border_width = width;
        self.visual.border_color = Some(color.into());
        self
    }

    pub fn top_highlight(mut self, color: impl Into<Rgba>) -> Self {
        self.visual.top_highlight = Some(color.into());
        self
    }

    pub fn shadow(mut self, shadow: impl Into<DropShadow>) -> Self {
        self.visual.shadow = Some(shadow.into());
        self
    }

    pub fn rounded(mut self, corners: impl Into<Corners>) -> Self {
        self.visual.radii = corners.into();
        self
    }

    pub fn smoothing(mut self, smoothing: f32) -> Self {
        self.visual.smoothing = smoothing;
        self
    }

    /// Apply a Triton design material (`Solid`, `Mica`, `Glass`, `Thick`): sets
    /// fill tint, top highlight, subtle hairline border, and compositor blur flag.
    pub fn material(mut self, material: Material, theme: &Theme) -> Self {
        self.visual.fill = Some(theme.material_fill(material));
        self.visual.top_highlight = theme.material_highlight(material);
        if let Some(border) = theme.material_border(material) {
            self.visual.border_width = 1.0;
            self.visual.border_color = Some(border);
        }
        self.visual.blur = material.spec(theme.variant).is_some();
        self.captures_input = true;
        self
    }

    pub fn clip(mut self) -> Self {
        self.visual.clip = true;
        self
    }

    pub fn scroll_y(mut self) -> Self {
        self.visual.clip = true;
        self.scrollable = true;
        self.layout.overflow.y = Overflow::Scroll;
        // Direct children inside a vertical scroll container should keep their
        // intrinsic/specified height instead of shrinking to fit the viewport.
        for child in &mut self.children {
            child.layout.flex_shrink = 0.0;
        }
        self
    }

    pub fn transform(mut self, tf: SubtreeTransform) -> Self {
        self.visual.transform = tf;
        self
    }

    pub fn opacity(mut self, opacity: f32) -> Self {
        self.visual.transform.opacity = opacity;
        self
    }

    pub fn caret(mut self, x_offset: f32, height: f32, color: impl Into<Rgba>) -> Self {
        self.visual.caret = Some((x_offset, height, color.into()));
        self
    }

    // ── interaction ──────────────────────────────────────────────────────────

    pub fn on_click(mut self, f: impl Fn() -> M + 'static) -> Self {
        self.handlers.on_click = Some(Box::new(f));
        self.captures_input = true;
        self
    }

    pub fn on_drag(mut self, f: impl Fn(f32, f32, Rect) -> M + 'static) -> Self {
        self.handlers.on_drag = Some(Box::new(f));
        self.captures_input = true;
        self
    }

    pub fn on_key(mut self, f: impl Fn(&InputEvent) -> Option<M> + 'static) -> Self {
        self.handlers.on_key = Some(Box::new(f));
        self.focusable = true;
        self.captures_input = true;
        self
    }

    pub fn focusable(mut self) -> Self {
        self.focusable = true;
        self.captures_input = true;
        self
    }
}

// ── ergonomic constructors ───────────────────────────────────────────────────

/// Vertical flex container (`FlexDirection::Column`).
pub fn col<M>() -> Element<M> {
    Element::container(FlexDirection::Column)
}

/// Horizontal flex container (`FlexDirection::Row`).
pub fn row<M>() -> Element<M> {
    Element::container(FlexDirection::Row)
}

/// Generic flex container (defaults to centered row, ideal for shapes/badges/spacers).
pub fn box_<M>() -> Element<M> {
    Element::container(FlexDirection::Row)
}

/// Text leaf backed by `cosmic-text` shaping & measurement.
pub fn text<M>(
    content: impl Into<String>,
    font: impl Into<FontSpec>,
    color: impl Into<Rgba>,
) -> Element<M> {
    Element {
        key: None,
        layout: Style {
            flex_shrink: 1.0,
            ..Style::default()
        },
        visual: VisualStyle::default(),
        content: ElementContent::Text {
            text: content.into(),
            font: font.into(),
            color: color.into(),
            align: TextAlign::Start,
            wrap: false,
        },
        children: Vec::new(),
        handlers: Handlers::default(),
        scrollable: false,
        focusable: false,
        captures_input: false,
    }
}

/// Material Symbols icon leaf using Triton's embedded symbol fonts.
pub fn icon<M>(glyph: Icon, style: IconStyle, size: f32, color: impl Into<Rgba>) -> Element<M> {
    let font = FontSpec {
        family: style.family(),
        size,
        line_height: size,
        weight: 500,
        letter_spacing: 0.0,
        uppercase: false,
    };
    text(glyph.codepoint().to_string(), font, color)
        .w(size)
        .h(size)
        .flex_shrink(0.0)
}

impl<M> Element<M> {
    pub fn text_align(mut self, align: TextAlign) -> Self {
        if let ElementContent::Text { align: a, .. } = &mut self.content {
            *a = align;
        }
        self
    }

    pub fn text_wrap(mut self) -> Self {
        if let ElementContent::Text { wrap, .. } = &mut self.content {
            *wrap = true;
        }
        self
    }
}

// ── laid-out tree & hit testing ──────────────────────────────────────────────

/// One interactive hit target extracted automatically during layout/paint.
pub struct HitEntry<M> {
    pub key: Option<NodeKey>,
    pub rect: Rect,
    pub clip: Option<Rect>,
    pub scrollable: bool,
    pub content_height: f32,
    pub focusable: bool,
    pub on_click: Option<ClickHandler<M>>,
    pub on_drag: Option<DragHandler<M>>,
    pub on_key: Option<KeyHandler<M>>,
}

impl<M> std::fmt::Debug for HitEntry<M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HitEntry")
            .field("key", &self.key)
            .field("rect", &self.rect)
            .field("scrollable", &self.scrollable)
            .finish_non_exhaustive()
    }
}

/// Result of laying out and painting a surface's declarative element tree.
#[derive(Debug)]
pub struct BuiltFrame<M> {
    /// Computed root size in logical px (used for dynamic content-driven surface sizing).
    pub content_size: (f32, f32),
    /// Blur regions requested by `Glass`/`Thick`/`Mica` elements (logical surface px).
    pub blur_rects: Vec<BlurRect>,
    /// Input-capturing regions in logical surface px (for `wl_surface.set_input_region`).
    pub input_rects: Vec<BlurRect>,
    /// Ordered hit targets (back-to-front; hit-testing iterates in reverse).
    pub hits: Vec<HitEntry<M>>,
}

impl<M> Default for BuiltFrame<M> {
    fn default() -> Self {
        Self {
            content_size: (0.0, 0.0),
            blur_rects: Vec::new(),
            input_rects: Vec::new(),
            hits: Vec::new(),
        }
    }
}

impl<M> BuiltFrame<M> {
    /// Dispatch an [`InputEvent`] against the laid-out hit tree, updating
    /// `interaction` and returning any emitted app messages plus whether a redraw
    /// is required.
    #[allow(clippy::too_many_lines)]
    pub fn dispatch_event(
        &self,
        event: &InputEvent,
        interaction: &mut InteractionState,
    ) -> (Vec<M>, bool) {
        let mut msgs = Vec::new();
        let mut dirty = false;

        match *event {
            InputEvent::PointerMove { x, y } => {
                interaction.pointer_pos = Some((x, y));
                if let Some(drag_key) = interaction.dragging {
                    if let Some(entry) = self
                        .hits
                        .iter()
                        .rev()
                        .find(|e| e.key == Some(drag_key) && e.on_drag.is_some())
                        && let Some(on_drag) = &entry.on_drag
                    {
                        msgs.push(on_drag(x, y, entry.rect));
                        dirty = true;
                    }
                    return (msgs, dirty);
                }
                let new_hover = self.top_interactive_at(x, y).and_then(|e| e.key);
                if new_hover != interaction.hovered {
                    interaction.hovered = new_hover;
                    dirty = true;
                }
            }
            InputEvent::PointerLeave => {
                interaction.pointer_pos = None;
                if interaction.hovered.is_some() {
                    interaction.hovered = None;
                    dirty = true;
                }
            }
            InputEvent::PointerDown { x, y, .. } => {
                interaction.pointer_pos = Some((x, y));
                if let Some(entry) = self.top_interactive_at(x, y) {
                    if entry.focusable {
                        interaction.focused = entry.key;
                        dirty = true;
                    }
                    if entry.on_drag.is_some() {
                        interaction.dragging = entry.key;
                        interaction.pressed = entry.key;
                        if let Some(on_drag) = &entry.on_drag {
                            msgs.push(on_drag(x, y, entry.rect));
                        }
                        dirty = true;
                    } else if entry.on_click.is_some() {
                        interaction.pressed = entry.key;
                        dirty = true;
                    }
                } else if interaction.focused.is_some() {
                    interaction.focused = None;
                    dirty = true;
                }
            }
            InputEvent::PointerUp { x, y, .. } => {
                let was_dragging = interaction.dragging.take();
                let was_pressed = interaction.pressed.take();
                if was_dragging.is_some() || was_pressed.is_some() {
                    dirty = true;
                }
                if was_dragging.is_none()
                    && let Some(entry) = self.top_interactive_at(x, y)
                    && (was_pressed.is_none() || entry.key == was_pressed)
                    && let Some(on_click) = &entry.on_click
                {
                    msgs.push(on_click());
                }
            }
            InputEvent::Scroll { x, y, delta_y } => {
                if let Some(entry) = self.hits.iter().rev().find(|e| {
                    e.scrollable && e.rect.contains(x, y) && e.clip.is_none_or(|c| c.contains(x, y))
                }) && let Some(key) = entry.key
                {
                    let max_scroll = (entry.content_height - entry.rect.h).max(0.0);
                    let cur = interaction.scroll_offset(key);
                    let next = (cur + delta_y).clamp(0.0, max_scroll);
                    if (next - cur).abs() > 0.1 {
                        interaction.set_scroll_offset(key, next);
                        dirty = true;
                    }
                }
            }
            InputEvent::KeyDown { .. } => {
                // Deliver to focused node first, then fall back to any key handler.
                if let Some(focused) = interaction.focused
                    && let Some(entry) = self
                        .hits
                        .iter()
                        .rev()
                        .find(|e| e.key == Some(focused) && e.on_key.is_some())
                    && let Some(on_key) = &entry.on_key
                    && let Some(m) = on_key(event)
                {
                    msgs.push(m);
                    dirty = true;
                    return (msgs, dirty);
                }
                for entry in self.hits.iter().rev() {
                    if let Some(on_key) = &entry.on_key
                        && let Some(m) = on_key(event)
                    {
                        msgs.push(m);
                        dirty = true;
                        break;
                    }
                }
            }
        }

        (msgs, dirty)
    }

    fn top_interactive_at(&self, x: f32, y: f32) -> Option<&HitEntry<M>> {
        self.hits.iter().rev().find(|e| {
            (e.on_click.is_some() || e.on_drag.is_some() || e.focusable)
                && e.rect.contains(x, y)
                && e.clip.is_none_or(|c| c.contains(x, y))
        })
    }
}

/// Measure the natural logical size of `root` using Taffy + `TextEngine`,
/// without emitting GPU commands. Used by dynamic content-sized surfaces before
/// configuring/resizing their Wayland layer surface.
pub fn measure_tree<M>(
    root: &Element<M>,
    max_space: (Option<f32>, Option<f32>),
    text: &mut TextEngine,
) -> (f32, f32) {
    let mut tree: TaffyTree<TextMeasureContext> = TaffyTree::new();
    let root_id = build_taffy_nodes(&mut tree, root);
    let avail = Size {
        width: max_space
            .0
            .map_or(AvailableSpace::MaxContent, AvailableSpace::Definite),
        height: max_space
            .1
            .map_or(AvailableSpace::MaxContent, AvailableSpace::Definite),
    };
    let _ = tree.compute_layout_with_measure(root_id, avail, |inputs, _node_id, ctx, style| {
        measure_leaf(inputs, ctx, style, text)
    });
    let layout = tree.layout(root_id).expect("root layout");
    (layout.size.width, layout.size.height)
}

/// Layout and paint `root` into `draw_list`, returning a [`BuiltFrame`] with
/// blur regions, input regions, and interactive hit targets.
#[allow(clippy::too_many_arguments)]
pub fn layout_and_paint<M>(
    root: Element<M>,
    available: (Option<f32>, Option<f32>),
    scale: f32,
    interaction: &InteractionState,
    text: &mut TextEngine,
    draw_list: &mut DrawList,
) -> BuiltFrame<M> {
    draw_list.clear();
    let mut tree: TaffyTree<TextMeasureContext> = TaffyTree::new();
    let root_id = build_taffy_nodes(&mut tree, &root);
    let avail = Size {
        width: available
            .0
            .map_or(AvailableSpace::MaxContent, AvailableSpace::Definite),
        height: available
            .1
            .map_or(AvailableSpace::MaxContent, AvailableSpace::Definite),
    };
    let _ = tree.compute_layout_with_measure(root_id, avail, |inputs, _node_id, ctx, style| {
        measure_leaf(inputs, ctx, style, text)
    });
    let root_layout = tree.layout(root_id).expect("root layout");
    let mut frame = BuiltFrame {
        content_size: (root_layout.size.width, root_layout.size.height),
        ..BuiltFrame::default()
    };

    let mut scratch_glyphs = Vec::new();
    paint_node(
        &tree,
        root_id,
        root,
        (0.0, 0.0),
        SubtreeTransform::IDENTITY,
        None,
        scale,
        interaction,
        text,
        draw_list,
        &mut scratch_glyphs,
        &mut frame,
    );
    frame
}

struct TextMeasureContext {
    text: String,
    font: FontSpec,
    wrap: bool,
}

fn build_taffy_nodes<M>(tree: &mut TaffyTree<TextMeasureContext>, elem: &Element<M>) -> NodeId {
    match &elem.content {
        ElementContent::Text {
            text, font, wrap, ..
        } => tree
            .new_leaf_with_context(
                elem.layout.clone(),
                TextMeasureContext {
                    text: text.clone(),
                    font: font.clone(),
                    wrap: *wrap,
                },
            )
            .expect("taffy text leaf"),
        ElementContent::Container => {
            let child_ids: Vec<NodeId> = elem
                .children
                .iter()
                .map(|c| build_taffy_nodes(tree, c))
                .collect();
            tree.new_with_children(elem.layout.clone(), &child_ids)
                .expect("taffy container")
        }
    }
}

fn measure_leaf(
    inputs: taffy::LayoutInput,
    ctx: Option<&mut TextMeasureContext>,
    style: &Style,
    text: &mut TextEngine,
) -> LayoutOutput {
    let Some(ctx) = ctx else {
        return taffy::compute_leaf_layout(inputs, style, |_, _| 0.0, |_, _| Size::ZERO);
    };
    taffy::compute_leaf_layout(
        inputs,
        style,
        |_, _| 0.0,
        |known, avail| {
            let max_w = if ctx.wrap {
                known.width.or(match avail.width {
                    AvailableSpace::Definite(w) => Some(w),
                    AvailableSpace::MinContent => Some(0.0),
                    AvailableSpace::MaxContent => None,
                })
            } else {
                known.width
            };
            let m = text.measure(&ctx.text, &ctx.font, max_w);
            Size {
                width: known.width.unwrap_or(m.width.ceil()),
                height: known.height.unwrap_or(m.height.ceil()),
            }
        },
    )
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn paint_node<M>(
    tree: &TaffyTree<TextMeasureContext>,
    node_id: NodeId,
    elem: Element<M>,
    origin: (f32, f32),
    parent_tf: SubtreeTransform,
    clip_rect: Option<Rect>,
    scale: f32,
    interaction: &InteractionState,
    text: &mut TextEngine,
    draw_list: &mut DrawList,
    scratch_glyphs: &mut Vec<crate::gpu::GlyphInstance>,
    frame: &mut BuiltFrame<M>,
) {
    let layout = *tree.layout(node_id).expect("node layout");
    let local_rect = Rect::new(
        origin.0 + layout.location.x,
        origin.1 + layout.location.y,
        layout.size.width,
        layout.size.height,
    );

    let cx = local_rect.x + local_rect.w * 0.5;
    let cy = local_rect.y + local_rect.h * 0.5;
    let tf = if elem.visual.transform.is_identity() {
        parent_tf
    } else {
        parent_tf.compose_about(elem.visual.transform, cx, cy)
    };
    if tf.opacity <= 0.001 {
        return;
    }

    let screen_rect = tf.map_rect(local_rect);
    let scissor = clip_rect.map(|c| rect_to_scissor(c, scale));
    let eff_scale = scale * tf.scale;

    // 1. Optional SDF drop shadow (painted behind the element itself).
    //    For translucent fills (Glass/Thick/alpha < 0.99), request an interior
    //    caster cutout so the dark shadow never bleeds through and muddies the glass.
    if let Some(sh) = elem.visual.shadow {
        let pad = sh.blur * 2.0;
        let sx = screen_rect.x + sh.offset_x * tf.scale - sh.spread * tf.scale - pad * tf.scale;
        let sy = screen_rect.y + sh.offset_y * tf.scale - sh.spread * tf.scale - pad * tf.scale;
        let sw = screen_rect.w + (sh.spread + pad) * 2.0 * tf.scale;
        let sh_h = screen_rect.h + (sh.spread + pad) * 2.0 * tf.scale;
        let radii = elem
            .visual
            .radii
            .clamped(local_rect.w, local_rect.h)
            .map(|r| (r + sh.spread).max(0.0) * eff_scale);
        let cutout = elem.visual.fill.is_some_and(|f| f.a > 0.0 && f.a < 0.99);
        draw_list.push_quad(
            scissor,
            QuadInstance {
                dst: [sx * scale, sy * scale, sw * scale, sh_h * scale],
                radii,
                fill: sh.color.to_array(),
                border: [
                    sh.offset_x * eff_scale,
                    sh.offset_y * eff_scale,
                    sh.spread * eff_scale,
                    if cutout { 1.0 } else { 0.0 },
                ],
                misc: [
                    KIND_SHADOW,
                    elem.visual.smoothing,
                    sh.blur * eff_scale,
                    tf.opacity,
                ],
            },
        );
    }

    // 2. Background fill / hover-press state / border / top highlight
    let hovered = elem.key.is_some_and(|k| interaction.is_hovered(k));
    let pressed = elem.key.is_some_and(|k| interaction.is_pressed(k));
    let fill = if pressed && elem.visual.press_fill.is_some() {
        elem.visual.press_fill
    } else if hovered && elem.visual.hover_fill.is_some() {
        elem.visual.hover_fill
    } else {
        elem.visual.fill
    };

    let has_shape =
        fill.is_some() || elem.visual.border_width > 0.0 || elem.visual.top_highlight.is_some();

    let clamped_radii = elem.visual.radii.clamped(local_rect.w, local_rect.h);
    if has_shape && screen_rect.w > 0.0 && screen_rect.h > 0.0 {
        let fill_arr = fill.unwrap_or(Rgba::TRANSPARENT).to_array();
        let (kind, border_arr, border_w) = if let Some(hl) = elem.visual.top_highlight {
            (
                KIND_TOP_HIGHLIGHT,
                hl.to_array(),
                elem.visual.border_width * eff_scale,
            )
        } else {
            (
                KIND_FILL,
                elem.visual
                    .border_color
                    .unwrap_or(Rgba::TRANSPARENT)
                    .to_array(),
                elem.visual.border_width * eff_scale,
            )
        };
        draw_list.push_quad(
            scissor,
            QuadInstance {
                dst: [
                    screen_rect.x * scale,
                    screen_rect.y * scale,
                    screen_rect.w * scale,
                    screen_rect.h * scale,
                ],
                radii: clamped_radii.map(|r| r * eff_scale),
                fill: fill_arr,
                border: border_arr,
                misc: [kind, elem.visual.smoothing, border_w, tf.opacity],
            },
        );
    }

    // Report compositor blur & Wayland input regions in logical surface coords.
    let blur_r = elem.visual.radii.max_clamped(local_rect.w, local_rect.h) * tf.scale;
    if elem.visual.blur && !screen_rect.is_empty() {
        frame.blur_rects.push(BlurRect {
            rect: screen_rect,
            radius: blur_r,
        });
    }
    if elem.captures_input && !screen_rect.is_empty() {
        frame.input_rects.push(BlurRect {
            rect: screen_rect,
            radius: blur_r,
        });
    }

    // 3. Text content
    if let ElementContent::Text {
        text: content,
        font,
        color,
        align,
        wrap,
    } = &elem.content
        && !content.is_empty()
    {
        let max_w = if *wrap { Some(local_rect.w) } else { None };
        let measured = text.measure(content, font, max_w);
        let dx = match align {
            TextAlign::Start => 0.0,
            TextAlign::Center => ((local_rect.w - measured.width) * 0.5).max(0.0),
            TextAlign::End => (local_rect.w - measured.width).max(0.0),
        };
        let dy = ((local_rect.h - measured.height) * 0.5).max(0.0);
        scratch_glyphs.clear();
        text.layout_glyphs(
            content,
            font,
            max_w,
            screen_rect.x + dx * tf.scale,
            screen_rect.y + dy * tf.scale,
            eff_scale,
            *color,
            tf.opacity,
            scratch_glyphs,
        );
        draw_list.push_glyphs(scissor, scratch_glyphs);
    }

    // 4. Optional text caret (rendered inside this node's clip)
    if let Some((cx_off, ch, ccol)) = elem.visual.caret {
        let caret_x = screen_rect.x + cx_off * tf.scale;
        let caret_y = screen_rect.y + (screen_rect.h - ch * tf.scale) * 0.5;
        draw_list.push_quad(
            scissor,
            QuadInstance {
                dst: [
                    caret_x * scale,
                    caret_y * scale,
                    (2.0 * eff_scale).max(1.0),
                    ch * eff_scale,
                ],
                radii: [eff_scale; 4],
                fill: ccol.to_array(),
                border: [0.0; 4],
                misc: [KIND_FILL, 0.0, 0.0, tf.opacity],
            },
        );
    }

    // 5. Scroll & clipping setup for children
    let child_clip = if elem.visual.clip {
        Some(match clip_rect {
            Some(prev) => prev.intersect(screen_rect),
            None => screen_rect,
        })
    } else {
        clip_rect
    };

    let scroll_y = if elem.scrollable {
        elem.key.map_or(0.0, |k| interaction.scroll_offset(k))
    } else {
        0.0
    };

    // Compute total content height for scroll clamping.
    let child_ids: Vec<NodeId> = tree.children(node_id).unwrap_or_default();
    let mut content_height = local_rect.h;
    if elem.scrollable {
        let mut max_bottom = 0.0f32;
        for &cid in &child_ids {
            if let Ok(cl) = tree.layout(cid) {
                max_bottom = max_bottom.max(cl.location.y + cl.size.height);
            }
        }
        content_height = max_bottom + layout.padding.bottom;
    }

    // Record hit entry BEFORE painting children so children sit later in the
    // vector (higher z-order when reverse-iterating).
    if elem.handlers.on_click.is_some()
        || elem.handlers.on_drag.is_some()
        || elem.handlers.on_key.is_some()
        || elem.scrollable
        || elem.focusable
    {
        frame.hits.push(HitEntry {
            key: elem.key,
            rect: screen_rect,
            clip: clip_rect,
            scrollable: elem.scrollable,
            content_height,
            focusable: elem.focusable,
            on_click: elem.handlers.on_click,
            on_drag: elem.handlers.on_drag,
            on_key: elem.handlers.on_key,
        });
    }

    // 6. Recurse into children
    let child_origin = (local_rect.x, local_rect.y - scroll_y);
    for (cid, child_elem) in child_ids.into_iter().zip(elem.children) {
        paint_node(
            tree,
            cid,
            child_elem,
            child_origin,
            tf,
            child_clip,
            scale,
            interaction,
            text,
            draw_list,
            scratch_glyphs,
            frame,
        );
    }

    // 7. Overlay subtle scrollbar thumb when scrollable content overflows viewport
    if elem.scrollable && content_height > local_rect.h + 1.0 {
        let track_inset = 6.0;
        let track_h = (local_rect.h - track_inset * 2.0).max(8.0);
        let thumb_h = ((local_rect.h / content_height) * track_h).clamp(24.0, track_h);
        let max_scroll = (content_height - local_rect.h).max(1.0);
        let progress = (scroll_y / max_scroll).clamp(0.0, 1.0);
        let thumb_y = screen_rect.y + (track_inset + progress * (track_h - thumb_h)) * tf.scale;
        let thumb_w = 3.5;
        let thumb_x = screen_rect.x + screen_rect.w - (thumb_w + 2.0) * tf.scale;
        let scroll_scissor = child_clip.map(|c| rect_to_scissor(c, scale));
        draw_list.push_quad(
            scroll_scissor,
            QuadInstance {
                dst: [
                    thumb_x * scale,
                    thumb_y * scale,
                    thumb_w * eff_scale,
                    thumb_h * eff_scale,
                ],
                radii: [thumb_w * 0.5 * eff_scale; 4],
                fill: [1.0, 1.0, 1.0, 0.24],
                border: [0.0; 4],
                misc: [KIND_FILL, 0.0, 0.0, tf.opacity],
            },
        );
    }
}

fn rect_to_scissor(r: Rect, scale: f32) -> ScissorRect {
    let x = (r.x * scale).floor() as i32;
    let y = (r.y * scale).floor() as i32;
    let x1 = ((r.x + r.w) * scale).ceil() as i32;
    let y1 = ((r.y + r.h) * scale).ceil() as i32;
    ScissorRect {
        x: x.max(0),
        y: y.max(0),
        w: (x1 - x.max(0)).max(0),
        h: (y1 - y.max(0)).max(0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Msg {
        Clicked(&'static str),
        Dragged(i32),
    }

    #[test]
    fn automatic_hit_testing_and_scroll_clamping() {
        let mut text = TextEngine::new();
        let mut draw_list = DrawList::default();
        let mut interaction = InteractionState::default();

        let root: Element<Msg> = col()
            .id("root")
            .w(200.0)
            .h(120.0)
            .p(10.0)
            .gap(8.0)
            .child(
                box_()
                    .id("btn-a")
                    .w(180.0)
                    .h(40.0)
                    .bg(Rgba::WHITE)
                    .on_click(|| Msg::Clicked("a")),
            )
            .child(
                col()
                    .id("scroller")
                    .w(180.0)
                    .h(50.0)
                    .scroll_y()
                    .children((0..5).map(|i| {
                        box_()
                            .id(NodeKey::from_pair("row", i))
                            .w(180.0)
                            .h(30.0)
                            .bg(Rgba::BLACK)
                    })),
            );

        let frame = layout_and_paint(
            root,
            (Some(200.0), Some(120.0)),
            1.75,
            &interaction,
            &mut text,
            &mut draw_list,
        );

        // Hover + click btn-a at (50, 25)
        let (m0, dirty0) = frame.dispatch_event(
            &InputEvent::PointerMove { x: 50.0, y: 25.0 },
            &mut interaction,
        );
        assert!(m0.is_empty() && dirty0);
        assert_eq!(interaction.hovered, Some(NodeKey::named("btn-a")));

        let (_, _) = frame.dispatch_event(
            &InputEvent::PointerDown {
                x: 50.0,
                y: 25.0,
                button: 272,
            },
            &mut interaction,
        );
        let (m1, _) = frame.dispatch_event(
            &InputEvent::PointerUp {
                x: 50.0,
                y: 25.0,
                button: 272,
            },
            &mut interaction,
        );
        assert_eq!(m1, vec![Msg::Clicked("a")]);

        // Scroll inside scroller (starts around y=58..108)
        let (_, dirty_scroll) = frame.dispatch_event(
            &InputEvent::Scroll {
                x: 50.0,
                y: 80.0,
                delta_y: 40.0,
            },
            &mut interaction,
        );
        assert!(dirty_scroll);
        assert_eq!(interaction.scroll_offset(NodeKey::named("scroller")), 40.0);

        // Overscroll clamps to content_height - viewport_height (150 - 50 = 100)
        let (_, _) = frame.dispatch_event(
            &InputEvent::Scroll {
                x: 50.0,
                y: 80.0,
                delta_y: 500.0,
            },
            &mut interaction,
        );
        assert_eq!(interaction.scroll_offset(NodeKey::named("scroller")), 100.0);
    }

    #[test]
    fn drag_captures_pointer_outside_bounds() {
        let mut text = TextEngine::new();
        let mut draw_list = DrawList::default();
        let mut interaction = InteractionState::default();

        let root: Element<Msg> = box_()
            .id("slider")
            .w(100.0)
            .h(30.0)
            .bg(Rgba::WHITE)
            .on_drag(|x, _y, rect| {
                let pct = ((x - rect.x) / rect.w).clamp(0.0, 1.0);
                Msg::Dragged((pct * 100.0).round() as i32)
            });

        let frame = layout_and_paint(
            root,
            (Some(100.0), Some(30.0)),
            1.0,
            &interaction,
            &mut text,
            &mut draw_list,
        );

        let (m_down, _) = frame.dispatch_event(
            &InputEvent::PointerDown {
                x: 25.0,
                y: 15.0,
                button: 272,
            },
            &mut interaction,
        );
        assert_eq!(m_down, vec![Msg::Dragged(25)]);

        let (m_move, _) = frame.dispatch_event(
            &InputEvent::PointerMove { x: 80.0, y: 90.0 },
            &mut interaction,
        );
        assert_eq!(m_move, vec![Msg::Dragged(80)]);
    }
}
