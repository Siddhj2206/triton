//! Scrollbars and dividers.
//!
//! The scrollbar is a 4 px thumb, 4 px from the edge, no rail: on-surface at 50%,
//! 75% while hovered or dragged. (The canvas widens it to 10 px on hover; iced's
//! scrollable has a fixed width, so that waits for our own scroll widget.)

use iced_core::{Background, Border, Color, Element, Shadow};
use iced_widget::container;
use iced_widget::rule::{self, FillMode};
use iced_widget::scrollable::{
    self, AutoScroll, Direction, Rail, Scrollable, Scrollbar, Scroller, Status,
};
use triton_design::color::with_alpha;
use triton_design::material::veil_hi;

use crate::{Renderer, Theme};

const THUMB: f32 = 4.0;
const MARGIN: f32 = 4.0;
const REST: f32 = 0.5;
const HOVER: f32 = 0.75;

impl scrollable::Catalog for Theme {
    type Class<'a> = ();

    fn default<'a>() -> Self::Class<'a> {}

    fn style(&self, _class: &(), status: Status) -> scrollable::Style {
        let s = &self.scheme;
        let (v_hot, h_hot) = match status {
            Status::Active { .. } => (false, false),
            Status::Hovered {
                is_vertical_scrollbar_hovered: v,
                is_horizontal_scrollbar_hovered: h,
                ..
            }
            | Status::Dragged {
                is_vertical_scrollbar_dragged: v,
                is_horizontal_scrollbar_dragged: h,
                ..
            } => (v, h),
        };
        let rail = |hot: bool| Rail {
            background: None,
            border: Border::default(),
            scroller: Scroller {
                background: Background::Color(with_alpha(
                    s.on_surface,
                    if hot { HOVER } else { REST },
                )),
                border: Border::default().rounded(THUMB / 2.0),
            },
        };
        scrollable::Style {
            container: container::Style::default(),
            vertical_rail: rail(v_hot),
            horizontal_rail: rail(h_hot),
            gap: None,
            auto_scroll: AutoScroll {
                background: Background::Color(s.surface_container_high),
                border: Border::default().rounded(THUMB * 3.0),
                shadow: Shadow::default(),
                icon: s.on_surface,
            },
        }
    }
}

/// Vertical scrolling with Triton's thin scrollbar.
pub fn scroll<'a, M: 'a>(
    content: impl Into<Element<'a, M, Theme, Renderer>>,
) -> Scrollable<'a, M, Theme, Renderer> {
    let bar = Scrollbar::new()
        .width(THUMB)
        .scroller_width(THUMB)
        .margin(MARGIN);
    iced_widget::scrollable(content).direction(Direction::Vertical(bar))
}

/// What a divider sits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Divider {
    /// outline-variant, 1 px.
    #[default]
    Solid,
    /// On Glass (menus): the veil highlight.
    Glass,
}

impl rule::Catalog for Theme {
    type Class<'a> = Divider;

    fn default<'a>() -> Self::Class<'a> {
        Divider::Solid
    }

    fn style(&self, class: &Divider) -> rule::Style {
        let color: Color = match class {
            Divider::Solid => self.scheme.outline_variant,
            Divider::Glass => veil_hi(self.variant),
        };
        rule::Style {
            color,
            radius: 0.0.into(),
            fill_mode: FillMode::Full,
            snap: true,
        }
    }
}

/// A 1 px horizontal divider.
pub fn divider<'a>(on: Divider) -> iced_widget::Rule<'a, Theme> {
    iced_widget::rule::horizontal(1).class(on)
}
