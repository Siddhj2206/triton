//! Every component in one view, as on the canvas's Component index and Controls
//! boards. The snapshot test renders it; the preview can show it too.

use iced_core::alignment::Vertical;
use iced_core::{Length, Padding};
use iced_widget::{Column, Row, Space, column, row};
use triton_design::{Icon, Material, TypeStyle};

use crate::button::{button, chip, icon_button, nav_item, segment_end, segmented};
use crate::control::{checkbox, radio, switch};
use crate::field::{Field, filled_input, search, text_field};
use crate::menu::{Entry, menu};
use crate::progress::{WaveTone, circular_progress, flat_progress, wave_shape, wavy_progress};
use crate::scroll::{Divider, divider, scroll};
use crate::slider::slider;
use crate::surface::{ListItem, card, icon_disc, island, kbd, list, pill, surface, tag, wrap};
use crate::surface::{toast, tooltip_label};
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

/// A handler that ignores its input and sends `m`.
fn sends<M: Clone, T>(m: &M) -> impl Fn(T) -> M + use<M, T> {
    let m = m.clone();
    move |_| m.clone()
}

fn controls<'a, M: Clone + 'a>(press: &M) -> Element<'a, M> {
    let p = || Some(press.clone());
    let toggles = row![
        switch(false).on_press_maybe(p()),
        switch(true).on_press_maybe(p()),
        checkbox(false).on_press_maybe(p()),
        checkbox(true).on_press_maybe(p()),
        radio(false).on_press_maybe(p()),
        radio(true).on_press_maybe(p()),
        switch(true),
    ]
    .spacing(20)
    .align_y(Vertical::Center);
    let sliders = column![
        row![
            icon(Icon::Brightness6, triton_design::IconStyle::Outlined, 24.0),
            slider(0.7, sends(press)),
        ]
        .spacing(12)
        .align_y(Vertical::Center),
        row![
            icon(Icon::VolumeUp, triton_design::IconStyle::Outlined, 24.0),
            slider(0.0, sends(press)),
        ]
        .spacing(12)
        .align_y(Vertical::Center),
    ]
    .spacing(6);
    let progress = column![
        wavy_progress(0.55, 0.0),
        flat_progress(0.62),
        row![
            circular_progress(0.75, 0.0),
            circular_progress(0.3, 0.0),
            iced_widget::stack![
                wave_shape(48.0, WaveTone::Tertiary),
                iced_widget::container(text("SK", TypeStyle::TITLE_M).class(Tone::OnSurface))
                    .center(48),
            ],
            wave_shape(40.0, WaveTone::Primary),
        ]
        .spacing(20)
        .align_y(Vertical::Center),
    ]
    .spacing(16);
    column![toggles, sliders, progress].spacing(20).into()
}

fn entry<'a, M: Clone + 'a>(press: &M) -> Element<'a, M> {
    let fields = column![
        search("Search apps and files", "", Field::Search).on_input(sends(press)),
        text_field(
            "Folder name",
            filled_input("", "niri-shell", false).on_input(sends(press)),
            None
        ),
        text_field(
            "Hostname",
            filled_input("", "triton box", true).on_input(sends(press)),
            Some("Hostnames can't contain spaces.")
        ),
    ]
    .spacing(14);
    fields.into()
}

fn floating<'a, M: Clone + 'a>(press: &M) -> Element<'a, M> {
    let p = || Some(press.clone());
    let item = |glyph, label, shortcut: Option<&'a str>| Entry::Item {
        icon: Some(glyph),
        label,
        shortcut,
        submenu: false,
        open: false,
        destructive: false,
        on_press: p(),
    };
    let menu_panel = menu(vec![
        item(Icon::FolderOpen, "Open", Some("Enter")),
        Entry::Item {
            icon: Some(Icon::OpenInNew),
            label: "Open with",
            shortcut: None,
            submenu: true,
            open: true,
            destructive: false,
            on_press: p(),
        },
        item(Icon::ContentCopy, "Copy path", Some("Ctrl C")),
        item(Icon::Edit, "Rename", Some("F2")),
        Entry::Divider,
        Entry::Item {
            icon: Some(Icon::Delete),
            label: "Move to Trash",
            shortcut: Some("Del"),
            submenu: false,
            open: false,
            destructive: true,
            on_press: p(),
        },
    ])
    .width(310);
    let quick = surface(
        column![
            row![
                icon(Icon::LightMode, triton_design::IconStyle::Outlined, 24.0),
                slider(0.7, sends(press)).on_glass(true),
            ]
            .spacing(12)
            .align_y(Vertical::Center),
            row![
                icon(Icon::VolumeUp, triton_design::IconStyle::Outlined, 24.0),
                slider(0.42, sends(press)).on_glass(true),
            ]
            .spacing(12)
            .align_y(Vertical::Center),
            search("Search", "", Field::SearchOnGlass).on_input(sends(press)),
        ]
        .spacing(6),
        Surface::Panel(Material::Glass),
    )
    .padding(16)
    .width(Length::Fill);
    let labels = row![
        toast("Moved 3 items to Trash", Some(("Undo", press.clone()))),
        tooltip_label("Copy path"),
    ]
    .spacing(16)
    .align_y(Vertical::Center);
    let scrolled = surface(
        scroll(
            column![
                text("Scrolled content", TypeStyle::TITLE_M),
                divider(Divider::Solid),
                text("Line one", TypeStyle::BODY_M),
                text("Line two", TypeStyle::BODY_M),
                text("Line three", TypeStyle::BODY_M),
                text("Line four", TypeStyle::BODY_M),
                text("Line five", TypeStyle::BODY_M),
            ]
            .spacing(8)
            .padding(Padding::from([12.0, 16.0])),
        )
        .height(120),
        Surface::Card,
    )
    .width(220);
    column![
        row![menu_panel, column![quick, scrolled].spacing(16)].spacing(24),
        labels
    ]
    .spacing(24)
    .into()
}

/// The whole gallery. `press` is the message every enabled control sends.
pub fn view<'a, M: Clone + 'a>(press: M) -> Element<'a, M> {
    let left = column![
        section("Buttons", buttons(&press)),
        section("Icon buttons", icon_buttons(&press)),
        section("Chips and connected groups", selection(&press)),
        section("Selection, sliders, progress", controls(&press)),
    ]
    .spacing(32)
    .width(Length::FillPortion(1));
    let right = column![
        section("Bar, labels, keys", chrome()),
        section("Navigation and lists", navigation(&press)),
        section("Text fields", entry(&press)),
        section("Menus, toasts, scrolling", floating(&press)),
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
