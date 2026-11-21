//! Text in the type scale, and icons.

use iced_core::alignment::{Horizontal, Vertical};
use iced_core::text::{LineHeight, Shaping};
use iced_core::widget::text::{Catalog, Style};
use iced_core::{Color, Font};
use iced_widget::text::IntoFragment;
use triton_design::color::with_alpha;
use triton_design::{Family, Icon, IconStyle, Scheme, TypeStyle};

use crate::{Renderer, Theme};

pub type Text<'a> = iced_widget::Text<'a, Theme, Renderer>;

/// Which color role text takes. `Inherit` uses the color the container or button
/// sets, which is right for anything inside a component.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tone {
    #[default]
    Inherit,
    OnSurface,
    OnSurfaceVariant,
    /// Dark scheme only on materials: in light, primary is for fills, never text.
    Primary,
    Tertiary,
    Error,
    Warn,
    Disabled,
}

impl Tone {
    pub fn color(self, s: &Scheme) -> Option<Color> {
        Some(match self {
            Tone::Inherit => return None,
            Tone::OnSurface => s.on_surface,
            Tone::OnSurfaceVariant => s.on_surface_variant,
            Tone::Primary => s.primary,
            Tone::Tertiary => s.tertiary,
            Tone::Error => s.error,
            Tone::Warn => s.signal_warn,
            Tone::Disabled => with_alpha(
                s.on_surface,
                triton_design::color::STATE_LAYER.disabled_content,
            ),
        })
    }
}

impl Catalog for Theme {
    type Class<'a> = Tone;

    fn default<'a>() -> Self::Class<'a> {
        Tone::Inherit
    }

    fn style(&self, tone: &Tone) -> Style {
        Style {
            color: tone.color(&self.scheme),
        }
    }
}

/// Text in a type-scale role.
pub fn text<'a>(content: impl IntoFragment<'a>, style: TypeStyle) -> Text<'a> {
    iced_widget::text(content)
        .font(style.font())
        .size(style.size)
        .line_height(LineHeight::Relative(style.line_height_relative()))
}

/// The eyebrow: label-small, uppercase, primary.
pub fn eyebrow<'a>(content: &str) -> Text<'a> {
    let style = TypeStyle::LABEL_S;
    let content = if style.uppercase {
        content.to_uppercase()
    } else {
        content.to_owned()
    };
    // iced has no letter spacing; the +14% tracking is not drawn.
    text(content, style).class(Tone::Primary)
}

/// UI text at a size and weight the type scale does not name. Component metrics
/// from `components.css` use this; surfaces use [`text`].
pub(crate) fn ui<'a>(
    content: impl IntoFragment<'a>,
    size: f32,
    weight: iced_core::font::Weight,
) -> Text<'a> {
    iced_widget::text(content)
        .font(Font {
            family: iced_core::font::Family::Name(Family::Ui.name()),
            weight,
            ..Font::DEFAULT
        })
        .size(size)
        .line_height(LineHeight::Relative(1.3))
}

/// An icon, `size` logical px square, centered.
pub fn icon<'a>(icon: Icon, style: IconStyle, size: f32) -> Text<'a> {
    iced_widget::text(icon.codepoint().to_string())
        .font(Icon::font(style))
        .size(size)
        .line_height(LineHeight::Absolute(size.into()))
        .width(size)
        .height(size)
        .align_x(Horizontal::Center)
        .align_y(Vertical::Center)
        .shaping(Shaping::Basic)
}
