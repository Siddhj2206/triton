//! Every component in one view, as on the canvas's Component index and Controls
//! boards. The snapshot test renders it; the preview can show it too.

use iced_core::alignment::Vertical;
use iced_core::{Length, Padding};
use iced_widget::{Column, Row, Space, column, row};
use triton_design::{Icon, Material, TypeStyle};

use crate::button::{button, chip, icon_button, nav_item, segment_end, segmented};
use crate::surface::{ListItem, card, icon_disc, island, kbd, list, pill, surface, tag, wrap};
use crate::text::{eyebrow, icon, text};
use crate::{Element, IconKind, Kind, Segment, Size, Surface, Tone};

fn section<'a, M: 'a>(
    title: &'a str,
    body: impl Into<Element<'a, M>>,
) -> Column<'a, M, crate::Theme, crate::Renderer> {
    column![eyebrow(title), body.into()].spacing(12)
}

fn buttons<'a, M: Clone + 'a>(press: &M) -> Element<'a, M> {
    let p = || Some(press.clone());
    let kinds = row![
        button("Filled", None, Kind::Filled, Size::M).on_press_maybe(p()),
        button("Tonal", None, Kind::Tonal, Size::M).on_press_maybe(p()),
        button("Outlined", None, Kind::Outlined, Size::M).on_press_maybe(p()),
        button("Text", None, Kind::Text, Size::M).on_press_maybe(p()),
        button("Open", Some(Icon::OpenInNew), Kind::Tonal, Size::M).on_press_maybe(p()),
        button("Disabled", None, Kind::Filled, Size::M),
    ]
    .spacing(12)
    .align_y(Vertical::Center);
    let sizes = row![
        button("Small", None, Kind::Filled, Size::S).on_press_maybe(p()),
        button("Medium", None, Kind::Filled, Size::M).on_press_maybe(p()),
        button("Large", None, Kind::Filled, Size::L).on_press_maybe(p()),
        button("Extra large", None, Kind::Filled, Size::Xl).on_press_maybe(p()),
    ]
    .spacing(12)
    .align_y(Vertical::Center);
    column![kinds, sizes].spacing(16).into()
}

fn icon_buttons<'a, M: Clone + 'a>(press: &M) -> Element<'a, M> {
    let p = || Some(press.clone());
    row![
        icon_button(Icon::Settings, IconKind::Standard, Size::M, false).on_press_maybe(p()),
        icon_button(Icon::Add, IconKind::Filled, Size::M, false).on_press_maybe(p()),
        icon_button(Icon::Edit, IconKind::Tonal, Size::M, false).on_press_maybe(p()),
        icon_button(Icon::Delete, IconKind::Outlined, Size::M, false).on_press_maybe(p()),
        icon_button(Icon::Wifi, IconKind::Neutral, Size::M, false).on_press_maybe(p()),
        icon_button(Icon::Wifi, IconKind::Filled, Size::M, true).on_press_maybe(p()),
        icon_button(Icon::Bluetooth, IconKind::Filled, Size::L, true).on_press_maybe(p()),
        icon_button(Icon::DarkMode, IconKind::Neutral, Size::L, false).on_press_maybe(p()),
        icon_button(Icon::Close, IconKind::Standard, Size::S, false).on_press_maybe(p()),
    ]
    .spacing(12)
    .align_y(Vertical::Center)
    .into()
}

fn selection<'a, M: Clone + 'a>(press: &M) -> Element<'a, M> {
    let p = || Some(press.clone());
    let chips = row![
        chip("All", None, true).on_press_maybe(p()),
        chip("Apps", Some(Icon::Apps), false).on_press_maybe(p()),
        chip("Files", Some(Icon::Folder), false).on_press_maybe(p()),
        chip("Windows", Some(Icon::WebAsset), false).on_press_maybe(p()),
    ]
    .spacing(8);
    let seg = |icon, label, selected| Segment {
        icon: Some(icon),
        label: Some(label),
        selected,
        on_press: p(),
    };
    let modes = segmented(
        vec![
            seg(Icon::DarkMode, "Dark", true),
            seg(Icon::LightMode, "Light", false),
            seg(Icon::BrightnessAuto, "Auto", false),
        ],
        false,
        false,
    )
    .width(420);
    let toolbar = wrap(
        segmented(
            vec![
                seg(Icon::ScreenshotRegion, "Region", true),
                seg(Icon::SelectWindow, "Window", false),
                seg(Icon::DesktopWindows, "Screen", false),
            ],
            true,
            true,
        )
        .width(440),
        Material::Glass,
        segment_end(true),
        8.0,
    );
    column![chips, modes, toolbar].spacing(16).into()
}

fn navigation<'a, M: Clone + 'a>(press: &M) -> Element<'a, M> {
    let p = || Some(press.clone());
    let sidebar = surface(
        column![
            nav_item(Icon::Palette, "Appearance", true).on_press_maybe(p()),
            nav_item(Icon::Toolbar, "Bar & Shell", false).on_press_maybe(p()),
            nav_item(Icon::WebAsset, "Windows", false).on_press_maybe(p()),
            nav_item(Icon::Notifications, "Notifications", false).on_press_maybe(p()),
        ]
        .spacing(2),
        Surface::Material(Material::Mica),
    )
    .padding(12)
    .width(280);

    let rows = list(
        vec![
            ListItem {
                leading: Some(icon_disc(Icon::Wifi).into()),
                title: "Wi-Fi",
                supporting: Some("Home 5G"),
                trailing: Some(
                    icon(Icon::ChevronRight, triton_design::IconStyle::Outlined, 24.0).into(),
                ),
                selected: false,
            },
            ListItem {
                leading: Some(icon_disc(Icon::Bluetooth).into()),
                title: "Bluetooth",
                supporting: Some("WH-1000XM5"),
                trailing: Some(tag("2 devices").into()),
                selected: true,
            },
            ListItem {
                leading: Some(icon_disc(Icon::Bedtime).into()),
                title: "Night light",
                supporting: Some("Until 06:30"),
                trailing: None,
                selected: false,
            },
        ],
        false,
    )
    .width(Length::Fill);
    row![sidebar, rows].spacing(24).into()
}

fn chrome<'a, M: 'a>() -> Element<'a, M> {
    let workspaces = row![
        icon(Icon::Apps, triton_design::IconStyle::Filled, 24.0),
        text("1", TypeStyle::TITLE_M),
        text("2", TypeStyle::TITLE_M).class(Tone::OnSurfaceVariant),
        text("3", TypeStyle::TITLE_M).class(Tone::OnSurfaceVariant),
    ]
    .spacing(20)
    .align_y(Vertical::Center);
    let status = row![
        text("21:41", TypeStyle::TITLE_L),
        text("Sat 21 Nov", TypeStyle::BODY_M).class(Tone::OnSurfaceVariant),
        icon(Icon::Wifi, triton_design::IconStyle::Outlined, 20.0),
        icon(Icon::VolumeUp, triton_design::IconStyle::Outlined, 20.0),
        icon(Icon::Battery5Bar, triton_design::IconStyle::Outlined, 20.0),
        text("82%", TypeStyle::LABEL_L),
    ]
    .spacing(12)
    .align_y(Vertical::Center);
    let keys = row![
        kbd("Super"),
        kbd("Space"),
        text("open launcher", TypeStyle::BODY_S).class(Tone::OnSurfaceVariant)
    ]
    .spacing(6)
    .align_y(Vertical::Center);
    let bar = row![
        island(workspaces).padding(Padding::from([0.0, 20.0])),
        Space::new().width(Length::Fill),
        island(status).padding(Padding::from([0.0, 20.0])),
    ]
    .align_y(Vertical::Center);
    column![
        bar,
        row![
            pill(Icon::Folder, "Files"),
            pill(Icon::Terminal, "Console"),
            keys
        ]
        .spacing(16)
        .align_y(Vertical::Center),
        card(
            column![
                text("Card", TypeStyle::TITLE_M),
                text(
                    "Solid, 24 px corners. Rows on a card step up a tone.",
                    TypeStyle::BODY_M
                )
                .class(Tone::OnSurfaceVariant),
            ]
            .spacing(4)
        ),
    ]
    .spacing(16)
    .into()
}

/// The whole gallery. `press` is the message every enabled control sends.
pub fn view<'a, M: Clone + 'a>(press: M) -> Element<'a, M> {
    let left = column![
        section("Buttons", buttons(&press)),
        section("Icon buttons", icon_buttons(&press)),
        section("Chips and connected groups", selection(&press)),
    ]
    .spacing(32)
    .width(Length::FillPortion(1));
    let right = column![
        section("Bar, labels, keys", chrome()),
        section("Navigation and lists", navigation(&press)),
    ]
    .spacing(32)
    .width(Length::FillPortion(1));
    let body = Row::new().push(left).push(right).spacing(48);
    let page = column![text("Component index", TypeStyle::HEADLINE_L), body].spacing(24);
    iced_widget::container(page)
        .padding(40)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}
