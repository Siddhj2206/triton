//! The bar's components (`BarStates` on the canvas): the wave launcher, workspace
//! pills, the focused window, the clock, the status group and the bell. The shell
//! composes them into two Glass [`island`](crate::surface::island)s.

use iced_core::alignment::Vertical;
use iced_core::font::{Family as FontFamily, Weight};
use iced_core::text::LineHeight;
use iced_core::{Font, Length, Padding};
use iced_widget::{Row, Space, stack};
use triton_design::{Family, Icon, IconStyle};

use crate::button::{BAR_ITEM, ButtonClass, Pressable};
use crate::press::pressable;
use crate::progress::{WaveTone, wave_shape};
use crate::surface::{Panel, surface};
use crate::text::{icon, ui};
use crate::{Element, Surface, Tone};

/// Space between items on an island.
pub const GAP: f32 = 4.0;

fn display<'a>(
    content: impl iced_widget::text::IntoFragment<'a>,
    size: f32,
) -> crate::text::Text<'a> {
    iced_widget::text(content)
        .font(Font {
            family: FontFamily::Name(Family::Display.name()),
            weight: Weight::Semibold,
            ..Font::DEFAULT
        })
        .size(size)
        .line_height(LineHeight::Relative(1.0))
}

/// `.wv`: the 44 px wave with the apps glyph, opening the launcher.
pub fn launcher<'a, M: Clone + 'a>() -> Pressable<'a, M> {
    let glyph =
        iced_widget::container(icon(Icon::Apps, IconStyle::Outlined, 22.0).class(Tone::OnPrimary))
            .center(BAR_ITEM);
    pressable(stack![wave_shape(BAR_ITEM, WaveTone::Primary), glyph])
        .width(BAR_ITEM)
        .height(BAR_ITEM)
        .class(ButtonClass::Bare)
}

/// `.ws`: a workspace number; the active one is a wider primary pill.
pub fn workspace<'a, M: Clone + 'a>(label: String, active: bool) -> Pressable<'a, M> {
    let width = if active { 56.0 } else { 40.0 };
    pressable(iced_widget::container(display(label, 17.0)).center(Length::Fill))
        .width(width)
        .height(40)
        .class(ButtonClass::Workspace { active })
}

/// The focused window: app glyph and title, clipped so it never pushes the clock.
pub fn focused<'a, M: 'a>(title: &'a str) -> Element<'a, M> {
    let row = Row::new()
        .push(icon(Icon::WebAsset, IconStyle::Outlined, 20.0).class(Tone::OnSurfaceVariant))
        .push(
            iced_widget::container(
                ui(title, 14.0, Weight::Semibold).wrapping(iced_core::text::Wrapping::None),
            )
            .max_width(360)
            .clip(true),
        )
        .spacing(8)
        .align_y(Vertical::Center);
    iced_widget::container(row)
        .padding(Padding::from([0.0, 12.0]))
        .into()
}

/// `.clk`: time in Jost 26, date in 14 medium.
pub fn clock<'a, M: 'a>(time: String, date: String) -> Element<'a, M> {
    let row = Row::new()
        .push(display(time, 26.0))
        .push(ui(date, 14.0, Weight::Medium).class(Tone::OnSurfaceVariant))
        .spacing(12)
        .align_y(Vertical::Center);
    iced_widget::container(row)
        .padding(Padding {
            left: 18.0,
            right: 16.0,
            ..Padding::ZERO
        })
        .into()
}

/// One entry in the status group.
#[derive(Debug, Clone)]
pub struct Status {
    pub icon: Icon,
    pub label: Option<String>,
}

/// `.stat`: a 44 px primary-container pill of status icons. `None` if empty.
pub fn status<'a, M: 'a>(items: Vec<Status>) -> Option<Panel<'a, M>> {
    if items.is_empty() {
        return None;
    }
    let row = Row::with_children(items.into_iter().map(|item| -> Element<'a, M> {
        let mut r = Row::new()
            .push(icon(item.icon, IconStyle::Outlined, 24.0))
            .spacing(6)
            .align_y(Vertical::Center);
        if let Some(label) = item.label {
            r = r.push(ui(label, 15.0, Weight::Semibold));
        }
        r.into()
    }))
    .spacing(16)
    .align_y(Vertical::Center);
    Some(
        surface(row, Surface::Status)
            .height(BAR_ITEM)
            .center_y(BAR_ITEM)
            .padding(Padding::from([0.0, 18.0])),
    )
}

/// `.bell`: the notifications disc, with a tertiary dot when something is unread.
pub fn bell<'a, M: Clone + 'a>(unread: bool, dnd: bool) -> Pressable<'a, M> {
    let glyph = if dnd {
        Icon::NotificationsOff
    } else {
        Icon::Notifications
    };
    let mut layers =
        stack![iced_widget::container(icon(glyph, IconStyle::Outlined, 24.0)).center(BAR_ITEM)];
    if unread {
        layers = layers.push(
            iced_widget::container(surface(Space::new(), Surface::Dot).width(10).height(10))
                .padding(Padding {
                    top: 9.0,
                    left: BAR_ITEM - 10.0 - 10.0,
                    ..Padding::ZERO
                }),
        );
    }
    pressable(layers)
        .width(BAR_ITEM)
        .height(BAR_ITEM)
        .class(ButtonClass::BarDisc)
}
