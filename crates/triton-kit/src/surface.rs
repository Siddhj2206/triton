//! Surface and layout primitives: material panels, islands, cards, scrims,
//! dialogs, dividers, empty states, grouped lists, and scroll containers.
//!
//! Numbers come from `triton-design` tokens and the board reference
//! (`references/Triton Shell-project/triton/components.css`, `.t-*` classes).

use triton_design::color::{STATE_LAYER, Variant};
use triton_design::icon::{Icon, IconStyle};
use triton_design::material::{Material, ShadowToken};
use triton_design::motion::Spring;
use triton_design::shape::{CORNER_SMOOTHING, RADII, SIZE, list_corners, nested};
use triton_design::typography::TypeStyle;
use triton_engine::anim::Animator;
use triton_engine::style::{Corners, DropShadow, Edges, FontSpec, Rgba, Theme};
use triton_engine::ui::{
    ClickHandler, Element, InteractionState, NodeKey, ScrollbarStyle, box_, col, icon, row, text,
};

// ── Panels & islands ─────────────────────────────────────────────────────────

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

/// Wrap a floating element in transparent padding sized from its drop shadow so
/// the SDF shadow reaches zero alpha before the layer-surface edge clips it.
pub fn shadow_gutter<M>(element: Element<M>) -> Element<M> {
    let gutter = element
        .visual
        .shadow
        .map_or(Edges::ZERO, DropShadow::gutter);
    box_().p(gutter).child(element)
}

// ── Cards ────────────────────────────────────────────────────────────────────

/// `.t-card` reading surface: `surface_container_low`, r24, pad 20×22.
///
/// Rows inside a card step up one tone — build them with [`ListFill::Raised`]
/// or `surface_container_high` fills (`.t-card .t-li`).
pub fn card<M>(theme: &Theme) -> Element<M> {
    col()
        .bg(Rgba::from_color(theme.scheme.surface_container_low))
        .rounded(24.0)
        .smoothing(CORNER_SMOOTHING)
        .p(Edges::xy(22.0, 20.0))
}

// ── Scrim & dialog ───────────────────────────────────────────────────────────

/// Full-surface modal dim at 45% black (Containers board: "scrim 45%").
/// Fills its parent and captures pointer input so nothing below is reachable;
/// chain `.on_click(..)` for a tap-to-dismiss affordance.
pub fn scrim<M>() -> Element<M> {
    box_().w_full().h_full().bg(Rgba::new(0.0, 0.0, 0.0, 0.45))
}

/// Content and behavior of a [`dialog`] layer.
///
/// Destructive actions always use `error_container` styling — build them with
/// [`crate::button`] and `ButtonTone::Destructive`, never `Filled`
/// (Containers board: "Move to Trash" pattern).
pub struct Dialog<M> {
    /// Hit-test key for the card; keeps a press that starts on the card from
    /// releasing onto the dismiss scrim.
    pub key: NodeKey,
    /// Optional leading icon on a `tertiary_container` wave disc.
    pub icon: Option<Icon>,
    pub title: Option<String>,
    pub body: Option<String>,
    /// Right-aligned action row (typically [`crate::button`] elements).
    pub actions: Vec<Element<M>>,
    /// Emitted when the scrim behind the card is clicked.
    pub on_dismiss: Option<ClickHandler<M>>,
}

impl<M> Default for Dialog<M> {
    fn default() -> Self {
        Self {
            key: NodeKey::named("triton-dialog"),
            icon: None,
            title: None,
            body: None,
            actions: Vec::new(),
            on_dismiss: None,
        }
    }
}

impl<M> std::fmt::Debug for Dialog<M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Dialog")
            .field("key", &self.key)
            .field("icon", &self.icon)
            .field("title", &self.title)
            .field("body", &self.body)
            .field("actions", &self.actions.len())
            .field("on_dismiss", &self.on_dismiss.is_some())
            .finish()
    }
}

/// Modal dialog layer: the [`scrim`] fills the surface and a centered r28
/// Thick Glass card (`glass 88%, blur 40`) floats on top.
///
/// The card is hit-opaque: clicks on it never reach the dismiss scrim, and
/// wheel events cannot scroll content painted beneath.
pub fn dialog<M: 'static>(theme: &Theme, spec: Dialog<M>) -> Element<M> {
    let mut scrim_el = scrim().absolute().inset_all(0.0);
    if let Some(dismiss) = spec.on_dismiss {
        scrim_el = scrim_el.on_click(dismiss);
    }

    let mut card = panel(Material::Thick, RADII.xl, theme)
        .id(spec.key)
        .p(Edges::new(28.0, 28.0, 22.0, 28.0))
        .gap(14.0)
        .input_opaque();

    if spec.icon.is_some() || spec.title.is_some() {
        let mut header = row().gap(16.0).items_center();
        if let Some(glyph) = spec.icon {
            header = header.child(
                box_()
                    .size(56.0, 56.0)
                    .items_center()
                    .justify_center()
                    .wave()
                    .bg(Rgba::from_color(theme.scheme.tertiary_container))
                    .child(icon(
                        glyph,
                        IconStyle::Filled,
                        24.0,
                        Rgba::from_color(theme.scheme.on_tertiary_container),
                    )),
            );
        }
        if let Some(title) = spec.title {
            header = header.child(text(
                title,
                TypeStyle::HEADLINE_M,
                Rgba::from_color(theme.scheme.on_surface),
            ));
        }
        card = card.child(header);
    }

    if let Some(body) = spec.body {
        card = card.child(
            text(
                body,
                TypeStyle::BODY_M,
                Rgba::from_color(theme.scheme.on_surface_variant),
            )
            .text_wrap(),
        );
    }

    if !spec.actions.is_empty() {
        card = card.child(
            row()
                .w_full()
                .justify_end()
                .items_center()
                .gap(8.0)
                .children(spec.actions),
        );
    }

    // Shadow gutter inside the centered box: the large dialog shadow can never
    // clip, even when the dialog surface is not full-screen.
    let centered = box_()
        .flex_1()
        .h_full()
        .items_center()
        .justify_center()
        .child(shadow_gutter(card));

    box_().w_full().h_full().child(scrim_el).child(centered)
}

// ── Dividers ─────────────────────────────────────────────────────────────────

/// Hairline axis for a [`divider`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Orientation {
    /// `w_full` × `1px` horizontal rule.
    Horizontal,
    /// `1px` × `h_full` vertical rule.
    Vertical,
}

/// Hairline tone for a [`divider`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DividerStyle {
    /// `outline_variant` — the standard 1px divider on solid surfaces.
    Standard,
    /// Subtle hairline that stays visible on Glass/Thick fills.
    OnGlass,
}

/// 1px [`divider`](Orientation) in `outline_variant` (or the on-glass variant).
pub fn divider<M>(orientation: Orientation, style: DividerStyle, theme: &Theme) -> Element<M> {
    let color = match style {
        DividerStyle::Standard => Rgba::from_color(theme.scheme.outline_variant),
        DividerStyle::OnGlass => match theme.variant {
            Variant::Dark => Rgba::new(1.0, 1.0, 1.0, 0.12),
            Variant::Light => Rgba::new(0.0, 0.0, 0.0, 0.10),
        },
    };
    let el = match orientation {
        Orientation::Horizontal => box_().w_full().h(1.0),
        Orientation::Vertical => box_().w(1.0).h_full(),
    };
    el.bg(color)
}

// ── Empty state ──────────────────────────────────────────────────────────────

/// Centered "nothing here" placeholder: optional icon, title, and subtitle in
/// `on_surface_variant`. Fills its flex parent and centers within it.
pub fn empty_state<M>(
    glyph: Option<Icon>,
    title: &str,
    subtitle: Option<&str>,
    theme: &Theme,
) -> Element<M> {
    let onv = Rgba::from_color(theme.scheme.on_surface_variant);
    col()
        .flex_1()
        .min_h(80.0)
        .items_center()
        .justify_center()
        .gap(8.0)
        .child_opt(glyph.map(|g| icon(g, IconStyle::Outlined, 32.0, onv.with_alpha(0.8))))
        .child(text(title, TypeStyle::TITLE_M, onv))
        .child_opt(subtitle.map(|s| text(s, TypeStyle::BODY_S, onv.with_alpha(0.8))))
}

// ── Grouped lists ────────────────────────────────────────────────────────────

/// Row fill for [`list_group`] and [`list_row`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ListFill {
    /// `surface_container` — a list on a plain reading surface (`.t-li`).
    Container,
    /// `surface_container_high` — rows step up one tone inside a [`card`]
    /// (`.t-card .t-li`).
    Raised,
    /// `theme.veil()` — a list sitting on Glass, Thick, or a scrim, where a
    /// solid container would read as a hole in the material.
    Veil,
}

impl ListFill {
    fn color(self, theme: &Theme) -> Rgba {
        match self {
            Self::Container => Rgba::from_color(theme.scheme.surface_container),
            Self::Raised => Rgba::from_color(theme.scheme.surface_container_high),
            Self::Veil => theme.veil(),
        }
    }
}

/// `.t-list` grouped list: a 2px-gap column that assigns each row its
/// [`list_corners`] radii — `24px` outer ends, `6px` inner joins, `[24; 4]`
/// for a single-item group — and fills unstyled rows with `fill`.
pub fn list_group<M>(
    fill: ListFill,
    theme: &Theme,
    rows: impl IntoIterator<Item = Element<M>>,
) -> Element<M> {
    let rows: Vec<Element<M>> = rows.into_iter().collect();
    let count = rows.len();
    let fill_color = fill.color(theme);
    col()
        .gap(2.0)
        .children(rows.into_iter().enumerate().map(|(i, mut row)| {
            row.visual.radii = Corners(list_corners(i, count));
            if row.visual.fill.is_none() {
                row.visual.fill = Some(fill_color);
            }
            row
        }))
}

/// `.t-li` list row: `min-height 64`, pad `12×20`, `16px` gap, centered items.
///
/// Standalone it gets only-child corners (`[24; 4]`); inside [`list_group`] its
/// radii are reassigned by position. `selected` paints the
/// `secondary_container`/`on_secondary_container` `.on` state; `on_click` adds
/// the interactive state layer.
pub fn list_row<M: 'static>(
    key: impl Into<NodeKey>,
    fill: ListFill,
    selected: bool,
    theme: &Theme,
    on_click: Option<ClickHandler<M>>,
    children: impl IntoIterator<Item = Element<M>>,
) -> Element<M> {
    let bg = if selected {
        Rgba::from_color(theme.scheme.secondary_container)
    } else {
        fill.color(theme)
    };
    let fg = if selected {
        Rgba::from_color(theme.scheme.on_secondary_container)
    } else {
        Rgba::from_color(theme.scheme.on_surface)
    };

    let mut el = row()
        .id(key)
        .min_h(SIZE.list_row_min)
        .px(20.0)
        .py(12.0)
        .gap(16.0)
        .items_center()
        .rounded(Corners(list_corners(0, 1)))
        .smoothing(CORNER_SMOOTHING)
        .bg(bg)
        .children(children);

    if let Some(click) = on_click {
        el = el
            .hover_bg(bg.over(fg, STATE_LAYER.hover))
            .press_bg(bg.over(fg, STATE_LAYER.press))
            .on_click(click);
    }
    el
}

/// `.g` growing text block for a [`list_row`]: 16px/500 title over a 13px
/// `on_surface_variant` supporting line.
pub fn list_row_text<M>(
    title: &str,
    supporting: Option<&str>,
    selected: bool,
    theme: &Theme,
) -> Element<M> {
    let (title_col, sub_col) = if selected {
        (
            Rgba::from_color(theme.scheme.on_secondary_container),
            Rgba::from_color(theme.scheme.on_secondary_container).with_alpha(0.8),
        )
    } else {
        (
            Rgba::from_color(theme.scheme.on_surface),
            Rgba::from_color(theme.scheme.on_surface_variant),
        )
    };
    col()
        .flex_1()
        .gap(1.0)
        .child(text(title, FontSpec::ui(16.0).weight(500), title_col))
        .child_opt(supporting.map(|s| text(s, FontSpec::ui(13.0), sub_col)))
}

// ── Scroll area ──────────────────────────────────────────────────────────────

/// Layout options for [`scroll_area`].
#[derive(Debug, Clone, Copy, Default)]
pub struct ScrollAreaOptions {
    /// Vertical gap between children in logical px.
    pub gap: f32,
    /// Inner padding around the scrolled content.
    pub padding: Edges,
    /// Edge-fade widths at list ends (`Edges::xy(0.0, v)` fades top & bottom).
    pub edge_fade: Option<Edges>,
}

/// Scrollbar animation keys derived from the scroll key (XOR is a bijection, so
/// no two scroll areas can collide).
const SB_WIDTH_SALT: u64 = 0x5cb0_1111_2222_3333;
const SB_COLOR_SALT: u64 = 0x5cb0_4444_5555_6666;

/// Clipped vertical scroll container with wheel scrolling, an optional edge
/// fade, and the board scrollbar: a `4px`/`on_surface@.5` thumb at rest that
/// springs to `10px`/`@.75` while the pointer is over the viewport
/// (`InteractionState::is_scroll_hovered`) or it is being dragged.
///
/// The morph runs through `anim` (`Spring::EFFECTS`), so `reduce_motion`
/// degrades it to the standard 120ms fade automatically.
pub fn scroll_area<M>(
    key: impl Into<NodeKey>,
    height: f32,
    options: ScrollAreaOptions,
    theme: &Theme,
    interaction: &InteractionState,
    anim: &mut Animator,
    children: impl IntoIterator<Item = Element<M>>,
) -> Element<M> {
    let key = key.into();
    let active = interaction.is_scroll_hovered(key)
        || interaction.is_scrollbar_dragging(key)
        || interaction.is_dragging(key);

    let width = anim.spring_px(
        NodeKey(key.0 ^ SB_WIDTH_SALT),
        if active { 10.0 } else { 4.0 },
        Spring::EFFECTS,
    );
    let base = Rgba::from_color(theme.scheme.on_surface);
    let color = anim.color(
        NodeKey(key.0 ^ SB_COLOR_SALT),
        base.with_alpha(if active { 0.75 } else { 0.5 }),
        Spring::EFFECTS,
    );

    let mut el = col()
        .id(key)
        .w_full()
        .h(height)
        .p(options.padding)
        .gap(options.gap)
        .scroll_y()
        .scrollbar_style(ScrollbarStyle {
            width,
            margin: 4.0,
            track_inset: 8.0,
            min_thumb: 24.0,
            color,
        })
        .children(children);
    if let Some(fade) = options.edge_fade {
        el = el.edge_fade(fade);
    }
    el
}
