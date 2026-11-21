//! Containers: materials, islands, cards, list rows and the small labels.

use iced_core::{Background, Border, Color, Length, Padding, Shadow};
use iced_widget::container::{Catalog, Style};
use iced_widget::{Column, Container, Row, Space};
use triton_design::color::with_alpha;
use triton_design::material::ShadowToken;
use triton_design::shape::{concentric, list_corners};
use triton_design::{Icon, IconStyle, Material, RADII, SIZE};

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
    /// A floating material tightly wrapping one control whose corner radius is
    /// `inner`, `inset` px in. Its corner is concentric with the control's.
    Wrap {
        material: Material,
        inner: f32,
        inset: f32,
    },
    /// `.t-island`: a Glass pill in the bar.
    Island,
    /// `.t-pill`: a non-interactive Glass label (overview window names, hints).
    Pill,
    /// `.t-card`: Solid, 24 px corners.
    Card,
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
}

fn shadow(token: Option<ShadowToken>) -> Shadow {
    token.map_or_else(Shadow::default, ShadowToken::to_iced)
}

impl Catalog for Theme {
    type Class<'a> = Surface;

    fn default<'a>() -> Self::Class<'a> {
        Surface::None
    }

    fn style(&self, class: &Surface) -> Style {
        let s = &self.scheme;
        let v = self.variant;
        let fill = |m: Material| m.fill_adaptive(s, v, self.bright_backdrop);
        let solid = |c: Color, r: f32| Style {
            background: Some(Background::Color(c)),
            border: Border::default().rounded(r),
            ..Style::default()
        };
        // Glass and Thick float: shadow underneath. (The 1 px top highlight needs an
        // inset shadow, which iced lacks; it comes with the custom material widget.)
        let floating = |m: Material, r: f32| Style {
            text_color: Some(s.on_surface),
            background: Some(Background::Color(fill(m))),
            border: Border::default().rounded(r),
            shadow: shadow(m.shadow()),
            snap: true,
        };
        match *class {
            Surface::None => Style::default(),
            Surface::Material(m) => Style {
                text_color: Some(s.on_surface),
                ..floating(m, RADII.tiled)
            },
            Surface::Panel(m) => floating(m, RADII.floating_window),
            Surface::Wrap {
                material,
                inner,
                inset,
            } => floating(material, concentric(inner, inset)),
            Surface::Island => floating(Material::Glass, SIZE.bar_height / 2.0),
            // .t-pill: 36 high, radius 18.
            Surface::Pill => floating(Material::Glass, 18.0),
            Surface::Card => solid(s.surface_container_low, RADII.list_outer),
            Surface::Dialog => Style {
                shadow: shadow(Material::Thick.shadow()),
                ..solid(s.surface_container_low, RADII.floating_window)
            },
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
                let [tl, tr, br, bl] = list_corners(index, count);
                Style {
                    text_color: Some(if selected {
                        s.on_secondary_container
                    } else {
                        s.on_surface
                    }),
                    background: Some(Background::Color(bg)),
                    border: Border {
                        radius: iced_core::border::Radius {
                            top_left: tl,
                            top_right: tr,
                            bottom_right: br,
                            bottom_left: bl,
                        },
                        ..Border::default()
                    },
                    ..Style::default()
                }
            }
            Surface::IconDisc => Style {
                text_color: Some(s.on_secondary_container),
                ..solid(s.secondary_container, RADII.full)
            },
            // .t-kbd: 1 px outline at .35, radius 7.
            Surface::Kbd => Style {
                text_color: Some(s.on_surface),
                border: Border {
                    color: with_alpha(s.on_surface, 0.35),
                    width: 1.0,
                    radius: 7.0.into(),
                },
                ..Style::default()
            },
            // .t-tag: radius 10.
            Surface::Tag => Style {
                text_color: Some(s.on_tertiary_container),
                ..solid(s.tertiary_container, 10.0)
            },
        }
    }
}

pub type Panel<'a, M> = Container<'a, M, Theme, Renderer>;

/// Any content in a surface.
pub fn surface<'a, M: 'a>(content: impl Into<Element<'a, M>>, class: Surface) -> Panel<'a, M> {
    iced_widget::container(content).class(class)
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

/// A bar island: 56 high, content centered vertically.
pub fn island<'a, M: 'a>(content: impl Into<Element<'a, M>>) -> Panel<'a, M> {
    surface(content, Surface::Island)
        .height(SIZE.bar_height)
        .padding(Padding::from([0.0, 8.0]))
        .center_y(SIZE.bar_height)
}

/// `.t-card`: Solid, 24 px corners, padding 20 × 22.
pub fn card<'a, M: 'a>(content: impl Into<Element<'a, M>>) -> Panel<'a, M> {
    surface(content, Surface::Card).padding(Padding::from([20.0, 22.0]))
}

/// `.t-pill`: icon and label on Glass, 36 high.
pub fn pill<'a, M: 'a>(glyph: Icon, label: &'a str) -> Panel<'a, M> {
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
pub fn list<'a, M: 'a>(
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
        surface(
            row,
            Surface::ListRow {
                index,
                count,
                selected: item.selected,
                on_card,
            },
        )
        .width(Length::Fill)
        .padding(Padding::from([12.0, 20.0]))
        .into()
    });
    Column::with_children(rows).spacing(2)
}
