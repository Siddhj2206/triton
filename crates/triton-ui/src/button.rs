//! Everything you press: buttons, icon buttons, chips, nav items, connected groups.
//!
//! A button without `on_press` is disabled. State layers follow the tokens: hover
//! 8%, press 10% of the content color over the fill. Pressing tightens a button's
//! corners to 12 px (`.t-btn:active`) and selection squares an icon button; the
//! [`Pressable`] widget springs between these shapes and fades the layers.

use iced_core::alignment::Vertical;
use iced_core::font::Weight;
use iced_core::{Color, Length, Padding};
use iced_widget::Row;
use iced_widget::text::IntoFragment;
use triton_design::color::{STATE_LAYER, over, with_alpha};
use triton_design::material::{veil, veil_hi};
use triton_design::shape::connected_corners;
use triton_design::{Icon, IconStyle, RADII, SIZE};

use crate::paint::Paint;
use crate::text::{icon, ui};
use crate::{Element, Renderer, Theme};

/// `.t-btn` variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Kind {
    #[default]
    Filled,
    Tonal,
    Outlined,
    Text,
    /// On Glass or a scrim.
    Veil,
}

/// `.t-ibtn` variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IconKind {
    #[default]
    Standard,
    Filled,
    Tonal,
    Outlined,
    /// An unselected toggle: neutral fill.
    Neutral,
    /// On Glass or a scrim.
    Veil,
}

/// Control heights: 32, 40, 48, 56.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Size {
    S,
    #[default]
    M,
    L,
    Xl,
}

impl Size {
    pub const fn height(self) -> f32 {
        match self {
            Size::S => SIZE.control_s,
            Size::M => SIZE.control_m,
            Size::L => SIZE.control_l,
            Size::Xl => SIZE.control_xl,
        }
    }

    /// `.t-btn` horizontal padding and label size per height.
    const fn button_metrics(self) -> (f32, f32) {
        match self {
            Size::S => (16.0, 13.0),
            Size::M => (24.0, 14.0),
            Size::L => (28.0, 16.0),
            Size::Xl => (32.0, 17.0),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonClass {
    Button {
        kind: Kind,
        size: Size,
    },
    Icon {
        kind: IconKind,
        size: Size,
        toggled: bool,
    },
    Chip {
        selected: bool,
    },
    Nav {
        selected: bool,
    },
    Segment {
        index: usize,
        count: usize,
        selected: bool,
        large: bool,
        veil: bool,
    },
    /// The switch track (`.t-switch`).
    Switch {
        on: bool,
    },
    /// A checkbox box.
    Check {
        checked: bool,
    },
    /// A radio ring.
    Radio {
        selected: bool,
    },
    /// A bar workspace (`.ws`): 40 high, Jost 17; the active one is a 56 px primary
    /// pill.
    Workspace {
        active: bool,
    },
    /// A 44 px veil disc in the bar (`.bell`).
    BarDisc,
    /// No fill at rest, a round state layer on hover (the wave launcher).
    Bare,
    /// A menu row (`.mi`): pill, primary wash when hovered or its submenu is open.
    MenuItem {
        destructive: bool,
        open: bool,
    },
}

/// The resting colors before interaction is applied.
struct Look {
    fill: Option<Color>,
    content: Color,
    outline: Option<Color>,
    outline_width: f32,
    /// Hover and press wash this color (at `MENU_WASH`) instead of a state layer.
    wash: Option<Color>,
}

impl Look {
    fn new(fill: Option<Color>, content: Color, outline: Option<Color>) -> Look {
        Look {
            fill,
            content,
            outline,
            outline_width: 1.0,
            wash: None,
        }
    }
}

impl ButtonClass {
    /// Corner radii (top-left, top-right, bottom-right, bottom-left), at rest or
    /// pressed. Independent of the scheme, so the widget can animate them.
    pub(crate) fn radii(self, pressed: bool) -> [f32; 4] {
        match self {
            // .t-btn: a pill that tightens to 12 while pressed.
            ButtonClass::Button { size, .. } => {
                [if pressed {
                    RADII.sm
                } else {
                    size.height() / 2.0
                }; 4]
            }
            // .t-ibtn: round; selection squares the corners (12 at 40 px, scaled).
            ButtonClass::Icon { size, toggled, .. } => {
                let h = size.height();
                [if toggled {
                    RADII.sm * h / SIZE.control_m
                } else {
                    h / 2.0
                }; 4]
            }
            ButtonClass::Chip { .. } => [10.0; 4],
            ButtonClass::Nav { .. } | ButtonClass::MenuItem { .. } => [SIZE.control_l / 2.0; 4],
            ButtonClass::Segment {
                index,
                count,
                large,
                ..
            } => connected_corners(index, count, segment_end(large)),
            ButtonClass::Switch { .. } => [SIZE.switch.1 / 2.0; 4],
            ButtonClass::Workspace { .. } => [SIZE.control_m / 2.0; 4],
            ButtonClass::BarDisc | ButtonClass::Bare => [BAR_ITEM / 2.0; 4],
            ButtonClass::Check { .. } => [6.0; 4],
            ButtonClass::Radio { .. } => [12.0; 4],
        }
    }
}

impl Theme {
    fn look(&self, class: ButtonClass) -> Look {
        let s = &self.scheme;
        match class {
            ButtonClass::Button { kind, .. } => {
                let (fill, content, outline) = match kind {
                    Kind::Filled => (Some(s.primary), s.on_primary, None),
                    Kind::Tonal => (Some(s.secondary_container), s.on_secondary_container, None),
                    Kind::Outlined => (None, s.primary, Some(s.outline)),
                    Kind::Text => (None, s.primary, None),
                    Kind::Veil => (Some(veil(self.variant)), s.on_surface, None),
                };
                Look::new(fill, content, outline)
            }
            ButtonClass::Icon { kind, .. } => {
                let (fill, content, outline) = match kind {
                    IconKind::Standard => (None, s.on_surface_variant, None),
                    IconKind::Filled => (Some(s.primary), s.on_primary, None),
                    IconKind::Tonal => {
                        (Some(s.secondary_container), s.on_secondary_container, None)
                    }
                    IconKind::Outlined => (None, s.on_surface_variant, Some(s.outline)),
                    IconKind::Neutral => (
                        Some(s.surface_container_highest),
                        s.on_surface_variant,
                        None,
                    ),
                    IconKind::Veil => (Some(veil(self.variant)), s.on_surface, None),
                };
                Look::new(fill, content, outline)
            }
            // .t-chip: 1 px outline unless selected.
            ButtonClass::Chip { selected: true } => {
                Look::new(Some(s.secondary_container), s.on_secondary_container, None)
            }
            ButtonClass::Chip { selected: false } => {
                Look::new(None, s.on_surface_variant, Some(s.outline))
            }
            // .t-nav: primary container when selected.
            ButtonClass::Nav { selected } => Look::new(
                selected.then_some(s.primary_container),
                if selected {
                    s.on_primary_container
                } else {
                    s.on_surface
                },
                None,
            ),
            ButtonClass::Segment {
                selected,
                veil: on_glass,
                ..
            } => {
                let (fill, content) = if selected {
                    (s.primary, s.on_primary)
                } else if on_glass {
                    (veil(self.variant), s.on_surface)
                } else {
                    (s.surface_container_high, s.on_surface)
                };
                Look::new(Some(fill), content, None)
            }
            ButtonClass::Workspace { active: true } => {
                Look::new(Some(s.primary), s.on_primary, None)
            }
            ButtonClass::Workspace { active: false } => Look::new(None, s.on_surface_variant, None),
            ButtonClass::BarDisc => Look::new(Some(veil(self.variant)), s.on_surface, None),
            // The switch draws its own track colors (see `switch_colors`).
            ButtonClass::Bare | ButtonClass::Switch { .. } => Look::new(None, s.on_surface, None),
            ButtonClass::Check { checked: true } => Look::new(Some(s.primary), s.on_primary, None),
            ButtonClass::Check { checked: false } => Look {
                outline_width: 2.0,
                ..Look::new(None, s.on_surface_variant, Some(s.outline))
            },
            // The dot is the content.
            ButtonClass::Radio { selected } => Look {
                outline_width: 2.0,
                ..Look::new(
                    None,
                    s.primary,
                    Some(if selected { s.primary } else { s.outline }),
                )
            },
            // .mi: an open submenu keeps the wash.
            ButtonClass::MenuItem { destructive, open } => Look {
                wash: Some(s.primary),
                ..Look::new(
                    open.then_some(with_alpha(s.primary, MENU_WASH)),
                    if destructive { s.error } else { s.on_surface },
                    None,
                )
            },
        }
    }

    /// The switch track at `on` (0 off to 1 on): fill, outline, thumb, check.
    pub(crate) fn switch_colors(&self, on: f32) -> [Color; 4] {
        let s = &self.scheme;
        [
            mix(s.surface_container_highest, s.primary, on),
            mix(s.outline, s.primary, on),
            mix(s.outline, s.on_primary, on),
            with_alpha(s.primary, on),
        ]
    }

    /// The paint and content color of a pressable, given how hovered and pressed it
    /// is (0 to 1, animated) and its current corner radii.
    pub(crate) fn press_paint(
        &self,
        class: ButtonClass,
        hover: f32,
        press: f32,
        disabled: bool,
        radii: [f32; 4],
    ) -> (Paint, Color) {
        let s = &self.scheme;
        let look = self.look(class);
        let layer = STATE_LAYER.hover * hover + (STATE_LAYER.press - STATE_LAYER.hover) * press;
        let fill = match (look.fill, look.wash) {
            // A washed control (menu rows) fades its wash in, a step stronger pressed.
            (base, Some(wash)) => {
                let rest = if base.is_some() { MENU_WASH } else { 0.0 };
                let alpha = rest.max(MENU_WASH * hover.max(press)) + PRESS_WASH * press;
                (alpha > 0.0).then(|| with_alpha(wash, alpha))
            }
            // A veil brightens toward veil-hi instead of taking a state layer.
            (Some(f), None) if f == veil(self.variant) => {
                Some(mix(f, veil_hi(self.variant), hover.max(press)))
            }
            (Some(f), None) => Some(over(f, look.content, layer)),
            (None, None) => (layer > 0.0).then(|| with_alpha(look.content, layer)),
        };
        let mut paint = Paint {
            fill,
            border: look.outline.map(|c| (c, look.outline_width)),
            radii,
            // In-shell controls ease their corners; nothing here floats.
            smooth: true,
            shadow: None,
            highlight: false,
        };
        let mut content = look.content;
        if disabled {
            content = with_alpha(s.on_surface, STATE_LAYER.disabled_content);
            let dim = with_alpha(s.on_surface, STATE_LAYER.disabled_container);
            paint.fill = look.fill.map(|_| dim);
            paint.border = look.outline.map(|_| (dim, look.outline_width));
        }
        (paint, content)
    }
}

/// Linear blend from `a` to `b`.
pub(crate) fn mix(a: Color, b: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    Color::from_rgba(
        a.r + (b.r - a.r) * t,
        a.g + (b.g - a.g) * t,
        a.b + (b.b - a.b) * t,
        a.a + (b.a - a.a) * t,
    )
}

/// Bar items that end an island: 44 px, so with the island's 6 px inset their
/// corners are concentric with its 28 px ends.
pub(crate) const BAR_ITEM: f32 = 44.0;

/// `.mi.hov`: the menu highlight is primary at 16%; pressing adds this much.
const MENU_WASH: f32 = 0.16;
const PRESS_WASH: f32 = 0.08;

/// Content centered vertically in the control's height (iced buttons top-align).
fn centered<'a, M: 'a>(
    content: Row<'a, M, Theme, Renderer>,
) -> iced_widget::Container<'a, M, Theme, Renderer> {
    iced_widget::container(content).center_y(Length::Fill)
}

pub use crate::press::Pressable;
use crate::press::pressable;

/// `.t-btn`: a label, optionally led by an icon.
pub fn button<'a, M: 'a>(
    label: impl IntoFragment<'a>,
    leading: Option<Icon>,
    kind: Kind,
    size: Size,
) -> Pressable<'a, M> {
    let (pad, font) = size.button_metrics();
    let pad = if kind == Kind::Text { 14.0 } else { pad };
    let icon_size = if matches!(size, Size::L | Size::Xl) {
        20.0
    } else {
        18.0
    };
    let mut content = Row::new().spacing(8).align_y(Vertical::Center);
    if let Some(glyph) = leading {
        content = content.push(icon(glyph, IconStyle::Outlined, icon_size));
    }
    content = content.push(ui(label, font, Weight::Semibold));
    pressable(centered(content))
        .height(size.height())
        .padding(Padding::from([0.0, pad]))
        .class(ButtonClass::Button { kind, size })
}

/// `.t-ibtn`: a square icon button. `toggled` squares the corners and fills the
/// icon.
pub fn icon_button<'a, M: 'a>(
    glyph: Icon,
    kind: IconKind,
    size: Size,
    toggled: bool,
) -> Pressable<'a, M> {
    let h = size.height();
    let style = if toggled {
        IconStyle::Filled
    } else {
        IconStyle::Outlined
    };
    let glyph_size = if size == Size::S { 20.0 } else { 24.0 };
    pressable(
        iced_widget::container(icon(glyph, style, glyph_size))
            .center_x(Length::Fill)
            .center_y(Length::Fill),
    )
    .width(h)
    .height(h)
    .padding(0)
    .class(ButtonClass::Icon {
        kind,
        size,
        toggled,
    })
}

/// `.t-chip`: a filter chip, 36 high.
pub fn chip<'a, M: 'a>(label: &'a str, leading: Option<Icon>, selected: bool) -> Pressable<'a, M> {
    let mut content = Row::new().spacing(8).align_y(Vertical::Center);
    if let Some(glyph) = leading {
        let style = if selected {
            IconStyle::Filled
        } else {
            IconStyle::Outlined
        };
        content = content.push(icon(glyph, style, 18.0));
    }
    content = content.push(ui(label, 14.0, Weight::Medium));
    pressable(centered(content))
        .height(36)
        .padding(Padding::from([0.0, 14.0]))
        .class(ButtonClass::Chip { selected })
}

/// `.t-nav`: a sidebar item, full width.
pub fn nav_item<'a, M: 'a>(glyph: Icon, label: &'a str, selected: bool) -> Pressable<'a, M> {
    let style = if selected {
        IconStyle::Filled
    } else {
        IconStyle::Outlined
    };
    let weight = if selected {
        Weight::Semibold
    } else {
        Weight::Medium
    };
    let mut glyph = icon(glyph, style, 22.0);
    if !selected {
        glyph = glyph.class(crate::Tone::OnSurfaceVariant);
    }
    let content = Row::new()
        .push(glyph)
        .push(ui(label, 15.0, weight))
        .spacing(16)
        .align_y(Vertical::Center);
    pressable(centered(content))
        .width(Length::Fill)
        .height(SIZE.control_l)
        .padding(Padding::from([0.0, 16.0]))
        .class(ButtonClass::Nav { selected })
}

/// One segment of a connected group.
#[derive(Debug, Clone)]
pub struct Segment<'a, M> {
    pub icon: Option<Icon>,
    pub label: Option<&'a str>,
    pub selected: bool,
    pub on_press: Option<M>,
}

/// The outer corner radius of a segmented group's end segments: 22, or 24 for the
/// large variant. Wrap a group with this to get concentric corners.
pub fn segment_end(large: bool) -> f32 {
    if large {
        SIZE.control_l / 2.0
    } else {
        RADII.connected_end
    }
}

/// `.t-seg`: 2 to 4 segments joined by 3 px gaps. `large` is the 48 px toolbar
/// variant; `veil` puts unselected segments on Glass.
pub fn segmented<'a, M: Clone + 'a>(
    segments: Vec<Segment<'a, M>>,
    large: bool,
    veil: bool,
) -> Row<'a, M, Theme, Renderer> {
    let count = segments.len();
    let height = if large { SIZE.control_l } else { 44.0 };
    let font = if large { 15.0 } else { 14.0 };
    let children = segments
        .into_iter()
        .enumerate()
        .map(|(index, seg)| -> Element<'a, M> {
            let mut content = Row::new().spacing(8).align_y(Vertical::Center);
            if let Some(glyph) = seg.icon {
                let style = if seg.selected {
                    IconStyle::Filled
                } else {
                    IconStyle::Outlined
                };
                content = content.push(icon(glyph, style, 20.0));
            }
            if let Some(label) = seg.label {
                content = content.push(ui(label, font, Weight::Semibold));
            }
            pressable(
                iced_widget::container(content)
                    .center_x(Length::Fill)
                    .center_y(Length::Fill),
            )
            .width(Length::Fill)
            .height(height)
            .padding(Padding::from([0.0, 20.0]))
            .on_press_maybe(seg.on_press)
            .class(ButtonClass::Segment {
                index,
                count,
                selected: seg.selected,
                large,
                veil,
            })
            .into()
        });
    Row::with_children(children).spacing(3)
}
