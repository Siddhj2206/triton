//! Buttons: spring-morphing [`button`], toggled [`icon_button`], selectable
//! [`chip`], Quick Settings [`toggle_tile`], connected [`segmented`] groups,
//! and the expanding [`power_menu`] row.
//!
//! Metrics come from `triton-design` tokens and the board reference
//! (`references/Triton Shell-project/triton/components.css`, `.t-btn`,
//! `.t-ibtn`, `.t-chip`, `.t-seg`; `Main.dc.html` `.tile`; `Power.dc.html`
//! `.btn`). All state morphs run through the caller's [`Animator`], so
//! `reduce_motion` degrades every one of them to the standard 120ms fade.

use triton_design::color::STATE_LAYER;
use triton_design::icon::{Icon, IconStyle};
use triton_design::material::ShadowToken;
use triton_design::motion::Spring;
use triton_design::shape::{CORNER_SMOOTHING, RADII, SIZE, connected_corners};
use triton_engine::anim::Animator;
use triton_engine::style::{Corners, Edges, FontSpec, Rgba, Theme};
use triton_engine::ui::{
    ClickHandler, Element, InteractionState, NodeKey, box_, col, icon, row, text,
};

// ── Tones & sizes ────────────────────────────────────────────────────────────

/// Visual emphasis of a [`button`], [`icon_button`], or [`segmented`] cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum ButtonTone {
    /// `primary`/`on_primary` — the single high-emphasis action (`.t-btn`).
    #[default]
    Filled,
    /// `secondary_container`/`on_secondary_container` — medium emphasis.
    Tonal,
    /// Transparent with an inset 1px `outline` border, `primary` content.
    Outlined,
    /// Transparent, `primary` content, tighter `0 14` padding (`.t-btn.text`).
    Text,
    /// `surface_container_highest`/`on_surface_variant` — the unselected
    /// toggle look (`.t-ibtn.neutral`).
    Neutral,
    /// `theme.veil()`/`on_surface` — the fill for controls that sit on Glass,
    /// Thick, or a scrim, where a solid container reads as a hole.
    Veil,
    /// `error_container`/`on_error_container` — the only tone for destructive
    /// actions (dialogs, "Move to Trash"), never `Filled` primary.
    Destructive,
}

/// Triton control height token for [`button`] and [`icon_button`]: 32/40/48/56.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum ButtonSize {
    /// `control_s` 32px: pad `0 16`, 13px label.
    S,
    /// `control_m` 40px: pad `0 24`, 14px/600 label — the default `.t-btn`.
    #[default]
    M,
    /// `control_l` 48px: pad `0 28`, 16px label.
    L,
    /// `control_xl` 56px: pad `0 32`, 17px label.
    Xl,
}

impl ButtonSize {
    fn height(self) -> f32 {
        match self {
            Self::S => SIZE.control_s,
            Self::M => SIZE.control_m,
            Self::L => SIZE.control_l,
            Self::Xl => SIZE.control_xl,
        }
    }

    /// Horizontal padding for the solid/tonal tones (`.t-btn` per-size rules).
    fn padding(self) -> f32 {
        match self {
            Self::S => 16.0,
            Self::M => 24.0,
            Self::L => 28.0,
            Self::Xl => 32.0,
        }
    }

    fn font(self) -> FontSpec {
        let px = match self {
            Self::S => 13.0,
            Self::M => 14.0,
            Self::L => 16.0,
            Self::Xl => 17.0,
        };
        FontSpec::ui(px).weight(600)
    }

    /// Leading-glyph size: `icon_s` under `S`, `icon_m` otherwise.
    fn lead_icon(self) -> f32 {
        match self {
            Self::S => SIZE.icon_s,
            _ => SIZE.icon_m,
        }
    }
}

/// `(container, content)` colors for a tone on a text [`button`].
fn tone_colors(tone: ButtonTone, theme: &Theme) -> (Rgba, Rgba) {
    let s = &theme.scheme;
    match tone {
        ButtonTone::Filled => (Rgba::from_color(s.primary), Rgba::from_color(s.on_primary)),
        ButtonTone::Tonal => (
            Rgba::from_color(s.secondary_container),
            Rgba::from_color(s.on_secondary_container),
        ),
        ButtonTone::Outlined | ButtonTone::Text => (Rgba::TRANSPARENT, Rgba::from_color(s.primary)),
        ButtonTone::Neutral => (
            Rgba::from_color(s.surface_container_highest),
            Rgba::from_color(s.on_surface_variant),
        ),
        ButtonTone::Veil => (theme.veil(), Rgba::from_color(s.on_surface)),
        ButtonTone::Destructive => (
            Rgba::from_color(s.error_container),
            Rgba::from_color(s.on_error_container),
        ),
    }
}

/// `(container, content)` colors for a tone on an [`icon_button`]: the
/// transparent tones take `on_surface_variant` (`.t-ibtn` default) rather
/// than `primary`, since a bare icon carries no label emphasis.
fn icon_tone_colors(tone: ButtonTone, theme: &Theme) -> (Rgba, Rgba) {
    let s = &theme.scheme;
    match tone {
        ButtonTone::Outlined | ButtonTone::Text => {
            (Rgba::TRANSPARENT, Rgba::from_color(s.on_surface_variant))
        }
        _ => tone_colors(tone, theme),
    }
}

/// M3 disabled treatment: `on_surface` at the state-layer alphas. Outlined and
/// Text keep a transparent container; Outlined keeps its hairline.
fn disabled_colors(tone: ButtonTone, theme: &Theme) -> (Option<Rgba>, Rgba, Option<Rgba>) {
    let on_surface = Rgba::from_color(theme.scheme.on_surface);
    let content = on_surface.with_alpha(STATE_LAYER.disabled_content);
    let container = match tone {
        ButtonTone::Outlined | ButtonTone::Text => None,
        _ => Some(on_surface.with_alpha(STATE_LAYER.disabled_container)),
    };
    let border = (tone == ButtonTone::Outlined)
        .then(|| on_surface.with_alpha(STATE_LAYER.disabled_container));
    (container, content, border)
}

// ── button ───────────────────────────────────────────────────────────────────

/// Content and behavior of a [`button`].
pub struct Button<M> {
    /// Hit-test and animation key.
    pub key: NodeKey,
    pub label: String,
    /// Optional leading icon before the label (`.t-btn` 8px gap).
    pub lead_icon: Option<Icon>,
    pub tone: ButtonTone,
    pub size: ButtonSize,
    /// Emits no clicks and paints the M3 disabled alphas.
    pub disabled: bool,
    pub on_click: Option<ClickHandler<M>>,
}

impl<M> Default for Button<M> {
    fn default() -> Self {
        Self {
            key: NodeKey::named("triton-button"),
            label: String::new(),
            lead_icon: None,
            tone: ButtonTone::Filled,
            size: ButtonSize::M,
            disabled: false,
            on_click: None,
        }
    }
}

impl<M> std::fmt::Debug for Button<M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Button")
            .field("key", &self.key)
            .field("label", &self.label)
            .field("lead_icon", &self.lead_icon)
            .field("tone", &self.tone)
            .field("size", &self.size)
            .field("disabled", &self.disabled)
            .field("on_click", &self.on_click.is_some())
            .finish()
    }
}

/// Animator channel salts — XOR keeps each widget's channel keys unique per
/// `NodeKey` without the caller managing extra ids.
const CORNER_SALT: u64 = 0xb770_c0de_0000_0001;

/// `.t-btn` pill button: hover/press state layers, spring press
/// corner-tightening to `RADII.sm` (12px, board `:active{border-radius:12px}`)
/// on `Spring::SPATIAL_FAST`, optional leading icon, and M3 disabled alphas.
pub fn button<M: 'static>(
    theme: &Theme,
    interaction: &InteractionState,
    anim: &mut Animator,
    spec: Button<M>,
) -> Element<M> {
    let h = spec.size.height();
    let interactive = !spec.disabled && spec.on_click.is_some();

    let (bg, fg, border) = if spec.disabled {
        let (container, content, outline) = disabled_colors(spec.tone, theme);
        (container.unwrap_or(Rgba::TRANSPARENT), content, outline)
    } else {
        let (bg, fg) = tone_colors(spec.tone, theme);
        let outline =
            (spec.tone == ButtonTone::Outlined).then(|| Rgba::from_color(theme.scheme.outline));
        (bg, fg, outline)
    };

    // Press morphs the pill into a 12px squircle; release springs back.
    let rest = h * 0.5;
    let target_r = if interaction.is_pressed(spec.key) {
        RADII.sm
    } else {
        rest
    };
    let corners = anim.corners(
        NodeKey(spec.key.0 ^ CORNER_SALT),
        Corners::all(target_r),
        Spring::SPATIAL_FAST,
    );

    let pad = if spec.tone == ButtonTone::Text {
        14.0 // `.t-btn.text` tightens to 0 14 regardless of size
    } else {
        spec.size.padding()
    };

    let mut el = row()
        .id(spec.key)
        .h(h)
        .px(pad)
        .gap(8.0)
        .items_center()
        .justify_center()
        .rounded(corners)
        .smoothing(CORNER_SMOOTHING)
        .bg(bg)
        .child_opt(
            spec.lead_icon
                .map(|ic| icon(ic, IconStyle::Outlined, spec.size.lead_icon(), fg)),
        )
        .child(text(spec.label, spec.size.font(), fg));

    if let Some(outline) = border {
        el = el.border(1.0, outline);
    }
    if interactive {
        el = el
            .hover_bg(bg.over(fg, STATE_LAYER.hover))
            .press_bg(bg.over(fg, STATE_LAYER.press));
    }
    if let Some(click) = spec.on_click.filter(|_| !spec.disabled) {
        el = el.on_click(click);
    }
    el
}

// ── icon_button ──────────────────────────────────────────────────────────────

/// Content and behavior of an [`icon_button`].
pub struct IconButton<M> {
    /// Hit-test and animation key.
    pub key: NodeKey,
    pub icon: Icon,
    /// Selected/toggled state: squircle corners, `primary` fill, Filled glyph.
    pub toggled: bool,
    pub tone: ButtonTone,
    pub size: ButtonSize,
    /// Emits no clicks and paints the M3 disabled alphas.
    pub disabled: bool,
    pub on_click: Option<ClickHandler<M>>,
}

impl<M> Default for IconButton<M> {
    fn default() -> Self {
        Self {
            key: NodeKey::named("triton-icon-button"),
            icon: Icon::Settings,
            toggled: false,
            tone: ButtonTone::Text,
            size: ButtonSize::M,
            disabled: false,
            on_click: None,
        }
    }
}

impl<M> std::fmt::Debug for IconButton<M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IconButton")
            .field("key", &self.key)
            .field("icon", &self.icon)
            .field("toggled", &self.toggled)
            .field("tone", &self.tone)
            .field("size", &self.size)
            .field("disabled", &self.disabled)
            .field("on_click", &self.on_click.is_some())
            .finish()
    }
}

const TOGGLE_SALT: u64 = 0xb770_c0de_0000_0002;

/// `.t-ibtn` icon button: a circle at rest that morphs into a squircle when
/// `toggled` (`.toggled` r12 at 40px, r14 at 48px — `diameter * 0.3`), flips
/// the glyph to `IconStyle::Filled`, and takes the `primary` fill. The morph
/// runs on one `Spring::SPATIAL` progress channel so radius and smoothing move
/// together.
pub fn icon_button<M: 'static>(
    theme: &Theme,
    _interaction: &InteractionState,
    anim: &mut Animator,
    spec: IconButton<M>,
) -> Element<M> {
    let d = spec.size.height();
    let p = anim.spring(
        NodeKey(spec.key.0 ^ TOGGLE_SALT),
        f32::from(u8::from(spec.toggled)),
        Spring::SPATIAL,
    );

    let (bg, fg) = if spec.disabled {
        let (container, content, _) = disabled_colors(spec.tone, theme);
        (container.unwrap_or(Rgba::TRANSPARENT), content)
    } else if spec.toggled {
        (
            Rgba::from_color(theme.scheme.primary),
            Rgba::from_color(theme.scheme.on_primary),
        )
    } else {
        icon_tone_colors(spec.tone, theme)
    };

    // r = d/2 at rest → d*0.3 toggled (12 @40px, ~14 @48px); 0 → 0.6 smoothing.
    let toggled_r = (d * 0.3).round();
    let radius = (d * 0.5) + (toggled_r - d * 0.5) * p;
    let smoothing = CORNER_SMOOTHING * p;
    let glyph_style = if spec.toggled {
        IconStyle::Filled
    } else {
        IconStyle::Outlined
    };
    let glyph_size = (d * 0.5).clamp(SIZE.icon_s, SIZE.icon_l);

    let mut el = box_()
        .id(spec.key)
        .size(d, d)
        .items_center()
        .justify_center()
        .rounded(radius)
        .smoothing(smoothing)
        .bg(bg)
        .child(icon(spec.icon, glyph_style, glyph_size, fg));

    if !spec.disabled && spec.tone == ButtonTone::Outlined && !spec.toggled {
        el = el.border(1.0, Rgba::from_color(theme.scheme.outline));
    }
    if !spec.disabled && spec.on_click.is_some() {
        el = el
            .hover_bg(bg.over(fg, STATE_LAYER.hover))
            .press_bg(bg.over(fg, STATE_LAYER.press));
    }
    if let Some(click) = spec.on_click.filter(|_| !spec.disabled) {
        el = el.on_click(click);
    }
    el
}

// ── chip ─────────────────────────────────────────────────────────────────────

/// Behavior of a [`chip`]: selectable filter or read-only status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum ChipKind {
    /// Selectable filter chip: `outline` hairline at rest, fills
    /// `secondary_container`/`on_secondary_container` and shows a check when
    /// `selected` (`.t-chip.on`).
    #[default]
    Filter,
    /// Read-only neutral status on `surface_container_high`.
    StatusNeutral,
    /// Read-only warm status on `tertiary_container` ("Needs attention").
    StatusWarm,
    /// Read-only urgent status on `error_container`.
    StatusError,
}

/// Content and behavior of a [`chip`].
pub struct Chip<M> {
    /// Hit-test key.
    pub key: NodeKey,
    pub label: String,
    /// Leading glyph; a selected `Filter` chip replaces it with `check`.
    pub icon: Option<Icon>,
    pub kind: ChipKind,
    /// `Filter` only — swaps the hairline for the `secondary_container` fill.
    pub selected: bool,
    pub on_click: Option<ClickHandler<M>>,
    /// Trailing `close` affordance with its own hit area (input-chip dismiss).
    pub on_dismiss: Option<ClickHandler<M>>,
}

impl<M> Default for Chip<M> {
    fn default() -> Self {
        Self {
            key: NodeKey::named("triton-chip"),
            label: String::new(),
            icon: None,
            kind: ChipKind::Filter,
            selected: false,
            on_click: None,
            on_dismiss: None,
        }
    }
}

impl<M> std::fmt::Debug for Chip<M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Chip")
            .field("key", &self.key)
            .field("label", &self.label)
            .field("icon", &self.icon)
            .field("kind", &self.kind)
            .field("selected", &self.selected)
            .field("on_click", &self.on_click.is_some())
            .field("on_dismiss", &self.on_dismiss.is_some())
            .finish()
    }
}

const CHIP_DISMISS_SALT: u64 = 0xb770_c0de_0000_0003;

/// `.t-chip` 36px r10 chip: filter chips carry an `outline` hairline until
/// `selected` swaps it for `secondary_container` + a leading `check`; status
/// chips are read-only container fills. `on_dismiss` adds a trailing `close`
/// hit area (`input` chips).
pub fn chip<M: 'static>(theme: &Theme, spec: Chip<M>) -> Element<M> {
    let s = &theme.scheme;
    let selectable = spec.kind == ChipKind::Filter && spec.on_click.is_some();
    let (bg, fg, outlined) = match spec.kind {
        ChipKind::Filter if spec.selected => (
            Rgba::from_color(s.secondary_container),
            Rgba::from_color(s.on_secondary_container),
            false,
        ),
        ChipKind::Filter => (
            Rgba::TRANSPARENT,
            Rgba::from_color(s.on_surface_variant),
            true,
        ),
        ChipKind::StatusNeutral => (
            Rgba::from_color(s.surface_container_high),
            Rgba::from_color(s.on_surface),
            false,
        ),
        ChipKind::StatusWarm => (
            Rgba::from_color(s.tertiary_container),
            Rgba::from_color(s.on_tertiary_container),
            false,
        ),
        ChipKind::StatusError => (
            Rgba::from_color(s.error_container),
            Rgba::from_color(s.on_error_container),
            false,
        ),
    };

    // A selected filter chip leads with the check mark (`.t-chip.on` + board
    // "leading check on selected"), replacing any caller-supplied icon.
    let lead = if spec.selected && spec.kind == ChipKind::Filter {
        Some(Icon::Check)
    } else {
        spec.icon
    };

    let mut el = row()
        .id(spec.key)
        .h(36.0)
        .px(14.0)
        .gap(8.0)
        .items_center()
        .rounded(10.0)
        .smoothing(CORNER_SMOOTHING)
        .bg(bg)
        .child_opt(lead.map(|ic| icon(ic, IconStyle::Outlined, SIZE.icon_s, fg)))
        .child(text(spec.label, FontSpec::ui(14.0).weight(500), fg));

    if outlined {
        el = el.border(1.0, Rgba::from_color(s.outline));
    }
    if selectable {
        el = el
            .hover_bg(bg.over(fg, STATE_LAYER.hover))
            .press_bg(bg.over(fg, STATE_LAYER.press));
    }
    if let Some(click) = spec.on_click {
        el = el.on_click(click);
    }
    if let Some(dismiss) = spec.on_dismiss {
        let close = box_()
            .id(NodeKey(spec.key.0 ^ CHIP_DISMISS_SALT))
            .size(24.0, 24.0)
            .items_center()
            .justify_center()
            .rounded(12.0)
            .smoothing(0.0)
            .hover_bg(Rgba::TRANSPARENT.over(fg, STATE_LAYER.hover))
            .press_bg(Rgba::TRANSPARENT.over(fg, STATE_LAYER.press))
            .on_click(dismiss)
            .child(icon(Icon::Close, IconStyle::Outlined, SIZE.icon_s, fg));
        el = el.child(close);
    }
    el
}

// ── toggle_tile ──────────────────────────────────────────────────────────────

/// Content and behavior of a [`toggle_tile`] Quick Settings tile.
pub struct ToggleTile<M> {
    /// Hit-test and animation key.
    pub key: NodeKey,
    pub icon: Icon,
    pub title: String,
    /// Supporting line under the title (`.tile span`, 13px).
    pub subtitle: Option<String>,
    /// `primary`/`on_primary` when on, veil when off.
    pub on: bool,
    pub on_click: Option<ClickHandler<M>>,
    /// Trailing sub-page chevron with its own click handler (Wi-Fi → list).
    pub on_sub_page: Option<ClickHandler<M>>,
}

impl<M> Default for ToggleTile<M> {
    fn default() -> Self {
        Self {
            key: NodeKey::named("triton-toggle-tile"),
            icon: Icon::Wifi,
            title: String::new(),
            subtitle: None,
            on: false,
            on_click: None,
            on_sub_page: None,
        }
    }
}

impl<M> std::fmt::Debug for ToggleTile<M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToggleTile")
            .field("key", &self.key)
            .field("icon", &self.icon)
            .field("title", &self.title)
            .field("subtitle", &self.subtitle)
            .field("on", &self.on)
            .field("on_click", &self.on_click.is_some())
            .field("on_sub_page", &self.on_sub_page.is_some())
            .finish()
    }
}

const TILE_CORNER_SALT: u64 = 0xb770_c0de_0000_0004;
const TILE_BG_SALT: u64 = 0xb770_c0de_0000_0005;
const TILE_FG_SALT: u64 = 0xb770_c0de_0000_0006;
const TILE_CHEVRON_SALT: u64 = 0xb770_c0de_0000_0007;

/// Quick Settings `.tile`: a 72px r36 veil pill that morphs on `Spring::SPATIAL`
/// into an r20 `primary` squircle when `on` — corners, fill, and content color
/// all run through `anim`. Carries icon + 15px/600 title + optional 13px
/// subtitle, and a trailing `chevron_right` with a separate handler when the
/// toggle owns a sub-page.
pub fn toggle_tile<M: 'static>(
    theme: &Theme,
    anim: &mut Animator,
    spec: ToggleTile<M>,
) -> Element<M> {
    let s = &theme.scheme;
    let h = 72.0;
    let (target_bg, target_fg, target_sub) = if spec.on {
        (
            Rgba::from_color(s.primary),
            Rgba::from_color(s.on_primary),
            Rgba::from_color(s.on_primary).with_alpha(0.8),
        )
    } else {
        (
            theme.veil(),
            Rgba::from_color(s.on_surface),
            Rgba::from_color(s.on_surface_variant),
        )
    };
    let corners = anim.corners(
        NodeKey(spec.key.0 ^ TILE_CORNER_SALT),
        Corners::all(if spec.on { RADII.lg } else { h * 0.5 }),
        Spring::SPATIAL,
    );
    let bg = anim.color(
        NodeKey(spec.key.0 ^ TILE_BG_SALT),
        target_bg,
        Spring::SPATIAL,
    );
    let fg = anim.color(
        NodeKey(spec.key.0 ^ TILE_FG_SALT),
        target_fg,
        Spring::SPATIAL,
    );

    let mut texts =
        col()
            .flex_1()
            .gap(2.0)
            .child(text(spec.title, FontSpec::ui(15.0).weight(600), fg));
    if let Some(sub) = spec.subtitle {
        texts = texts.child(text(sub, FontSpec::ui(13.0), target_sub));
    }

    let mut el = row()
        .id(spec.key)
        .h(h)
        .w_full()
        .p(Edges::new(0.0, 10.0, 0.0, 12.0))
        .gap(12.0)
        .items_center()
        .rounded(corners)
        .smoothing(CORNER_SMOOTHING)
        .bg(bg)
        .child(icon(spec.icon, IconStyle::Outlined, SIZE.icon_l, fg))
        .child(texts);

    if let Some(sub_page) = spec.on_sub_page {
        let chevron = box_()
            .id(NodeKey(spec.key.0 ^ TILE_CHEVRON_SALT))
            .size(SIZE.control_m, SIZE.control_m)
            .items_center()
            .justify_center()
            .rounded(SIZE.control_m * 0.5)
            .smoothing(0.0)
            .hover_bg(Rgba::TRANSPARENT.over(fg, STATE_LAYER.hover))
            .press_bg(Rgba::TRANSPARENT.over(fg, STATE_LAYER.press))
            .on_click(sub_page)
            .child(icon(
                Icon::ChevronRight,
                IconStyle::Outlined,
                SIZE.icon_m,
                fg,
            ));
        el = el.child(chevron);
    }
    if spec.on_click.is_some() {
        el = el
            .hover_bg(bg.over(fg, STATE_LAYER.hover))
            .press_bg(bg.over(fg, STATE_LAYER.press));
    }
    if let Some(click) = spec.on_click {
        el = el.on_click(click);
    }
    el
}

// ── segmented ────────────────────────────────────────────────────────────────

/// Look of a [`segmented`] group (`.t-seg`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum SegmentedStyle {
    /// h44 cells on `surface_container_high`, `connected_end` (22px) outer radii.
    #[default]
    Standard,
    /// `.t-seg.l` — h48 cells, 24px outer radii, 15px labels.
    Large,
    /// `.t-seg.veil` — standard sizing, `theme.veil()` cells on Glass/scrim.
    Veil,
}

impl SegmentedStyle {
    fn height(self) -> f32 {
        match self {
            Self::Large => SIZE.control_l,
            _ => 44.0,
        }
    }

    fn end_radius(self) -> f32 {
        match self {
            Self::Large => 24.0,
            _ => RADII.connected_end,
        }
    }

    fn font(self) -> FontSpec {
        match self {
            Self::Large => FontSpec::ui(15.0).weight(600),
            _ => FontSpec::ui(14.0).weight(600),
        }
    }
}

/// One cell of a [`segmented`] group.
pub struct SegmentedItem<M> {
    /// Leading 20px glyph.
    pub icon: Option<Icon>,
    pub label: Option<String>,
    pub on_click: Option<ClickHandler<M>>,
}

impl<M> std::fmt::Debug for SegmentedItem<M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SegmentedItem")
            .field("icon", &self.icon)
            .field("label", &self.label)
            .field("on_click", &self.on_click.is_some())
            .finish()
    }
}

const SEG_CELL_SALT: u64 = 0xb770_c0de_0000_0008;

/// `.t-seg` connected button group: `flex:1` cells with a 3px gap and
/// [`connected_corners`] joins (8px inner, `connected_end` outer). The
/// `selected` cell paints `primary`/`on_primary` (`.on`); unselected cells
/// take the style fill — `surface_container_high`, or `theme.veil()` on Glass.
pub fn segmented<M: 'static>(
    key: impl Into<NodeKey>,
    style: SegmentedStyle,
    selected: Option<usize>,
    theme: &Theme,
    items: Vec<SegmentedItem<M>>,
) -> Element<M> {
    let group_key = key.into();
    let s = &theme.scheme;
    let count = items.len();
    let h = style.height();
    let end = style.end_radius();

    let cells = items.into_iter().enumerate().map(|(i, item)| {
        let cell_key = NodeKey(group_key.0 ^ SEG_CELL_SALT ^ (i as u64 + 1));
        let on = selected == Some(i);
        let (bg, fg) = if on {
            (Rgba::from_color(s.primary), Rgba::from_color(s.on_primary))
        } else {
            match style {
                SegmentedStyle::Veil => (theme.veil(), Rgba::from_color(s.on_surface)),
                _ => (
                    Rgba::from_color(s.surface_container_high),
                    Rgba::from_color(s.on_surface),
                ),
            }
        };
        let mut cell = row()
            .id(cell_key)
            .h(h)
            .flex_1()
            .px(20.0)
            .gap(8.0)
            .items_center()
            .justify_center()
            .rounded(Corners(connected_corners(i, count, end)))
            .smoothing(CORNER_SMOOTHING)
            .bg(bg)
            .child_opt(
                item.icon
                    .map(|ic| icon(ic, IconStyle::Outlined, SIZE.icon_m, fg)),
            )
            .child_opt(item.label.map(|l| text(l, style.font(), fg)));
        if let Some(click) = item.on_click {
            cell = cell
                .hover_bg(bg.over(fg, STATE_LAYER.hover))
                .press_bg(bg.over(fg, STATE_LAYER.press))
                .on_click(click);
        }
        cell
    });

    row().id(group_key).w_full().gap(3.0).children(cells)
}

// ── power_menu ───────────────────────────────────────────────────────────────

/// Compact-cell fill of a [`power_menu`] action (Power board `--pc/--sc/--tc`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum PowerCellTone {
    /// `primary_container`/`on_primary_container` — the default first action.
    Primary,
    /// `surface_container_high`/`on_surface` — neutral actions.
    #[default]
    Neutral,
    /// `tertiary_container`/`on_tertiary_container` — caution (restart).
    Tertiary,
}

/// One cell of a [`power_menu`] row.
pub struct PowerItem<M> {
    /// 44px glyph at rest (52px Filled when expanded).
    pub icon: Icon,
    /// Compact caption and the expanded title.
    pub label: String,
    /// Secondary line under the expanded title (board: "Press Enter").
    pub hint: Option<String>,
    pub tone: PowerCellTone,
    pub on_click: Option<ClickHandler<M>>,
}

impl<M> std::fmt::Debug for PowerItem<M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PowerItem")
            .field("icon", &self.icon)
            .field("label", &self.label)
            .field("hint", &self.hint)
            .field("tone", &self.tone)
            .field("on_click", &self.on_click.is_some())
            .finish()
    }
}

const POWER_CELL: f32 = 140.0;
const POWER_EXPANDED: f32 = 300.0;
/// Power-board inner join radius (`border-radius:14px` on unfocused cells) —
/// larger than `connected_inner` since the cells are much taller.
const POWER_INNER: f32 = 14.0;
const POWER_PROGRESS_SALT: u64 = 0xb770_c0de_0000_0009;
const POWER_BG_SALT: u64 = 0xb770_c0de_0000_000a;
const POWER_FG_SALT: u64 = 0xb770_c0de_0000_000b;
const POWER_CELL_SALT: u64 = 0xb770_c0de_0000_000c;

fn power_tone_colors(tone: PowerCellTone, theme: &Theme) -> (Rgba, Rgba) {
    let s = &theme.scheme;
    match tone {
        PowerCellTone::Primary => (
            Rgba::from_color(s.primary_container),
            Rgba::from_color(s.on_primary_container),
        ),
        PowerCellTone::Neutral => (
            Rgba::from_color(s.surface_container_high),
            Rgba::from_color(s.on_surface),
        ),
        PowerCellTone::Tertiary => (
            Rgba::from_color(s.tertiary_container),
            Rgba::from_color(s.on_tertiary_container),
        ),
    }
}

/// Power-board connected row: 140×140 cells (`70/14` end caps, 6px gap) where
/// the `focused` cell expands on `Spring::SPATIAL` into a ~300px `tertiary`
/// pill — icon + Jost title + hint — while the rest stay compact icon+label
/// tiles. One progress channel drives width, radii, smoothing, and the
/// compact/expanded content crossfade so every property moves together.
pub fn power_menu<M: 'static>(
    key: impl Into<NodeKey>,
    focused: usize,
    theme: &Theme,
    anim: &mut Animator,
    items: Vec<PowerItem<M>>,
) -> Element<M> {
    let group_key = key.into();
    let count = items.len();
    let expanded_bg = Rgba::from_color(theme.scheme.tertiary);
    let expanded_fg = Rgba::from_color(theme.scheme.on_tertiary);

    let cells = items.into_iter().enumerate().map(|(i, item)| {
        let cell_key = NodeKey(group_key.0 ^ POWER_CELL_SALT ^ (i as u64 + 1));
        let on = i == focused;
        let progress = anim.spring(
            NodeKey(cell_key.0 ^ POWER_PROGRESS_SALT),
            f32::from(u8::from(on)),
            Spring::SPATIAL,
        );
        let (rest_bg, rest_fg) = power_tone_colors(item.tone, theme);
        let bg = anim.color(
            NodeKey(cell_key.0 ^ POWER_BG_SALT),
            if on { expanded_bg } else { rest_bg },
            Spring::SPATIAL,
        );
        let fg = anim.color(
            NodeKey(cell_key.0 ^ POWER_FG_SALT),
            if on { expanded_fg } else { rest_fg },
            Spring::SPATIAL,
        );

        let width = POWER_CELL + (POWER_EXPANDED - POWER_CELL) * progress;
        let half = POWER_CELL * 0.5;
        let rest_l = if i == 0 { half } else { POWER_INNER };
        let rest_r = if i + 1 == count { half } else { POWER_INNER };
        let left = rest_l + (half - rest_l) * progress;
        let right = rest_r + (half - rest_r) * progress;
        let corners = Corners::each(left, right, right, left).clamped(width, POWER_CELL);
        let smoothing = CORNER_SMOOTHING * (1.0 - progress.clamp(0.0, 1.0));

        // Compact content: icon over 15px caption (`.btn` column layout).
        let compact = col()
            .absolute()
            .inset_all(0.0)
            .items_center()
            .justify_center()
            .gap(10.0)
            .opacity(1.0 - progress)
            .child(icon(item.icon, IconStyle::Outlined, 44.0, fg))
            .child(text(item.label.clone(), FontSpec::ui(15.0).weight(600), fg));

        // Expanded content: row of Filled 52px glyph + Jost title + hint.
        let mut texts =
            col()
                .gap(6.0)
                .child(text(item.label, FontSpec::display(32.0).weight(600), fg));
        if let Some(hint) = item.hint {
            texts = texts.child(text(
                hint,
                FontSpec::ui(13.0).weight(500),
                fg.with_alpha(0.8),
            ));
        }
        let expanded = row()
            .absolute()
            .inset_all(0.0)
            .items_center()
            .justify_center()
            .gap(16.0)
            .opacity(progress)
            .child(icon(item.icon, IconStyle::Filled, 52.0, fg))
            .child(texts);

        let mut cell = box_()
            .id(cell_key)
            .w(width)
            .h(POWER_CELL)
            .clip()
            .rounded(Corners(corners))
            .smoothing(smoothing)
            .bg(bg)
            .child(compact)
            .child(expanded);

        // The expanded pill floats above the row (board `0 14px 40px` shadow).
        if progress > 0.05 {
            cell = cell.shadow(ShadowToken::FLOATING);
        }
        if let Some(click) = item.on_click {
            cell = cell
                .hover_bg(bg.over(fg, STATE_LAYER.hover))
                .press_bg(bg.over(fg, STATE_LAYER.press))
                .on_click(click);
        }
        cell
    });

    row()
        .id(group_key)
        .gap(6.0)
        .items_center()
        .justify_center()
        .children(cells)
}
