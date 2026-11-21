//! The canvas's "Component index" board, drawn by our own stack: a page of
//! type, color and spacing the pixel gate (`mise run pixel`) diffs against a
//! browser render of `references/.../ComponentIndex.dc.html`. Metrics come from
//! the board's CSS — where iced cannot match them (letter spacing, `<b>` at
//! weight 700) the page uses the nearest token and the diff reports it.

use iced_core::alignment::Vertical;
use iced_core::font::Weight;
use iced_core::{Length, Padding};
use iced_widget::{Column, Space, column, container, row};
use triton_design::{Family, TypeStyle};

use crate::surface::{Surface, surface};
use crate::text::{Tone, text};
use crate::{Element, Theme};

/// The board's logical size; the reference is shot at the same window size.
pub const WIDTH: f32 = 1646.0;
pub const HEIGHT: f32 = 1620.0;

const EYEBROW: TypeStyle = TypeStyle {
    size: 13.0,
    line_height: 18.0,
    weight: Weight::Semibold,
    family: Family::Ui,
    tracking: 0.14,
    uppercase: true,
};
const TITLE: TypeStyle = TypeStyle {
    size: 72.0,
    line_height: 72.0,
    weight: Weight::Semibold,
    family: Family::Display,
    tracking: 0.0,
    uppercase: false,
};
const HEAD: TypeStyle = TypeStyle {
    size: 12.0,
    line_height: 17.0,
    weight: Weight::Semibold,
    family: Family::Ui,
    tracking: 0.12,
    uppercase: true,
};
const CELL: TypeStyle = TypeStyle {
    size: 14.0,
    line_height: 18.9,
    weight: Weight::Normal,
    family: Family::Ui,
    tracking: 0.0,
    uppercase: false,
};
// `.r b`: 15 px at the row's 1.35 line-height; the name drives row height.
const NAME: TypeStyle = TypeStyle {
    size: 15.0,
    line_height: 20.25,
    weight: Weight::Semibold,
    family: Family::Ui,
    tracking: 0.0,
    uppercase: false,
};
const MONO_CELL: TypeStyle = TypeStyle {
    size: 12.5,
    line_height: 17.0,
    weight: Weight::Normal,
    family: Family::Mono,
    tracking: 0.0,
    uppercase: false,
};

const ROWS: &[(&str, &str, &str, &str, &str)] = &[
    (
        "Button",
        "Filled, tonal, outlined, text. 32, 40, 48, 56. Press tightens corners to 12.",
        "Everywhere",
        "button::Style plus corner spring",
        "t-btn",
    ),
    (
        "Icon button",
        "Standard, filled, tonal, outlined, neutral; veil on Glass. 32, 40, 48. Toggle morphs round to square.",
        "Bar, panels, previews, capture",
        "button + icon",
        "t-ibtn",
    ),
    (
        "Connected group",
        "2 to 4 segments, ends 22, inner 8. Large and veil variants for toolbars on Glass.",
        "Power, Screenshot, Settings",
        "row of buttons, per-corner radius",
        "t-seg",
    ),
    (
        "Switch",
        "52 \u{d7} 32, check in thumb when on.",
        "Settings, quick settings",
        "toggler with custom style",
        "t-switch",
    ),
    (
        "Slider",
        "16 track, 4 \u{d7} 44 handle, optional stops.",
        "Quick settings, Settings, media",
        "slider::Style",
        "t-slider",
    ),
    (
        "Wavy progress",
        "Linear and circular. Played part wavy, rest flat grey.",
        "Power, Screenshot, media, downloads",
        "canvas widget",
        "\u{2014}",
    ),
    (
        "Loading wave",
        "Wave shape rotating and morphing.",
        "Controls, Login states",
        "canvas widget",
        "\u{2014}",
    ),
    (
        "Chip",
        "Outlined or selected. 36 high, radius 10.",
        "Launcher, Clipboard, Settings",
        "button",
        "t-chip",
    ),
    (
        "Floating label",
        "Glass pill, 36 high, icon and name. Not interactive, so not a chip.",
        "Overview, capture",
        "container + text",
        "t-pill",
    ),
    (
        "Search field",
        "52 high pill, leading icon.",
        "Launcher, Clipboard, Settings",
        "text_input",
        "t-field",
    ),
    (
        "List and row",
        "2 px gaps, outer 24, inner 6, tonal icon disc.",
        "Settings, Containers",
        "column of containers",
        "t-list, t-li, t-ic",
    ),
    (
        "Nav item",
        "Pill, primary container when selected.",
        "Settings sidebar, rail",
        "button",
        "t-nav",
    ),
    (
        "Card",
        "Solid, 24 radius.",
        "Containers, Notifications",
        "container",
        "t-card",
    ),
    (
        "Menu and submenu",
        "Glass, 48 rows, 24 radius.",
        "Containers, context menus",
        "layer surface",
        "t-glass",
    ),
    (
        "Dialog",
        "Solid, 28 radius, scrim.",
        "Containers, Clipboard, Power",
        "layer surface",
        "t-thick",
    ),
    (
        "Toast and notification",
        "Glass, grouped in the center.",
        "Notifications",
        "layer surface",
        "t-glass",
    ),
    (
        "Tooltip, badge, tag",
        "Small, tertiary container for tags.",
        "Controls",
        "container",
        "t-tag",
    ),
    (
        "Key cap",
        "Mono 12, 1 px outline.",
        "Launcher, Screenshot, Switcher, Clipboard",
        "container",
        "t-kbd",
    ),
    (
        "Scrollbar",
        "6 px, auto-hides.",
        "Settings, Containers",
        "scrollable::Style",
        "\u{2014}",
    ),
    (
        "Wave shape",
        "9 lobes, filled. Identity only.",
        "Launcher, avatars, album art, swatches",
        "canvas::Path",
        "tokens.shape.wave",
    ),
    (
        "Island",
        "Glass, 56 high, full radius.",
        "Bar, media pill",
        "layer surface",
        "t-island",
    ),
    (
        "Window frame",
        "Tiled: square, no gap. Floating: 28, shadow, outline.",
        "Main, Settings, Quick settings",
        "niri rules",
        "t-window-*",
    ),
    (
        "Material surface",
        "Solid, Mica, Glass, Thick Glass.",
        "Everywhere",
        "container + niri blur",
        "t-solid, t-mica, t-glass, t-thick",
    ),
    (
        "Preview card",
        "Thumbnail, app disc, title, workspace. Selected is solid primary.",
        "Switcher, Wallpaper picker, Clipboard",
        "container",
        "\u{2014}",
    ),
    (
        "Player card",
        "Wave art, wavy progress, wave play button.",
        "Media bar",
        "layer surface",
        "\u{2014}",
    ),
    (
        "OSD",
        "Glass pill, icon in a wave or disc.",
        "OSD",
        "layer surface",
        "t-glass",
    ),
    (
        "Section label",
        "Eyebrow caps, 13 / .14em.",
        "Everywhere",
        "text",
        "t-eyebrow",
    ),
];

fn caps<'a>(content: &str, style: TypeStyle, tone: Tone) -> crate::text::Text<'a> {
    let content = if style.uppercase {
        content.to_uppercase()
    } else {
        content.to_owned()
    };
    text(content, style).class(tone)
}

fn cells<'a, M: 'a>(
    name: &'a str,
    variants: &'a str,
    used: &'a str,
    iced: &'a str,
    class: &'a str,
) -> iced_widget::Row<'a, M, Theme, crate::Renderer> {
    row![
        text(name, NAME).width(200.0),
        text(variants, CELL)
            .class(Tone::OnSurfaceVariant)
            .width(Length::Fill),
        text(used, CELL).class(Tone::OnSurfaceVariant).width(330.0),
        text(iced, MONO_CELL)
            .class(Tone::OnSurfaceVariant)
            .width(270.0),
        text(class, MONO_CELL).class(Tone::Primary).width(200.0),
    ]
    .spacing(20)
    .align_y(Vertical::Center)
}

fn view_row<'a, M: 'a>(
    name: &'a str,
    variants: &'a str,
    used: &'a str,
    iced: &'a str,
    class: &'a str,
) -> crate::surface::Panel<'a, M> {
    surface(cells(name, variants, used, iced, class), Surface::Row)
        .padding(Padding {
            top: 12.0,
            right: 16.0,
            bottom: 12.0,
            left: 16.0,
        })
        .width(Length::Fill)
}

/// The board page itself. `M` is unused: nothing on it is interactive.
pub fn view<'a, M: 'a>() -> Element<'a, M> {
    let header = column![
        caps(
            "Triton \u{b7} Futura design language",
            EYEBROW,
            Tone::Primary
        ),
        Space::new().height(10.0),
        text("Component index", TITLE),
        Space::new().height(12.0),
        text(
            "Every reusable piece, where it appears, and how it maps to iced. \
             Reference CSS lives in triton/components.css, tokens in triton/tokens.json.",
            TypeStyle::BODY_M,
        )
        .class(Tone::OnSurfaceVariant),
    ];

    let table_header = container(
        row![
            caps("Component", HEAD, Tone::Primary).width(200.0),
            caps("Variants and size", HEAD, Tone::Primary).width(Length::Fill),
            caps("Used in", HEAD, Tone::Primary).width(330.0),
            caps("iced", HEAD, Tone::Primary).width(270.0),
            caps("Class", HEAD, Tone::Primary).width(200.0),
        ]
        .spacing(20),
    )
    .padding(Padding {
        top: 12.0,
        right: 16.0,
        bottom: 6.0,
        left: 16.0,
    })
    .width(Length::Fill);

    let mut table = Column::new().spacing(3).push(table_header);
    for &(name, variants, used, iced, class) in ROWS {
        table = table.push(view_row(name, variants, used, iced, class));
    }

    surface(
        column![header, Space::new().height(36.0), table].width(1566.0),
        Surface::Canvas,
    )
    .padding(Padding::from([40.0, 0.0]))
    .center_x(Length::Fill)
    .height(Length::Fill)
    .into()
}
