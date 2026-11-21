//! Triton's iced theme. It implements iced's per-widget `Catalog` traits with
//! classes that are component variants, so a surface can ask for a "tonal button"
//! or a "Glass container" but cannot write its own style (ADR-0006). A widget whose
//! catalog is not implemented here does not compile against this theme, which is the
//! point: new widgets get a component first.

use iced_core::theme::{Base, Mode, Palette, Style};
use triton_design::{Scheme, Variant};

#[derive(Debug, Clone, PartialEq)]
pub struct Theme {
    pub scheme: Scheme,
    pub variant: Variant,
    /// Swap springs for 120 ms fades.
    pub reduce_motion: bool,
    /// The wallpaper under the bar is bright: Mica takes its adaptive extra tint.
    pub bright_backdrop: bool,
}

impl Theme {
    /// The canvas's reference scheme for `variant`. At runtime the scheme comes from
    /// the wallpaper.
    pub fn reference(variant: Variant) -> Theme {
        Theme {
            scheme: Scheme::reference(variant),
            variant,
            reduce_motion: false,
            bright_backdrop: false,
        }
    }

    pub fn dark() -> Theme {
        Theme::reference(Variant::Dark)
    }

    pub fn light() -> Theme {
        Theme::reference(Variant::Light)
    }
}

impl Default for Theme {
    fn default() -> Theme {
        Theme::dark()
    }
}

impl Base for Theme {
    fn default(preference: Mode) -> Theme {
        match preference {
            Mode::Light => Theme::light(),
            Mode::Dark | Mode::None => Theme::dark(),
        }
    }

    fn mode(&self) -> Mode {
        match self.variant {
            Variant::Dark => Mode::Dark,
            Variant::Light => Mode::Light,
        }
    }

    fn base(&self) -> Style {
        Style {
            background_color: self.scheme.surface,
            text_color: self.scheme.on_surface,
        }
    }

    fn palette(&self) -> Option<Palette> {
        let s = &self.scheme;
        Some(Palette {
            background: s.surface,
            text: s.on_surface,
            primary: s.primary,
            success: s.signal_info,
            warning: s.signal_warn,
            danger: s.error,
        })
    }

    fn name(&self) -> &str {
        match self.variant {
            Variant::Dark => "Triton Dark",
            Variant::Light => "Triton Light",
        }
    }
}
