//! Containers: materials, islands, cards, list rows and the small labels.

use iced_core::layout::{self, Layout};
use iced_core::widget::{Tree, Widget, tree};
use iced_core::{Color, Length, Padding, Rectangle, Shadow, Vector, mouse, renderer};
use iced_widget::container::{Catalog, Style};
use iced_widget::text::IntoFragment;
use iced_widget::{Column, Container, Row, Space};
use triton_design::color::with_alpha;
use triton_design::material::ShadowToken;
use triton_design::shape::{concentric, list_corners};
use triton_design::{Icon, IconStyle, Material, RADII, SIZE};

use crate::paint::{Paint, PaintCache};
use crate::text::icon;
use crate::{Element, Renderer, Theme};

/// What a container is. Each variant is a class in `components.css`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Surface {
    /// No fill: layout only.
    #[default]
    None,
    /// A material edge to edge, square (tiled chrome: the pinned bar, sidebars).
    Material(Material),
    /// A floating panel: a material with the 28 px floating radius.
    Panel(Material),
    /// A floating panel that is its own layer surface: niri casts its shadow (a
    /// layer rule), so it paints only fill and highlight. A large iced shadow costs
    /// seconds per frame on tiny-skia (spike 1).
    LayerPanel(Material),
    /// A floating material tightly wrapping one control whose corner radius is
    /// `inner`, `inset` px in. Its corner is concentric with the control's.
    Wrap {
        material: Material,
        inner: f32,
        inset: f32,
    },
    /// `.t-island`: a Glass pill in the bar.
    Island,
    /// An island that is its own layer surface: niri casts its shadow and clips
    /// its blur to the pill (a layer rule), so it paints only fill and highlight.
    LayerIsland,
    /// `.t-pill`: a non-interactive Glass label (overview window names, hints).
    Pill,
    /// `.t-card`: Solid, 24 px corners.
    Card,
    /// Flat `surface-container-lowest` fill: a board page's backdrop.
    Canvas,
    /// Flat `surface-container-low` fill at radius 14: a dense data row.
    Row,
    /// A dialog: Solid, 28 px corners, over a scrim.
    Dialog,
    /// `.t-li`: row `index` of `count` in a grouped list.
    ListRow {
        index: usize,
        count: usize,
        selected: bool,
        /// Rows on a card step up one tone.
        on_card: bool,
    },
    /// `.t-ic`: the tonal disc behind a list icon.
    IconDisc,
    /// `.t-kbd`: a key cap.
    Kbd,
    /// `.t-tag`: a small tertiary label.
    Tag,
    /// The bar's status group: a container pill tinted by what it reports —
    /// primary while all is well, the board's warm colors otherwise.
    Status(StatusTone),
    /// The bell's unread dot: tertiary.
    Dot,
    /// The radio's 12 px primary dot.
    RadioDot,
    /// A Glass tooltip: 32 high, radius 12.
    Tooltip,
    /// `.t-scrim`: the input-capture surface a modal sits on. Paints nothing —
    /// the modal is modal by input, not by dimming.
    Scrim,
}

/// How the status group reads: blue when all is well, rust (tertiary) when
/// something is off, error red when it is urgent.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum StatusTone {
    #[default]
    Normal,
    Warn,
    Alert,
}

impl Theme {
    fn status_paint(&self, tone: StatusTone) -> (Paint, Option<Color>) {
        let s = &self.scheme;
        let (fill, on) = match tone {
            StatusTone::Normal => (s.primary_container, s.on_primary_container),
            StatusTone::Warn => (s.tertiary_container, s.on_tertiary_container),
            StatusTone::Alert => (s.error_container, s.on_error_container),
        };
        (
            Paint {
                fill: Some(fill),
                radii: [RADII.full; 4],
                smooth: true,
                ..Paint::default()
            },
            Some(on),
        )
    }
}

fn shadow(token: Option<ShadowToken>) -> Option<Shadow> {
    token.map(ShadowToken::to_iced)
}

impl Theme {
    /// What a surface paints, and the content color it sets (`None` inherits).
    pub(crate) fn surface_paint(&self, class: Surface) -> (Paint, Option<Color>) {
        let s = &self.scheme;
        let v = self.variant;
        // In-shell shapes: flat, smoothed corners.
        let solid = |c: Color, r: f32| Paint {
            fill: Some(c),
            radii: [r; 4],
            smooth: true,
            ..Paint::default()
        };
        // Glass and Thick float: circular corners like niri's windows, a shadow
        // underneath, the 1 px highlight on top.
        let floating = |m: Material, r: f32| Paint {
            fill: Some(m.fill_adaptive(s, v, self.bright_backdrop)),
            radii: [r; 4],
            smooth: false,
            shadow: shadow(m.shadow()),
            highlight: matches!(m, Material::Glass | Material::Thick),
            border: None,
        };
        // Its own layer surface: niri casts the shadow.
        let layered = |m: Material, r: f32| Paint {
            shadow: None,
            ..floating(m, r)
        };
        let on = Some(s.on_surface);
        match class {
            // Scrim is `None` too: transparent — it exists so a press anywhere
            // outside the modal dismisses it.
            Surface::None | Surface::Scrim => (Paint::default(), None),
            Surface::Material(m) => (floating(m, RADII.tiled), on),
            Surface::Panel(m) => (floating(m, RADII.floating_window), on),
            Surface::LayerPanel(m) => (layered(m, RADII.floating_window), on),
            Surface::Wrap {
                material,
                inner,
                inset,
            } => (floating(material, concentric(inner, inset)), on),
            Surface::Island => (floating(Material::Glass, SIZE.bar_height / 2.0), on),
            Surface::LayerIsland => (layered(Material::Glass, SIZE.bar_height / 2.0), on),
            // .t-pill: 36 high, radius 18.
            Surface::Pill => (floating(Material::Glass, 18.0), on),
            Surface::Tooltip => (
                Paint {
                    shadow: Some(ShadowToken::RAISED.to_iced()),
                    ..floating(Material::Glass, RADII.sm)
                },
                on,
            ),
            Surface::RadioDot => (solid(s.primary, RADII.full), None),
            Surface::Status(tone) => self.status_paint(tone),
            Surface::Dot => (solid(s.tertiary, RADII.full), None),
            // The board page's backdrop and its data rows.
            Surface::Canvas => (solid(s.surface_container_lowest, 0.0), None),
            Surface::Row => (solid(s.surface_container_low, 14.0), None),
            Surface::Card => (solid(s.surface_container_low, RADII.list_outer), None),
            Surface::Dialog => (
                Paint {
                    smooth: false,
                    shadow: shadow(Material::Thick.shadow()),
                    ..solid(s.surface_container_low, RADII.floating_window)
                },
                None,
            ),
            Surface::ListRow {
                index,
                count,
                selected,
                on_card,
            } => {
                let bg = match (selected, on_card) {
                    (true, _) => s.secondary_container,
                    (false, true) => s.surface_container_high,
                    (false, false) => s.surface_container,
                };
                let text = if selected {
                    s.on_secondary_container
                } else {
                    s.on_surface
                };
                (
                    Paint {
                        radii: list_corners(index, count),
                        ..solid(bg, 0.0)
                    },
                    Some(text),
                )
            }
            Surface::IconDisc => (
                solid(s.secondary_container, RADII.full),
                Some(s.on_secondary_container),
            ),
            // .t-kbd: 1 px outline at .35, radius 7.
            Surface::Kbd => (
                Paint {
                    fill: None,
                    border: Some((with_alpha(s.on_surface, 0.35), 1.0)),
                    ..solid(Color::TRANSPARENT, 7.0)
                },
                on,
            ),
            // .t-tag: radius 10.
            Surface::Tag => (
                solid(s.tertiary_container, 10.0),
                Some(s.on_tertiary_container),
            ),
        }
    }
}

/// The container only lays out and sets the content color; [`Panel`] paints.
impl Catalog for Theme {
    type Class<'a> = Surface;

    fn default<'a>() -> Self::Class<'a> {
        Surface::None
    }

    fn style(&self, class: &Surface) -> Style {
        Style {
            text_color: self.surface_paint(*class).1,
            ..Style::default()
        }
    }
}

/// A surface: an iced container for layout, painted by [`paint`](crate::paint) so
/// it gets smoothed corners and the Glass highlight.
pub struct Panel<'a, M> {
    inner: Container<'a, M, Theme, Renderer>,
    class: Surface,
}

impl<M> std::fmt::Debug for Panel<'_, M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Panel")
            .field("class", &self.class)
            .finish_non_exhaustive()
    }
}

macro_rules! forward {
    ($($name:ident($($arg:ident: $ty:ty),*)),* $(,)?) => {
        $(
            #[must_use]
            pub fn $name(mut self, $($arg: $ty),*) -> Self {
                self.inner = self.inner.$name($($arg),*);
                self
            }
        )*
    };
}

impl<M> Panel<'_, M> {
    forward!(
        padding(p: impl Into<Padding>),
        width(w: impl Into<Length>),
        height(h: impl Into<Length>),
        max_width(w: impl Into<iced_core::Pixels>),
        center_x(w: impl Into<Length>),
        center_y(h: impl Into<Length>),
        center(l: impl Into<Length>),
        align_x(a: impl Into<iced_core::alignment::Horizontal>),
        align_y(a: impl Into<iced_core::alignment::Vertical>),
    );
}

impl<M> Widget<M, Theme, Renderer> for Panel<'_, M> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<PaintCache>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(PaintCache::default())
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.inner as &dyn Widget<M, Theme, Renderer>)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(&[&self.inner as &dyn Widget<M, Theme, Renderer>]);
    }

    fn size(&self) -> iced_core::Size<Length> {
        self.inner.size()
    }

    fn size_hint(&self) -> iced_core::Size<Length> {
        self.inner.size_hint()
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.inner.layout(&mut tree.children[0], renderer, limits)
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn iced_core::widget::Operation,
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
        let bounds = layout.bounds();
        let layered = bounds.intersects(viewport) && {
            let (paint, _) = theme.surface_paint(self.class);
            crate::paint::draw(renderer, bounds, paint, tree.state.downcast_ref())
        };
        let content = |r: &mut Renderer| {
            self.inner
                .draw(&tree.children[0], r, theme, style, layout, cursor, viewport);
        };
        if layered {
            iced_core::Renderer::with_layer(renderer, *viewport, content);
        } else {
            content(renderer);
        }
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
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

impl<'a, M: 'a> From<Panel<'a, M>> for Element<'a, M> {
    fn from(panel: Panel<'a, M>) -> Self {
        iced_core::Element::new(panel)
    }
}

/// Any content in a surface.
pub fn surface<'a, M: 'a>(content: impl Into<Element<'a, M>>, class: Surface) -> Panel<'a, M> {
    Panel {
        inner: iced_widget::container(content).class(class),
        class,
    }
}

/// A floating material around one control with corner radius `inner`, `inset` px
/// on every side, its corners concentric with the control's.
pub fn wrap<'a, M: 'a>(
    content: impl Into<Element<'a, M>>,
    material: Material,
    inner: f32,
    inset: f32,
) -> Panel<'a, M> {
    surface(
        content,
        Surface::Wrap {
            material,
            inner,
            inset,
        },
    )
    .padding(inset)
}

/// The bar island's inner padding.
pub const ISLAND_INSET: f32 = 6.0;

/// A Glass tooltip label, for iced's `tooltip` widget.
pub fn tooltip_label<'a, M: 'a>(label: &'a str) -> Panel<'a, M> {
    surface(
        crate::text::ui(label, 13.0, iced_core::font::Weight::Medium),
        Surface::Tooltip,
    )
    .height(32)
    .center_y(32)
    .padding(Padding::from([0.0, 12.0]))
}

/// A toast: a Glass pill, 56 high, with a message and an optional text action. The
/// 40 px action sits 8 px in, concentric with the 28 px ends.
pub fn toast<'a, M: Clone + 'a>(message: &'a str, action: Option<(&'a str, M)>) -> Panel<'a, M> {
    let mut content = Row::new()
        .spacing(22)
        .align_y(iced_core::alignment::Vertical::Center)
        .push(crate::text::ui(
            message,
            15.0,
            iced_core::font::Weight::Medium,
        ));
    let right = if let Some((label, on_press)) = action {
        content = content.push(
            crate::button::button(label, None, crate::Kind::Text, crate::Size::M)
                .on_press(on_press),
        );
        8.0
    } else {
        24.0
    };
    surface(content, Surface::Island)
        .height(SIZE.bar_height)
        .center_y(SIZE.bar_height)
        .padding(Padding {
            left: 24.0,
            right,
            ..Padding::ZERO
        })
}

/// A bar island (`.isl`): 56 high, 6 px inset, content centered vertically. Its
/// 44 px end items are concentric with its ends (22 + 6 = 28).
pub fn island<'a, M: 'a>(content: impl Into<Element<'a, M>>) -> Panel<'a, M> {
    surface(content, Surface::Island)
        .height(SIZE.bar_height)
        .padding(Padding::from([0.0, ISLAND_INSET]))
        .center_y(SIZE.bar_height)
}

/// An [`island`] that fills its own layer surface (see [`Surface::LayerIsland`]).
pub fn layer_island<'a, M: 'a>(content: impl Into<Element<'a, M>>) -> Panel<'a, M> {
    surface(content, Surface::LayerIsland)
        .height(SIZE.bar_height)
        .padding(Padding::from([0.0, ISLAND_INSET]))
        .center_y(SIZE.bar_height)
}

/// `.t-card`: Solid, 24 px corners, padding 20 × 22.
pub fn card<'a, M: 'a>(content: impl Into<Element<'a, M>>) -> Panel<'a, M> {
    surface(content, Surface::Card).padding(Padding::from([20.0, 22.0]))
}

/// `.t-pill`: icon and label on Glass, 36 high.
pub fn pill<'a, M: 'a>(glyph: Icon, label: impl IntoFragment<'a>) -> Panel<'a, M> {
    let content = Row::new()
        .push(icon(glyph, IconStyle::Outlined, 18.0))
        .push(crate::text::ui(
            label,
            14.0,
            iced_core::font::Weight::Semibold,
        ))
        .spacing(8)
        .align_y(iced_core::alignment::Vertical::Center);
    surface(content, Surface::Pill)
        .height(36)
        .padding(Padding {
            left: 10.0,
            right: 14.0,
            ..Padding::ZERO
        })
        .center_y(36)
}

/// `.t-ic`: a 40 px tonal disc with a 22 px icon.
pub fn icon_disc<'a, M: 'a>(glyph: Icon) -> Panel<'a, M> {
    surface(icon(glyph, IconStyle::Outlined, 22.0), Surface::IconDisc)
        .width(40)
        .height(40)
        .center(40)
}

/// `.t-kbd`: a key cap in mono 12/600.
pub fn kbd<'a, M: 'a>(key: &'a str) -> Panel<'a, M> {
    let label = iced_widget::text(key)
        .font(iced_core::Font {
            weight: iced_core::font::Weight::Semibold,
            ..triton_design::TypeStyle::MONO.font()
        })
        .size(12);
    surface(label, Surface::Kbd)
        .height(24)
        .padding(Padding::from([0.0, 6.0]))
        .center_y(24)
}

/// `.t-tag`: 12/600 on tertiary container.
pub fn tag<'a, M: 'a>(label: &'a str) -> Panel<'a, M> {
    surface(
        crate::text::ui(label, 12.0, iced_core::font::Weight::Semibold),
        Surface::Tag,
    )
    .padding(Padding::from([3.0, 10.0]))
}

/// One row of a grouped list: optional leading element, title, supporting line,
/// optional trailing element.
pub struct ListItem<'a, M> {
    pub leading: Option<Element<'a, M>>,
    pub title: &'a str,
    pub supporting: Option<&'a str>,
    pub trailing: Option<Element<'a, M>>,
    pub selected: bool,
    /// Press message; rows without one stay inert.
    pub on_press: Option<M>,
}

impl<M> std::fmt::Debug for ListItem<'_, M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ListItem")
            .field("title", &self.title)
            .field("supporting", &self.supporting)
            .field("selected", &self.selected)
            .finish_non_exhaustive()
    }
}

/// `.t-list`: rows with 2 px gaps, outer corners on the ends only. Rows are at
/// least 64 px high.
pub fn list<'a, M: Clone + 'a>(
    items: Vec<ListItem<'a, M>>,
    on_card: bool,
) -> Column<'a, M, Theme, Renderer> {
    let count = items.len();
    let rows = items.into_iter().enumerate().map(|(index, item)| {
        let mut texts = Column::new().push(crate::text::ui(
            item.title,
            16.0,
            iced_core::font::Weight::Medium,
        ));
        if let Some(s) = item.supporting {
            let tone = if item.selected {
                crate::Tone::Inherit
            } else {
                crate::Tone::OnSurfaceVariant
            };
            texts =
                texts.push(crate::text::ui(s, 13.0, iced_core::font::Weight::Normal).class(tone));
        }
        let mut row = Row::new()
            // Min height: 64 minus the 12 px vertical padding on each side.
            .push(Space::new().width(0).height(SIZE.list_row_min - 24.0))
            .spacing(16)
            .align_y(iced_core::alignment::Vertical::Center);
        if let Some(l) = item.leading {
            row = row.push(l);
        }
        row = row.push(iced_widget::container(texts).width(Length::Fill));
        if let Some(t) = item.trailing {
            row = row.push(t);
        }
        let row = surface(
            row,
            Surface::ListRow {
                index,
                count,
                selected: item.selected,
                on_card,
            },
        )
        .width(Length::Fill)
        .padding(Padding::from([12.0, 20.0]));
        match item.on_press {
            Some(m) => iced_widget::mouse_area(row).on_press(m).into(),
            None => row.into(),
        }
    });
    Column::with_children(rows).spacing(2)
}
