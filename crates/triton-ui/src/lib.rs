//! Triton's component set: the canvas's Component index as iced widgets.
//!
//! Every surface (bar, panels, lock, greeter, settings) builds from these
//! constructors and the [`Theme`]. The styles are ports of
//! `reference/components.css` (the canvas's reference implementation); values come
//! from `triton-design`, never literals, except component metrics that the CSS
//! defines and the tokens do not (each one cites its CSS class).

pub mod bar;
pub mod button;
pub mod control;
pub mod field;
pub mod gallery;
pub mod measure;
pub mod menu;
mod paint;
pub mod press;
pub mod progress;
pub mod scroll;
pub mod slider;
pub mod surface;
pub mod text;
pub mod theme;

pub use button::{ButtonClass, IconKind, Kind, Segment, Size};
pub use surface::Surface;
pub use text::Tone;
pub use theme::Theme;

/// The renderer: tiny-skia here; the `triton` binary adds wgpu in front of it
/// (ADR-0007), which turns this into iced's wgpu-or-tiny-skia fallback renderer.
pub type Renderer = iced_renderer::Renderer;

/// An element built from Triton components.
pub type Element<'a, Message> = iced_core::Element<'a, Message, Theme, Renderer>;

/// The iced settings every Triton process and test starts from: the embedded fonts,
/// Google Sans Flex as the default, body-medium as the default size.
pub fn settings() -> iced_core::Settings {
    use triton_design::{Family, TypeStyle};
    iced_core::Settings {
        fonts: triton_design::fonts::ALL
            .iter()
            .map(|f| std::borrow::Cow::Borrowed(*f))
            .collect(),
        default_font: iced_core::Font::with_name(Family::Ui.name()),
        default_text_size: TypeStyle::BODY_M.size.into(),
        ..iced_core::Settings::default()
    }
}
