//! Selection controls: switch, checkbox, radio.
//!
//! Each is a [`Pressable`] whose class draws the track, box or ring, so hover,
//! press, disabled and motion come from one place.

use iced_core::Length;
use triton_design::{Icon, IconStyle, SIZE};

use crate::button::{ButtonClass, Pressable};
use crate::press::pressable;
use crate::surface::surface;
use crate::text::icon;
use crate::{Element, Surface};

/// `.t-switch`: 52 × 32. Off: a 16 px outline dot 8 px in. On: a 24 px disc with a
/// check, 4 px from the right. The widget draws and slides the thumb.
pub fn switch<'a, M: 'a>(on: bool) -> Pressable<'a, M> {
    let (w, h) = SIZE.switch;
    pressable(iced_widget::Space::new())
        .width(w)
        .height(h)
        .class(ButtonClass::Switch { on })
}

/// A 24 px checkbox; checked shows an 18 px check.
pub fn checkbox<'a, M: 'a>(checked: bool) -> Pressable<'a, M> {
    let mark: Element<'a, M> = if checked {
        icon(Icon::Check, IconStyle::Outlined, 18.0).into()
    } else {
        iced_widget::Space::new().into()
    };
    pressable(iced_widget::container(mark).center(Length::Fill))
        .width(24)
        .height(24)
        .padding(0)
        .class(ButtonClass::Check { checked })
}

/// A 24 px radio; selected shows a 12 px dot.
pub fn radio<'a, M: 'a>(selected: bool) -> Pressable<'a, M> {
    let dot: Element<'a, M> = if selected {
        surface(iced_widget::Space::new(), Surface::RadioDot)
            .width(12)
            .height(12)
            .into()
    } else {
        iced_widget::Space::new().into()
    };
    pressable(iced_widget::container(dot).center(Length::Fill))
        .width(24)
        .height(24)
        .padding(0)
        .class(ButtonClass::Radio { selected })
}
