//! Menus: Glass, 48 px pill rows, 10 px inset. The panel's corner is concentric
//! with the rows' (24 + 10), so the hover pill hugs it.

use iced_core::alignment::Vertical;
use iced_core::font::Weight;
use iced_core::{Length, Padding};
use iced_widget::{Column, Row, Space};
use triton_design::{Family, Icon, IconStyle, Material, SIZE};

use crate::button::ButtonClass;
use crate::scroll::{Divider, divider};
use crate::surface::{Panel, wrap};
use crate::text::{icon, ui};
use crate::{Element, Tone};

/// Panel inset around the rows.
const INSET: f32 = 10.0;

/// One menu entry.
#[derive(Debug, Clone)]
pub enum Entry<'a, M> {
    Item {
        icon: Option<Icon>,
        label: &'a str,
        /// A shortcut shown on the right in mono, e.g. "Ctrl C".
        shortcut: Option<&'a str>,
        /// Opens a submenu: shows a chevron instead of a shortcut.
        submenu: bool,
        /// Its submenu is showing: keeps the highlight.
        open: bool,
        destructive: bool,
        on_press: Option<M>,
    },
    Divider,
}

fn row<'a, M: Clone + 'a>(entry: Entry<'a, M>) -> Element<'a, M> {
    let Entry::Item {
        icon: glyph,
        label,
        shortcut,
        submenu,
        open,
        destructive,
        on_press,
    } = entry
    else {
        return iced_widget::container(divider(Divider::Glass))
            .padding(Padding::from([6.0, 14.0]))
            .into();
    };
    let side = if destructive {
        Tone::Error
    } else {
        Tone::OnSurfaceVariant
    };
    let mut content = Row::new().spacing(14).align_y(Vertical::Center);
    if let Some(g) = glyph {
        content = content.push(icon(g, IconStyle::Outlined, 22.0).class(side));
    }
    content = content.push(ui(label, 15.0, Weight::Medium));
    content = content.push(Space::new().width(Length::Fill));
    if submenu {
        content = content.push(icon(Icon::ChevronRight, IconStyle::Outlined, 22.0).class(side));
    } else if let Some(keys) = shortcut {
        content = content.push(
            iced_widget::text(keys)
                .font(iced_core::Font::with_name(Family::Mono.name()))
                .size(13)
                .class(side),
        );
    }
    crate::press::pressable(iced_widget::container(content).center_y(Length::Fill))
        .width(Length::Fill)
        .height(SIZE.control_l)
        .padding(Padding::from([0.0, 16.0]))
        .on_press_maybe(on_press)
        .class(ButtonClass::MenuItem { destructive, open })
        .into()
}

/// A menu panel. Set its width; rows fill it.
pub fn menu<'a, M: Clone + 'a>(entries: Vec<Entry<'a, M>>) -> Panel<'a, M> {
    let rows = Column::with_children(entries.into_iter().map(row));
    wrap(rows, Material::Glass, SIZE.control_l / 2.0, INSET)
}
