//! Text entry: the search field and the filled text field.

use iced_core::text::LineHeight;
use iced_core::{Background, Border, Color, Length, Padding};
use iced_widget::text_input::{Catalog, Icon as InputIcon, Side, Status, Style};
use iced_widget::{Column, TextInput};
use triton_design::color::with_alpha;
use triton_design::material::veil;
use triton_design::shape::nested;
use triton_design::{Icon, IconStyle, SIZE};

use crate::text::ui;
use crate::{Renderer, Theme, Tone};

/// What a text input is.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Field {
    /// `.t-field`: the 52 px search pill. `veil` on Glass.
    #[default]
    Search,
    SearchOnGlass,
    /// A `veil` search field nested `inset` px inside a container of radius
    /// `outer`: its radius is `nested(outer, inset)` so the two corners are
    /// concentric (the same rule `Surface::Wrap` applies).
    Nested {
        outer: f32,
        inset: f32,
    },
    /// A filled field: 56 high, radius 16. Focus draws a 2 px primary ring;
    /// `error` a 2 px error ring.
    Filled {
        error: bool,
    },
}

/// Selection highlight: primary at this alpha.
const SELECTION: f32 = 0.35;
/// Filled field: height, radius, focus ring width, label row.
const FILLED_HEIGHT: f32 = 56.0;
const FILLED_RADIUS: f32 = 16.0;
const RING: f32 = 2.0;

impl Catalog for Theme {
    type Class<'a> = Field;

    fn default<'a>() -> Self::Class<'a> {
        Field::Search
    }

    fn style(&self, class: &Field, status: Status) -> Style {
        let s = &self.scheme;
        let (background, radius) = match class {
            Field::Search => (s.surface_container_highest, SIZE.search_field / 2.0),
            Field::SearchOnGlass => (veil(self.variant), SIZE.search_field / 2.0),
            Field::Nested { outer, inset } => (veil(self.variant), nested(*outer, *inset)),
            Field::Filled { .. } => (s.surface_container_highest, FILLED_RADIUS),
        };
        let ring = match (class, status) {
            (Field::Filled { error: true }, _) => Some(s.error),
            (Field::Filled { .. }, Status::Focused { .. }) => Some(s.primary),
            _ => None,
        };
        let disabled = status == Status::Disabled;
        let value = if disabled {
            with_alpha(
                s.on_surface,
                triton_design::color::STATE_LAYER.disabled_content,
            )
        } else {
            s.on_surface
        };
        Style {
            background: Background::Color(background),
            border: Border {
                color: ring.unwrap_or(Color::TRANSPARENT),
                width: if ring.is_some() { RING } else { 0.0 },
                radius: radius.into(),
            },
            icon: s.on_surface_variant,
            placeholder: s.on_surface_variant,
            value,
            selection: with_alpha(s.primary, SELECTION),
        }
    }
}

pub type Input<'a, M> = TextInput<'a, M, Theme, Renderer>;

/// `.t-field`: the search field with a leading search icon, styled by `class`
/// (`Field::SearchOnGlass` on a panel, `Field::Nested` inside a rounded one).
/// Add `.on_input(..)`.
pub fn search<'a, M: Clone + 'a>(placeholder: &str, value: &str, class: Field) -> Input<'a, M> {
    iced_widget::text_input(placeholder, value)
        .icon(InputIcon {
            font: Icon::font(IconStyle::Outlined),
            code_point: Icon::Search.codepoint(),
            size: Some(24.0.into()),
            spacing: 12.0,
            side: Side::Left,
        })
        .size(16)
        .line_height(LineHeight::Absolute(24.0.into()))
        .padding(Padding::from([14.0, 18.0]))
        .width(Length::Fill)
        .class(class)
}

/// A filled text field: the label sits inside the field above the value, and an
/// optional error line goes under it. Build `input` with [`filled_input`] and add
/// `.on_input(..)` to it first. (The label does not turn primary on focus: it cannot
/// see the input's focus. That comes with a field widget of our own.)
pub fn text_field<'a, M: Clone + 'a>(
    label: &'a str,
    input: Input<'a, M>,
    error: Option<&'a str>,
) -> Column<'a, M, Theme, Renderer> {
    let label = ui(label, 12.0, iced_core::font::Weight::Normal).class(if error.is_some() {
        Tone::Error
    } else {
        Tone::OnSurfaceVariant
    });
    let field = iced_widget::stack![
        input,
        iced_widget::container(label).padding(Padding {
            top: 8.0,
            left: 16.0,
            ..Padding::ZERO
        }),
    ];
    let mut col = Column::new().push(field).spacing(6);
    if let Some(e) = error {
        col = col.push(
            iced_widget::container(ui(e, 12.0, iced_core::font::Weight::Normal).class(Tone::Error))
                .padding(Padding::from([0.0, 16.0])),
        );
    }
    col
}

/// The input inside a [`text_field`]: 56 high, the value set below the label.
pub fn filled_input<'a, M: Clone + 'a>(
    placeholder: &str,
    value: &str,
    error: bool,
) -> Input<'a, M> {
    iced_widget::text_input(placeholder, value)
        .size(16)
        .line_height(LineHeight::Absolute(22.0.into()))
        .padding(Padding {
            top: 26.0,
            bottom: FILLED_HEIGHT - 26.0 - 22.0,
            left: 16.0,
            right: 16.0,
        })
        .width(Length::Fill)
        .class(Field::Filled { error })
}
