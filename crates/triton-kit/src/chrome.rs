//! Bar chrome and identity pieces: the spring-morphing workspace [`pill`],
//! [`status_pill`] / [`overflow_pill`] bar chrome, the `.t-ic` [`icon_disc`],
//! the Wave [`wave_avatar`], notification [`badge`]s, [`keycap`]s, and the
//! keyboard [`hint_bar`] footer grammar.
//!
//! Numbers come from `triton-design` tokens and the board reference
//! (`references/Triton Shell-project/triton/components.css`, `.ws`/`.stat`/
//! `.t-ic`/`.hx`/`.bd`/`.t-kbd`/`.kc` classes).

use triton_design::color::STATE_LAYER;
use triton_design::icon::{Icon, IconStyle};
use triton_design::motion::Spring;
use triton_design::shape::{CORNER_SMOOTHING, RADII, SIZE};
use triton_engine::anim::Animator;
use triton_engine::image::{ImageId, ImageStyle};
use triton_engine::style::{Edges, FontSpec, Rgba, Theme};
use triton_engine::ui::{ClickHandler, Element, NodeKey, box_, col, icon, image, row, text};

// ── Workspace pill ───────────────────────────────────────────────────────────

/// Animation channel salts: XOR is a bijection, so channels derived from the
/// caller's `NodeKey` can never collide with each other or across pills.
const PILL_MORPH_SALT: u64 = 0x7773_5f6d_6f72_7068; // "ws_morph"
const STAT_BG_SALT: u64 = 0x7374_6174_5f62_6700; // "stat_bg"
const STAT_FG_SALT: u64 = 0x7374_6174_5f66_6700; // "stat_fg"

/// `.ws` workspace pill: an idle 40px dot that spring-morphs into a 56px
/// active pill — width, corner radius (h/2 → 12 squircle), smoothing, and
/// `veil`/`on_surface_variant` → `primary`/`on_primary` all derive from one
/// `SPATIAL_FAST` morph channel keyed by `key` inside `anim`.
///
/// The label is Jost 17/600 (the board's numeric workspace grammar).
pub fn pill<M: 'static>(
    key: impl Into<NodeKey>,
    label: impl Into<String>,
    active: bool,
    theme: &Theme,
    anim: &mut Animator,
    on_click: impl Fn() -> M + 'static,
) -> Element<M> {
    let key = key.into();
    let m = anim.spring(
        NodeKey(key.0 ^ PILL_MORPH_SALT),
        f32::from(u8::from(active)),
        Spring::SPATIAL_FAST,
    );
    let h = SIZE.control_m;
    let w = 40.0 + 16.0 * m;
    let fg = Rgba::from_color(theme.scheme.on_surface_variant)
        .lerp(Rgba::from_color(theme.scheme.on_primary), m);
    let bg = theme.veil().lerp(Rgba::from_color(theme.scheme.primary), m);
    let radius = h * 0.5 - (h * 0.5 - RADII.sm) * m;

    box_()
        .id(key)
        .size(w, h)
        .items_center()
        .justify_center()
        .rounded(radius)
        .smoothing(CORNER_SMOOTHING * m)
        .bg(bg)
        .hover_bg(bg.over(fg, STATE_LAYER.hover))
        .press_bg(bg.over(fg, STATE_LAYER.press))
        .on_click(on_click)
        .child(text(label, FontSpec::display(17.0).weight(600), fg))
}

// ── Status pill ──────────────────────────────────────────────────────────────

/// Severity tone of a `.stat` bar status pill ("blue fine, rust off, red
/// urgent" — `BarStates` board). Color swaps animate through `anim`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PillStatus {
    /// Unadorned `veil`/`on_surface` — the quiet resting state on Glass.
    Neutral,
    /// `primary_container`/`on_primary_container` — the default `.stat`.
    Primary,
    /// `tertiary_container`/`on_tertiary_container` — needs attention.
    Attention,
    /// `error_container`/`on_error_container` — urgent.
    Urgent,
}

impl PillStatus {
    fn colors(self, theme: &Theme) -> (Rgba, Rgba) {
        let (bg, fg) = match self {
            Self::Neutral => return (theme.veil(), Rgba::from_color(theme.scheme.on_surface)),
            Self::Primary => (
                theme.scheme.primary_container,
                theme.scheme.on_primary_container,
            ),
            Self::Attention => (
                theme.scheme.tertiary_container,
                theme.scheme.on_tertiary_container,
            ),
            Self::Urgent => (
                theme.scheme.error_container,
                theme.scheme.on_error_container,
            ),
        };
        (Rgba::from_color(bg), Rgba::from_color(fg))
    }
}

/// `.stat` bar status pill: h44 r22, pad `0 18`, gap 16, weight-600 content
/// (icons + clock + battery cluster). Children are arbitrary rows/icons/text;
/// passing `on_click` adds the hover/press state layer and hit target.
pub fn status_pill<M: 'static>(
    key: impl Into<NodeKey>,
    status: PillStatus,
    theme: &Theme,
    anim: &mut Animator,
    on_click: Option<ClickHandler<M>>,
    children: impl IntoIterator<Item = Element<M>>,
) -> Element<M> {
    let key = key.into();
    let (bg_t, fg_t) = status.colors(theme);
    let bg = anim.color(NodeKey(key.0 ^ STAT_BG_SALT), bg_t, Spring::EFFECTS_FAST);
    let fg = anim.color(NodeKey(key.0 ^ STAT_FG_SALT), fg_t, Spring::EFFECTS_FAST);

    let mut el = row()
        .id(key)
        .h(44.0)
        .px(18.0)
        .gap(16.0)
        .items_center()
        .rounded(22.0)
        .smoothing(0.0)
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

/// "N more" overflow pill for collapsed bar/toast content: `more_horiz` icon
/// plus a `{hidden} more` count on a quiet `veil` pill.
pub fn overflow_pill<M: 'static>(
    key: impl Into<NodeKey>,
    hidden: usize,
    theme: &Theme,
    on_click: impl Fn() -> M + 'static,
) -> Element<M> {
    let bg = theme.veil();
    let fg = Rgba::from_color(theme.scheme.on_surface_variant);
    row()
        .id(key)
        .h(SIZE.control_m)
        .px(14.0)
        .gap(6.0)
        .items_center()
        .rounded(SIZE.control_m * 0.5)
        .smoothing(0.0)
        .bg(bg)
        .hover_bg(bg.over(fg, STATE_LAYER.hover))
        .press_bg(bg.over(fg, STATE_LAYER.press))
        .on_click(on_click)
        .child(icon(Icon::MoreHoriz, IconStyle::Outlined, SIZE.icon_s, fg))
        .child(text(
            format!("{hidden} more"),
            FontSpec::ui(13.0).weight(600),
            fg,
        ))
}

// ── Icon disc ────────────────────────────────────────────────────────────────

/// `.t-ic` 40px icon disc: `secondary_container` circle with a 22px
/// `on_secondary_container` glyph — the standard list-row leading icon.
pub fn icon_disc<M>(glyph: Icon, theme: &Theme) -> Element<M> {
    icon_disc_styled(
        glyph,
        SIZE.control_m,
        theme.scheme.secondary_container,
        theme.scheme.on_secondary_container,
    )
}

/// [`icon_disc`] with a custom diameter and colors (e.g. call-toast accept
/// disc, app-icon swatch). The glyph scales with the disc.
pub fn icon_disc_styled<M>(
    glyph: Icon,
    diameter: f32,
    bg: impl Into<Rgba>,
    fg: impl Into<Rgba>,
) -> Element<M> {
    box_()
        .size(diameter, diameter)
        .items_center()
        .justify_center()
        .rounded(diameter * 0.5)
        .smoothing(0.0)
        .bg(bg)
        .child(icon(
            glyph,
            IconStyle::Outlined,
            (diameter * 0.55).clamp(18.0, 28.0),
            fg,
        ))
}

// ── Wave avatar ──────────────────────────────────────────────────────────────

/// 9-lobe Wave identity avatar: `tertiary_container` Wave SDF fill carrying
/// Jost 600 initials — never a circle (Wave = identity only: avatars, album
/// art, launcher, loading).
pub fn wave_avatar<M>(initials: impl Into<String>, diameter: f32, theme: &Theme) -> Element<M> {
    box_()
        .size(diameter, diameter)
        .items_center()
        .justify_center()
        .wave()
        .bg(Rgba::from_color(theme.scheme.tertiary_container))
        .child(text(
            initials,
            FontSpec::display(diameter * 0.375).weight(600),
            Rgba::from_color(theme.scheme.on_tertiary_container),
        ))
}

/// Image variant of [`wave_avatar`] (contact photo, album art): the image is
/// wave-clipped on the GPU (`IMAGE_CLIP_WAVE`), `Cover` fit by default.
pub fn wave_avatar_image<M>(id: ImageId, style: ImageStyle, diameter: f32) -> Element<M> {
    image(id, style).size(diameter, diameter).wave()
}

// ── Badges ───────────────────────────────────────────────────────────────────

/// Unread marker attached to a host element's top-right corner.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum Badge {
    /// 10px `tertiary` dot ringed by 2px of the parent surface color so it
    /// reads through whatever the host paints underneath.
    Dot(Rgba),
    /// `tertiary`/`on_tertiary` count pill; counts above 99 render `99+`.
    Count(u32),
}

/// Standalone badge element for callers that position it themselves. Most
/// hosts want [`badged`], which overlays the top-right corner automatically.
pub fn badge<M>(kind: Badge, theme: &Theme) -> Element<M> {
    let tertiary = Rgba::from_color(theme.scheme.tertiary);
    match kind {
        Badge::Dot(ring) => box_()
            .size(14.0, 14.0)
            .rounded(7.0)
            .smoothing(0.0)
            .bg(tertiary)
            .border(2.0, ring),
        Badge::Count(count) => {
            let label = if count > 99 {
                "99+".to_owned()
            } else {
                count.to_string()
            };
            box_()
                .min_w(20.0)
                .h(20.0)
                .px(5.0)
                .items_center()
                .justify_center()
                .rounded(10.0)
                .smoothing(0.0)
                .bg(tertiary)
                .child(text(
                    label,
                    FontSpec::ui(12.0).weight(700),
                    Rgba::from_color(theme.scheme.on_tertiary),
                ))
        }
    }
}

/// Wrap `host` with a [`Badge`] overhanging its top-right corner (dot: 2px up
/// and right; count pill: 6px up, 2px right — the board's unread placement on
/// the notification bell).
pub fn badged<M>(host: Element<M>, kind: Badge, theme: &Theme) -> Element<M> {
    let margin = match kind {
        Badge::Dot(_) => Edges::new(-2.0, -2.0, 0.0, 0.0),
        Badge::Count(_) => Edges::new(-6.0, -2.0, 0.0, 0.0),
    };
    box_().child(host).child(
        box_()
            .absolute()
            .inset_all(0.0)
            .items_start()
            .justify_end()
            .child(badge(kind, theme).m(margin)),
    )
}

// ── Keycaps & hint bar ───────────────────────────────────────────────────────

/// Visual treatment of a [`keycap`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum KeycapStyle {
    /// `.t-kbd` flat: 24×24 min, r7, JetBrains Mono 12/600, inset 1px border.
    Flat,
    /// `.kc` 3-D settings keycap: 30×30 min, r8, `surface_container_high` with
    /// a 2px darker bottom edge strip.
    Raised,
}

/// Single keyboard keycap (`.t-kbd` / `.kc u`). Text is `on_surface`; the flat
/// variant is transparent with an inset border, the raised variant paints a
/// `surface_container_high` body over a darker 2px bottom strip.
pub fn keycap<M>(label: impl Into<String>, style: KeycapStyle, theme: &Theme) -> Element<M> {
    let on_surface = Rgba::from_color(theme.scheme.on_surface);
    match style {
        KeycapStyle::Flat => box_()
            .min_w(24.0)
            .h(24.0)
            .px(6.0)
            .items_center()
            .justify_center()
            .rounded(7.0)
            .smoothing(CORNER_SMOOTHING)
            .border(1.0, on_surface.with_alpha(0.35))
            .child(text(label, FontSpec::mono(12.0).weight(600), on_surface)),
        KeycapStyle::Raised => col()
            .min_w(30.0)
            .h(30.0)
            .items_stretch()
            .rounded(8.0)
            .smoothing(CORNER_SMOOTHING)
            .bg(Rgba::from_color(theme.scheme.surface_container_high))
            .child(
                box_()
                    .flex_1()
                    .px(9.0)
                    .items_center()
                    .justify_center()
                    .child(text(label, FontSpec::mono(13.0).weight(500), on_surface)),
            )
            .child(
                box_()
                    .h(2.0)
                    .rounded([0.0, 0.0, 8.0, 8.0])
                    .smoothing(CORNER_SMOOTHING)
                    .bg(Rgba::new(0.0, 0.0, 0.0, 0.35)),
            ),
    }
}

/// Keyboard-shortcut footer row (`.kb` grammar): each hint is a flat
/// [`keycap`] followed by a 13px `on_surface_variant` action label, hints
/// spaced 16px apart — the Launcher/Power/Switcher footer pattern.
pub fn hint_bar<M, K, V>(hints: impl IntoIterator<Item = (K, V)>, theme: &Theme) -> Element<M>
where
    K: Into<String>,
    V: Into<String>,
{
    let onv = Rgba::from_color(theme.scheme.on_surface_variant);
    row()
        .gap(16.0)
        .items_center()
        .children(hints.into_iter().map(|(keys, action)| {
            row()
                .gap(8.0)
                .items_center()
                .child(keycap(keys, KeycapStyle::Flat, theme))
                .child(text(action, FontSpec::ui(13.0), onv))
        }))
}
