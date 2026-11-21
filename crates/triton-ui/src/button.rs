//! Everything you press: buttons, icon buttons, chips, nav items, connected groups.
//!
//! A button without `on_press` is disabled. State layers follow the tokens: hover
//! 8%, press 10% of the content color over the fill. Pressing tightens a button's
//! corners to 12 px (`.t-btn:active`); the animated morph comes with the shape
//! widget, this is the settled state.

use iced_core::alignment::Vertical;
use iced_core::border::Radius;
use iced_core::font::Weight;
use iced_core::{Background, Border, Color, Length, Padding, Shadow};
use iced_widget::button::{Catalog, Status, Style};
use iced_widget::{Button, Row};
use triton_design::color::{STATE_LAYER, over, with_alpha};
use triton_design::material::{veil, veil_hi};
use triton_design::shape::connected_corners;
use triton_design::{Icon, IconStyle, RADII, SIZE};

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
}

/// The resting look before status is applied.
struct Look {
    fill: Option<Color>,
    content: Color,
    outline: Option<Color>,
    radius: Radius,
    /// Radius while pressed, if pressing changes the shape.
    pressed_radius: Option<f32>,
}

impl Look {
    fn new(
        fill: Option<Color>,
        content: Color,
        outline: Option<Color>,
        radius: impl Into<Radius>,
    ) -> Look {
        Look {
            fill,
            content,
            outline,
            radius: radius.into(),
            pressed_radius: None,
        }
    }
}

impl Theme {
    fn look(&self, class: ButtonClass) -> Look {
        match class {
            ButtonClass::Button { kind, size } => self.button_look(kind, size),
            ButtonClass::Icon {
                kind,
                size,
                toggled,
            } => self.icon_look(kind, size, toggled),
            ButtonClass::Chip { selected } => self.chip_look(selected),
            ButtonClass::Nav { selected } => self.nav_look(selected),
            ButtonClass::Segment {
                index,
                count,
                selected,
                large,
                veil,
            } => self.segment_look(index, count, selected, large, veil),
        }
    }

    /// `.t-btn`: a pill whose corners tighten to 12 while pressed.
    fn button_look(&self, kind: Kind, size: Size) -> Look {
        let s = &self.scheme;
        let (fill, content, outline) = match kind {
            Kind::Filled => (Some(s.primary), s.on_primary, None),
            Kind::Tonal => (Some(s.secondary_container), s.on_secondary_container, None),
            Kind::Outlined => (None, s.primary, Some(s.outline)),
            Kind::Text => (None, s.primary, None),
            Kind::Veil => (Some(veil(self.variant)), s.on_surface, None),
        };
        Look {
            pressed_radius: Some(RADII.sm),
            ..Look::new(fill, content, outline, size.height() / 2.0)
        }
    }

    /// `.t-ibtn`: round; selection squares the corners (12 at 40 px, scaled).
    fn icon_look(&self, kind: IconKind, size: Size, toggled: bool) -> Look {
        let s = &self.scheme;
        let (fill, content, outline) = match kind {
            IconKind::Standard => (None, s.on_surface_variant, None),
            IconKind::Filled => (Some(s.primary), s.on_primary, None),
            IconKind::Tonal => (Some(s.secondary_container), s.on_secondary_container, None),
            IconKind::Outlined => (None, s.on_surface_variant, Some(s.outline)),
            IconKind::Neutral => (
                Some(s.surface_container_highest),
                s.on_surface_variant,
                None,
            ),
            IconKind::Veil => (Some(veil(self.variant)), s.on_surface, None),
        };
        let h = size.height();
        let radius = if toggled {
            RADII.sm * h / SIZE.control_m
        } else {
            h / 2.0
        };
        Look::new(fill, content, outline, radius)
    }

    /// `.t-chip`: radius 10, 1 px outline unless selected.
    fn chip_look(&self, selected: bool) -> Look {
        let s = &self.scheme;
        if selected {
            Look::new(
                Some(s.secondary_container),
                s.on_secondary_container,
                None,
                10.0,
            )
        } else {
            Look::new(None, s.on_surface_variant, Some(s.outline), 10.0)
        }
    }

    /// `.t-nav`: a 48 px pill, primary container when selected.
    fn nav_look(&self, selected: bool) -> Look {
        let s = &self.scheme;
        let content = if selected {
            s.on_primary_container
        } else {
            s.on_surface
        };
        Look::new(
            selected.then_some(s.primary_container),
            content,
            None,
            SIZE.control_l / 2.0,
        )
    }

    /// `.t-seg`: ends 22 (24 large), joins 8.
    fn segment_look(
        &self,
        index: usize,
        count: usize,
        selected: bool,
        large: bool,
        on_glass: bool,
    ) -> Look {
        let s = &self.scheme;
        let end = segment_end(large);
        let [top_left, top_right, bottom_right, bottom_left] = connected_corners(index, count, end);
        let (fill, content) = if selected {
            (s.primary, s.on_primary)
        } else if on_glass {
            (veil(self.variant), s.on_surface)
        } else {
            (s.surface_container_high, s.on_surface)
        };
        let radius = Radius {
            top_left,
            top_right,
            bottom_right,
            bottom_left,
        };
        Look::new(Some(fill), content, None, radius)
    }
}

impl Catalog for Theme {
    type Class<'a> = ButtonClass;

    fn default<'a>() -> Self::Class<'a> {
        ButtonClass::Button {
            kind: Kind::Filled,
            size: Size::M,
        }
    }

    fn style(&self, class: &ButtonClass, status: Status) -> Style {
        let s = &self.scheme;
        let look = self.look(*class);
        let layer = match status {
            Status::Hovered => STATE_LAYER.hover,
            Status::Pressed => STATE_LAYER.press,
            Status::Active | Status::Disabled => 0.0,
        };
        let fill = match look.fill {
            // A veil brightens to veil-hi instead of taking a state layer.
            Some(f) if layer > 0.0 && f == veil(self.variant) => Some(veil_hi(self.variant)),
            Some(f) => Some(over(f, look.content, layer)),
            None if layer > 0.0 => Some(with_alpha(look.content, layer)),
            None => None,
        };
        let radius = match (status, look.pressed_radius) {
            (Status::Pressed, Some(r)) => r.into(),
            _ => look.radius,
        };
        let mut style = Style {
            background: fill.map(Background::Color),
            text_color: look.content,
            border: Border {
                color: look.outline.unwrap_or(Color::TRANSPARENT),
                width: if look.outline.is_some() { 1.0 } else { 0.0 },
                radius,
            },
            shadow: Shadow::default(),
            snap: true,
        };
        if status == Status::Disabled {
            let content = with_alpha(s.on_surface, STATE_LAYER.disabled_content);
            style.text_color = content;
            if look.fill.is_some() {
                style.background = Some(Background::Color(with_alpha(
                    s.on_surface,
                    STATE_LAYER.disabled_container,
                )));
            }
            if look.outline.is_some() {
                style.border.color = with_alpha(s.on_surface, STATE_LAYER.disabled_container);
            }
        }
        style
    }
}

/// Content centered vertically in the control's height (iced buttons top-align).
fn centered<'a, M: 'a>(
    content: Row<'a, M, Theme, Renderer>,
) -> iced_widget::Container<'a, M, Theme, Renderer> {
    iced_widget::container(content).center_y(Length::Fill)
}

pub type Pressable<'a, M> = Button<'a, M, Theme, Renderer>;

/// `.t-btn`: a label, optionally led by an icon.
pub fn button<'a, M: 'a>(
    label: &'a str,
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
    iced_widget::button(centered(content))
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
    iced_widget::button(
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
    iced_widget::button(centered(content))
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
    iced_widget::button(centered(content))
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
            iced_widget::button(
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
