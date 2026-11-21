//! Text entry: the search field and the filled text field.

use iced_core::alignment::Vertical;
use iced_core::layout::{self, Layout};
use iced_core::text::Renderer as _;
use iced_core::text::{self, LineHeight, Shaping, Wrapping};
use iced_core::widget::{self, Tree, Widget};
use iced_core::{
    Background, Border, Color, Font, Length, Padding, Point, Rectangle, Size, mouse, renderer,
};
use iced_widget::text_input::{self, Catalog, Icon as InputIcon, Side, Status, Style};
use iced_widget::{Column, TextInput};
use triton_design::color::with_alpha;
use triton_design::material::veil;
use triton_design::shape::nested;
use triton_design::{Family, Icon, IconStyle, SIZE};

use crate::text::ui;
use crate::{Element, Renderer, Theme, Tone};

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
/// `.on_input(..)` to it first.
pub fn text_field<'a, M: Clone + 'a>(
    label: &'a str,
    input: Input<'a, M>,
    error: Option<&'a str>,
) -> Column<'a, M, Theme, Renderer> {
    let field: Element<'a, M> = FilledField {
        inner: input,
        label,
        error: error.is_some(),
    }
    .into();
    let mut col = Column::new().push(field).spacing(6);
    if let Some(e) = error {
        col = col.push(
            iced_widget::container(ui(e, 12.0, iced_core::font::Weight::Normal).class(Tone::Error))
                .padding(Padding::from([0.0, 16.0])),
        );
    }
    col
}

/// The label's inset inside the field.
const LABEL_X: f32 = 16.0;
const LABEL_Y: f32 = 8.0;

/// The field surface: delegates everything to its input and paints the label
/// itself, so the label turns primary while the input is focused — a sibling
/// widget cannot see another's focus.
pub struct FilledField<'a, M> {
    inner: Input<'a, M>,
    label: &'a str,
    error: bool,
}

impl<M> std::fmt::Debug for FilledField<'_, M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FilledField")
            .field("label", &self.label)
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}

impl<M: Clone> Widget<M, Theme, Renderer> for FilledField<'_, M> {
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.inner as &dyn Widget<M, Theme, Renderer>)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(&[&self.inner as &dyn Widget<M, Theme, Renderer>]);
    }

    fn size(&self) -> Size<Length> {
        Widget::size(&self.inner)
    }

    fn size_hint(&self) -> Size<Length> {
        Widget::size_hint(&self.inner)
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        Widget::layout(&mut self.inner, &mut tree.children[0], renderer, limits)
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        self.inner
            .operate(&mut tree.children[0], layout, renderer, operation);
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &iced_core::Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn iced_core::Clipboard,
        shell: &mut iced_core::Shell<'_, M>,
        viewport: &Rectangle,
    ) {
        self.inner.update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.inner
            .mouse_interaction(&tree.children[0], layout, cursor, viewport, renderer)
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        Widget::draw(
            &self.inner,
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
        let focused = tree.children[0]
            .state
            .downcast_ref::<text_input::State<<Renderer as text::Renderer>::Paragraph>>()
            .is_focused();
        let s = &theme.scheme;
        let color = if self.error {
            s.error
        } else if focused {
            s.primary
        } else {
            s.on_surface_variant
        };
        let b = layout.bounds();
        renderer.fill_text(
            text::Text {
                content: self.label.to_owned(),
                bounds: Size::new(b.width - LABEL_X * 2.0, 12.0),
                size: 12.0.into(),
                line_height: LineHeight::Relative(1.3),
                font: Font {
                    family: iced_core::font::Family::Name(Family::Ui.name()),
                    weight: iced_core::font::Weight::Normal,
                    ..Font::DEFAULT
                },
                align_x: text::Alignment::Left,
                align_y: Vertical::Top,
                shaping: Shaping::Basic,
                wrapping: Wrapping::None,
            },
            Point::new(b.x + LABEL_X, b.y + LABEL_Y),
            color,
            *viewport,
        );
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: iced_core::Vector,
    ) -> Option<iced_core::overlay::Element<'b, M, Theme, Renderer>> {
        self.inner.overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}

impl<'a, M: Clone + 'a> From<FilledField<'a, M>> for Element<'a, M> {
    fn from(field: FilledField<'a, M>) -> Self {
        iced_core::Element::new(field)
    }
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
