//! Overlays and notification surfaces: the concentric [`menu`] with nested
//! submenu support, the floating Glass [`tooltip`], and the Material 3 Futura
//! [`toast_card`] notification family with dismiss routing and a hover-pause
//! hook for auto-dismiss timers.
//!
//! Metrics come from `triton-design` tokens and the board reference
//! (`references/Triton Shell-project/{Containers,Notifications,Controls,
//! Materials}.dc.html`: `.mi`, `.nc`, `.b`, the "tooltip · glass" swatch).

use triton_design::color::STATE_LAYER;
use triton_design::icon::{Icon, IconStyle};
use triton_design::material::{Material, ShadowToken};
use triton_design::shape::{CORNER_SMOOTHING, RADII, SIZE, nested};
use triton_design::typography::TypeStyle;
use triton_engine::anim::Animator;
use triton_engine::style::{Corners, Edges, FontSpec, Rgba, Theme};
use triton_engine::ui::{
    ClickHandler, Element, InteractionState, NodeKey, box_, col, icon, row, text,
};

use crate::chrome::{icon_disc_styled, wave_avatar, wave_avatar_image};
use crate::progress::{Progress, wavy_progress};
use crate::surface::{DividerStyle, Orientation, divider, panel};

// ── Menu ─────────────────────────────────────────────────────────────────────

/// Inset between a menu panel's `RADII.xl` edge and its rows, giving the
/// concentric `nested(28, 4) = 24` full-pill rows the board calls for.
const MENU_PAD: f32 = 4.0;
/// Hover tint on menu rows (Containers board `.mi.hov`: `primary` at 16%).
const MENU_HOVER_ALPHA: f32 = 0.16;
/// Press tint sits one state-layer step stronger than hover.
const MENU_PRESS_ALPHA: f32 = 0.24;
/// Gap between a parent menu's right edge and an open submenu panel.
const SUBMENU_GAP: f32 = 8.0;

/// Content and behavior of a [`menu`] panel.
pub struct Menu<M> {
    /// Hit-test key for the panel itself (padding clicks stop here, so a press
    /// started inside the menu can never release onto content beneath it).
    pub key: NodeKey,
    /// Fixed panel width in logical px; row content clips are the caller's
    /// concern (labels should stay short per the board grammar).
    pub width: f32,
    pub entries: Vec<MenuEntry<M>>,
    /// Key of the [`MenuItem`] whose `submenu` is currently expanded; when set
    /// the secondary Glass menu renders beside its row.
    pub open_submenu: Option<NodeKey>,
}

impl<M> Default for Menu<M> {
    fn default() -> Self {
        Self {
            key: NodeKey::named("triton-menu"),
            width: 260.0,
            entries: Vec::new(),
            open_submenu: None,
        }
    }
}

impl<M> std::fmt::Debug for Menu<M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Menu")
            .field("key", &self.key)
            .field("width", &self.width)
            .field("entries", &self.entries.len())
            .field("open_submenu", &self.open_submenu)
            .finish()
    }
}

/// One row of a [`menu`]: either an interactive [`MenuItem`] or a
/// [`Separator`](MenuEntry::Separator) hairline between sections.
#[non_exhaustive]
pub enum MenuEntry<M> {
    Item(MenuItem<M>),
    /// 1px [`divider`] in the on-glass tone, inset like the board's
    /// `margin: 6px 14px` rule.
    Separator,
}

impl<M> std::fmt::Debug for MenuEntry<M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Item(item) => f.debug_tuple("Item").field(item).finish(),
            Self::Separator => f.write_str("Separator"),
        }
    }
}

/// Content and behavior of a single `.mi` menu row.
pub struct MenuItem<M> {
    /// Hit-test key — also matches against [`Menu::open_submenu`].
    pub key: NodeKey,
    /// Optional leading glyph (22px, `on_surface_variant` per `.mi .t-ms`).
    pub icon: Option<Icon>,
    pub label: String,
    /// Right-aligned shortcut hint (`.mi .k`: JetBrains Mono 13px,
    /// `on_surface_variant`).
    pub shortcut: Option<String>,
    /// Muted content and no click — M3 disabled alpha throughout.
    pub disabled: bool,
    /// Destructive entry (`error` content — the board's "Move to Trash" row).
    pub danger: bool,
    /// Nested menu spec; renders the secondary Glass panel beside this row
    /// when [`Menu::open_submenu`] names this item's `key`. Always paints the
    /// trailing `chevron_right` indicator.
    pub submenu: Option<Box<Menu<M>>>,
    /// Item activation message; suppressed while `disabled`.
    pub on_click: Option<ClickHandler<M>>,
}

impl<M> Default for MenuItem<M> {
    fn default() -> Self {
        Self {
            key: NodeKey::named("triton-menu-item"),
            icon: None,
            label: String::new(),
            shortcut: None,
            disabled: false,
            danger: false,
            submenu: None,
            on_click: None,
        }
    }
}

impl<M> std::fmt::Debug for MenuItem<M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MenuItem")
            .field("key", &self.key)
            .field("icon", &self.icon)
            .field("label", &self.label)
            .field("shortcut", &self.shortcut)
            .field("disabled", &self.disabled)
            .field("danger", &self.danger)
            .field("submenu", &self.submenu.is_some())
            .field("on_click", &self.on_click.is_some())
            .finish()
    }
}

/// `.mi` context/dropdown menu: a `RADII.xl` (28px) Glass panel with a 4px
/// inset, so rows land on the concentric `r24` full-pill radius — hover paints
/// `primary` at 16% and press a step stronger.
///
/// The panel is [`Element::input_opaque`]: clicks on its padding cannot bleed
/// through to whatever the menu floats above. Nest menus by giving a
/// [`MenuItem`] a `submenu` spec and naming its key in
/// [`Menu::open_submenu`] — the secondary panel is the same widget, so submenus
/// compose to any depth.
pub fn menu<M: 'static>(theme: &Theme, spec: Menu<M>) -> Element<M> {
    let row_radius = nested(RADII.xl, MENU_PAD);
    let width = spec.width;
    let open = spec.open_submenu;
    panel(Material::Glass, RADII.xl, theme)
        .id(spec.key)
        .w(width)
        .p(Edges::all(MENU_PAD))
        .input_opaque()
        .children(spec.entries.into_iter().map(|entry| match entry {
            MenuEntry::Separator => box_().w_full().px(10.0).py(6.0).child(divider(
                Orientation::Horizontal,
                DividerStyle::OnGlass,
                theme,
            )),
            MenuEntry::Item(item) => menu_row(item, open, width, row_radius, theme),
        }))
}

fn menu_row<M: 'static>(
    item: MenuItem<M>,
    open_submenu: Option<NodeKey>,
    menu_width: f32,
    row_radius: f32,
    theme: &Theme,
) -> Element<M> {
    let MenuItem {
        key,
        icon: lead,
        label,
        shortcut,
        disabled,
        danger,
        submenu,
        on_click,
    } = item;
    let s = &theme.scheme;
    let has_submenu = submenu.is_some();
    let is_open = has_submenu && open_submenu == Some(key);

    let (label_c, detail_c) = if disabled {
        let muted = Rgba::from_color(s.on_surface).with_alpha(STATE_LAYER.disabled_content);
        (muted, muted)
    } else if danger {
        let error = Rgba::from_color(s.error);
        (error, error)
    } else {
        (
            Rgba::from_color(s.on_surface),
            Rgba::from_color(s.on_surface_variant),
        )
    };

    let mut el = row()
        .id(key)
        .h(SIZE.control_l)
        .px(16.0)
        .gap(14.0)
        .items_center()
        .rounded(row_radius)
        .smoothing(0.0) // full-pill rows keep true semicircular ends
        .child_opt(lead.map(|g| icon(g, IconStyle::Outlined, 22.0, detail_c)))
        .child(text(label, FontSpec::ui(15.0).weight(500), label_c))
        .child(box_().flex_1())
        .child_opt(shortcut.map(|sc| text(sc, FontSpec::mono(13.0), detail_c)))
        .child_opt(
            has_submenu.then(|| icon(Icon::ChevronRight, IconStyle::Outlined, 22.0, detail_c)),
        );

    if !disabled {
        // `input_opaque` keeps the row hit-visible for hover/press feedback
        // even without a click handler (submenu parents), and a press started
        // on it can never release onto content beneath the menu.
        el = el
            .input_opaque()
            .hover_bg(Rgba::from_color(s.primary).with_alpha(MENU_HOVER_ALPHA))
            .press_bg(Rgba::from_color(s.primary).with_alpha(MENU_PRESS_ALPHA));
        if let Some(click) = on_click {
            el = el.on_click(click);
        }
    }

    if is_open {
        // `submenu` presence is implied by `is_open`; `let-else` keeps the
        // invariant local instead of reaching for `Option` unwraps.
        let Some(sub) = submenu else { return el };
        let sub_panel = menu(theme, *sub)
            .absolute()
            .inset_xy(menu_width - MENU_PAD + SUBMENU_GAP, -MENU_PAD);
        return box_().w_full().child(el).child(sub_panel);
    }
    el
}

// ── Tooltip ──────────────────────────────────────────────────────────────────

/// "tooltip · glass" floating label: a 32px `RADII.sm` pill in
/// [`Material::Glass`] (tint, 1px top highlight, hairline, compositor blur),
/// `px 12`, 13px/500 text, and the board's subtle raised shadow
/// (`0 6 16 #000@.4`).
///
/// Wrap in [`crate::shadow_gutter`] when the pill sits near a layer-surface
/// edge so the shadow can reach zero alpha without clipping.
pub fn tooltip<M>(label: impl Into<String>, theme: &Theme) -> Element<M> {
    row()
        .h(32.0)
        .px(12.0)
        .items_center()
        .material(Material::Glass, theme)
        .rounded(RADII.sm)
        .smoothing(0.0) // pill reads as a pure radius, matching niri corners
        .shadow(triton_engine::style::DropShadow {
            offset_x: 0.0,
            offset_y: 6.0,
            blur: 16.0,
            spread: 0.0,
            color: Rgba::new(0.0, 0.0, 0.0, 0.4),
        })
        .child(text(
            label,
            FontSpec::ui(13.0).weight(500),
            Rgba::from_color(theme.scheme.on_surface),
        ))
}

// ── Toast card ───────────────────────────────────────────────────────────────

/// Board `.nc` translucency for a critical card: `error_container` at 78%
/// keeps the compositor blur working underneath, exactly like the reference's
/// `rgb(147 0 10 / .78)`.
const CRITICAL_FILL_ALPHA: f32 = 0.78;

/// Animation / hit-test channel salts (XOR-bijective off the card's `NodeKey`).
const TOAST_DISMISS_SALT: u64 = 0x746f_615f_6469_736d; // "toa_dism"
const TOAST_PROGRESS_SALT: u64 = 0x746f_615f_7072_6f67; // "toa_prog"

/// Visual family of a [`toast_card`] (Notifications board card types).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum ToastVariant {
    /// Glass fill, `on_surface` / `on_surface_variant` content — auto-dismiss
    /// timing is the caller's timer.
    Standard,
    /// `error_container` fill at 78% over the same Glass blur +
    /// `on_error_container` content; stays until dismissed and ignores DND.
    Critical,
    /// Live transfer/install card: composes [`crate::wavy_progress`] plus a
    /// percent readout. `label` replaces the auto `{value}%` text when set.
    Progress { value: f32, label: Option<String> },
}

/// Optional leading identity element of a [`toast_card`] (48px — the `.hx`
/// wave avatar for contacts or a `.t-ic` app icon disc).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum ToastIdentity {
    /// Contact initials on a 9-lobe `tertiary_container` Wave.
    Initials(String),
    /// App glyph on a `secondary_container` disc (app/file icons are circles,
    /// never Waves).
    AppIcon(Icon),
    /// Contact photo / album art clipped to the Wave.
    Image(
        triton_engine::image::ImageId,
        triton_engine::image::ImageStyle,
    ),
}

/// Emphasis of a [`ToastAction`] button (`.b.pf` / `.b.pt` / `.b.pe` / `.b.pg`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum ToastActionTone {
    /// `primary`/`on_primary` — the high-emphasis reply/accept action.
    #[default]
    Primary,
    /// Translucent `veil_hi` layer with the card's own content color — quiet
    /// actions like "Dismiss" that must work on Glass *and* on
    /// `error_container`.
    Tonal,
    /// `error_container`/`on_error_container` — destructive actions.
    Destructive,
    /// Call-accept green (`#8CD6A0`/`#00391B` on the board's `.b.pg`).
    Accept,
}

/// A 36px pill action button in a [`toast_card`] (`.b`: h36 r18, pad `0 16`,
/// 13px/600).
pub struct ToastAction<M> {
    /// Hit-test key.
    pub key: NodeKey,
    pub label: String,
    /// Optional leading glyph at 18px.
    pub icon: Option<Icon>,
    pub tone: ToastActionTone,
    pub on_click: Option<ClickHandler<M>>,
}

impl<M> Default for ToastAction<M> {
    fn default() -> Self {
        Self {
            key: NodeKey::named("triton-toast-action"),
            label: String::new(),
            icon: None,
            tone: ToastActionTone::Primary,
            on_click: None,
        }
    }
}

impl<M> std::fmt::Debug for ToastAction<M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToastAction")
            .field("key", &self.key)
            .field("label", &self.label)
            .field("icon", &self.icon)
            .field("tone", &self.tone)
            .field("on_click", &self.on_click.is_some())
            .finish()
    }
}

/// Content and behavior of a [`toast_card`] notification (`.nc`).
pub struct ToastCard<M> {
    /// Hit-test + hover-pause key; the card is marked [`Element::hoverable`]
    /// so [`toast_is_hovered`] stays true over its child buttons.
    pub key: NodeKey,
    /// Fixed card width in logical px (board toasts sit in a ~400px column).
    pub width: f32,
    pub variant: ToastVariant,
    pub identity: Option<ToastIdentity>,
    /// 16px/600 headline (`.nc .hd b`).
    pub title: Option<String>,
    /// 12px `on_surface_variant` timestamp (`.nc .hd span`).
    pub timestamp: Option<String>,
    /// 14px `on_surface_variant` body copy (`.nc .bd`, wraps).
    pub body: Option<String>,
    /// Right-to-left trailing action row of [`ToastAction`] buttons.
    pub actions: Vec<ToastAction<M>>,
    /// Renders the dedicated 24px `close` button in the header; its click
    /// emits this message without triggering `on_activate`.
    pub on_dismiss: Option<ClickHandler<M>>,
    /// Card-body activation (tap to open the app). Never fires from the
    /// dismiss or action buttons — dispatch routes the click to the deepest
    /// hit target only.
    pub on_activate: Option<ClickHandler<M>>,
}

impl<M> Default for ToastCard<M> {
    fn default() -> Self {
        Self {
            key: NodeKey::named("triton-toast"),
            width: 360.0,
            variant: ToastVariant::Standard,
            identity: None,
            title: None,
            timestamp: None,
            body: None,
            actions: Vec::new(),
            on_dismiss: None,
            on_activate: None,
        }
    }
}

impl<M> std::fmt::Debug for ToastCard<M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToastCard")
            .field("key", &self.key)
            .field("width", &self.width)
            .field("variant", &self.variant)
            .field("identity", &self.identity)
            .field("title", &self.title)
            .field("timestamp", &self.timestamp)
            .field("body", &self.body)
            .field("actions", &self.actions.len())
            .field("on_dismiss", &self.on_dismiss.is_some())
            .field("on_activate", &self.on_activate.is_some())
            .finish()
    }
}

/// Material 3 Futura notification card (`.nc`): a Glass floating surface with
/// the board's asymmetric corners — `r24` everywhere except a tight `10px`
/// bottom-right (`Corners` order `tl, tr, br, bl`), the corner that points
/// away from the screen edge the toast arrived from.
///
/// The card is [`Element::input_opaque`] (a press on the card can never
/// release onto surfaces beneath) and `hoverable`, so
/// [`toast_is_hovered`] can pause the auto-dismiss timer even while the
/// pointer is over the dismiss or action buttons.
pub fn toast_card<M: 'static>(
    theme: &Theme,
    anim: &mut Animator,
    spec: ToastCard<M>,
) -> Element<M> {
    let s = &theme.scheme;
    let critical = matches!(spec.variant, ToastVariant::Critical);
    let (content, muted) = if critical {
        (
            Rgba::from_color(s.on_error_container),
            Rgba::from_color(s.on_error_container),
        )
    } else {
        (
            Rgba::from_color(s.on_surface),
            Rgba::from_color(s.on_surface_variant),
        )
    };
    let identity_w = if spec.identity.is_some() { 62.0 } else { 0.0 };

    let mut card = row()
        .id(spec.key)
        .hoverable()
        .input_opaque()
        .w(spec.width)
        .material(Material::Glass, theme)
        .shadow(ShadowToken::FLOATING)
        .rounded(Corners::each(24.0, 24.0, 10.0, 24.0))
        .smoothing(CORNER_SMOOTHING)
        .p(Edges::new(14.0, 16.0, 14.0, 14.0))
        .gap(14.0);
    if critical {
        // Keep Glass's blur + highlight + hairline, swap the tint.
        card = card.bg(Rgba::from_color(s.error_container).with_alpha(CRITICAL_FILL_ALPHA));
    }
    if let Some(activate) = spec.on_activate {
        card = card.on_click(activate);
    }

    card = card.child_opt(spec.identity.map(|identity| match identity {
        ToastIdentity::Initials(initials) => wave_avatar(initials, 48.0, theme),
        ToastIdentity::AppIcon(glyph) => {
            icon_disc_styled(glyph, 48.0, s.secondary_container, s.on_secondary_container)
        }
        ToastIdentity::Image(id, style) => wave_avatar_image(id, style, 48.0),
    }));

    // ── text column: header (title + timestamp + dismiss), body, extras ──
    let mut hd = row().w_full().gap(10.0).items_center();
    if let Some(title) = spec.title {
        hd = hd.child(text(title, TypeStyle::TITLE_M, content));
    }
    hd = hd.child(box_().flex_1());
    if let Some(time) = spec.timestamp {
        hd = hd.child(text(time, FontSpec::ui(12.0), muted));
    }
    if let Some(dismiss) = spec.on_dismiss {
        hd = hd.child(
            box_()
                .id(NodeKey(spec.key.0 ^ TOAST_DISMISS_SALT))
                .size(24.0, 24.0)
                .items_center()
                .justify_center()
                .rounded(12.0)
                .smoothing(0.0)
                .hover_bg(Rgba::TRANSPARENT.over(content, STATE_LAYER.hover))
                .press_bg(Rgba::TRANSPARENT.over(content, STATE_LAYER.press))
                .on_click(dismiss)
                .child(icon(Icon::Close, IconStyle::Outlined, SIZE.icon_s, muted)),
        );
    }

    let mut tx = col().flex_1().gap(2.0).child(hd);
    if let Some(body) = spec.body {
        tx = tx.child(text(body, FontSpec::ui(14.0), muted).text_wrap());
    }

    if let ToastVariant::Progress { value, label } = spec.variant {
        let p = value.clamp(0.0, 1.0);
        let status = label.unwrap_or_else(|| format!("{:.0}%", p * 100.0));
        // Bar spans the text column minus the readout and its 10px gap.
        let bar_w = (spec.width - 30.0 - identity_w - 50.0).max(24.0);
        tx = tx.child(
            row()
                .gap(10.0)
                .items_center()
                .m(Edges::new(8.0, 0.0, 0.0, 0.0))
                .child(wavy_progress(
                    NodeKey(spec.key.0 ^ TOAST_PROGRESS_SALT),
                    bar_w,
                    3.5,
                    Progress::Determinate(value),
                    theme,
                    anim,
                ))
                .child(text(status, FontSpec::ui(12.0), muted)),
        );
    }

    if !spec.actions.is_empty() {
        tx = tx.child(
            row()
                .w_full()
                .gap(8.0)
                .items_center()
                .m(Edges::new(10.0, 0.0, 0.0, 0.0))
                .children(
                    spec.actions
                        .into_iter()
                        .map(|a| toast_action(a, content, theme)),
                ),
        );
    }

    card.child(tx)
}

/// `.b` toast action: h36 r18 pill, pad `0 16`, 13px/600. `Tonal` paints the
/// card's own content color so it reads on Glass and `error_container` alike.
fn toast_action<M: 'static>(action: ToastAction<M>, content: Rgba, theme: &Theme) -> Element<M> {
    let s = &theme.scheme;
    let (bg, fg) = match action.tone {
        ToastActionTone::Primary => (Rgba::from_color(s.primary), Rgba::from_color(s.on_primary)),
        ToastActionTone::Tonal => (theme.veil_hi(), content),
        ToastActionTone::Destructive => (
            Rgba::from_color(s.error_container),
            Rgba::from_color(s.on_error_container),
        ),
        ToastActionTone::Accept => (
            Rgba::new(0.549, 0.839, 0.627, 1.0), // #8CD6A0
            Rgba::new(0.0, 0.224, 0.106, 1.0),   // #00391B
        ),
    };

    let mut el = row()
        .id(action.key)
        .h(36.0)
        .px(16.0)
        .gap(6.0)
        .items_center()
        .justify_center()
        .rounded(18.0)
        .smoothing(0.0)
        .bg(bg)
        .child_opt(
            action
                .icon
                .map(|g| icon(g, IconStyle::Outlined, SIZE.icon_s, fg)),
        )
        .child(text(action.label, FontSpec::ui(13.0).weight(600), fg));
    if action.on_click.is_some() {
        el = el
            .hover_bg(bg.over(fg, STATE_LAYER.hover))
            .press_bg(bg.over(fg, STATE_LAYER.press));
    }
    if let Some(click) = action.on_click {
        el = el.on_click(click);
    }
    el
}

/// Whether the pointer is anywhere inside the toast keyed by `key` — including
/// over its dismiss and action buttons — so the caller can drive
/// `AppCommand::PauseTimer` / `ResumeTimer` without the timer racing while the
/// user reaches for a button (Notifications board: "hover pauses the timer").
pub fn toast_is_hovered(key: impl Into<NodeKey>, interaction: &InteractionState) -> bool {
    interaction.is_hovered_within(key.into())
}
