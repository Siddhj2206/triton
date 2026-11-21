//! The type scale. Jost for display, Google Sans Flex for UI, JetBrains Mono for
//! code. Icons are in [`crate::icon`]; the font files are in [`crate::fonts`].

use iced_core::Font;
use iced_core::font::{self, Weight};

use crate::tokens;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Family {
    /// Jost: clock, headings, OSD numbers.
    Display,
    /// Google Sans Flex: everything else.
    Ui,
    /// JetBrains Mono: code and key caps.
    Mono,
}

impl Family {
    pub const fn name(self) -> &'static str {
        match self {
            Family::Display => "Jost",
            Family::Ui => "Google Sans Flex",
            Family::Mono => "JetBrains Mono",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TypeStyle {
    pub size: f32,
    pub line_height: f32,
    pub weight: Weight,
    pub family: Family,
    /// Letter spacing as a fraction of `size`.
    pub tracking: f32,
    pub uppercase: bool,
}

impl TypeStyle {
    pub const DISPLAY: TypeStyle = tokens::TYPE_DISPLAY;
    pub const HEADLINE_L: TypeStyle = tokens::TYPE_HEADLINE_L;
    pub const HEADLINE_M: TypeStyle = tokens::TYPE_HEADLINE_M;
    pub const TITLE_L: TypeStyle = tokens::TYPE_TITLE_L;
    pub const TITLE_M: TypeStyle = tokens::TYPE_TITLE_M;
    pub const BODY_L: TypeStyle = tokens::TYPE_BODY_L;
    pub const BODY_M: TypeStyle = tokens::TYPE_BODY_M;
    pub const BODY_S: TypeStyle = tokens::TYPE_BODY_S;
    pub const LABEL_L: TypeStyle = tokens::TYPE_LABEL_L;
    pub const LABEL_S: TypeStyle = tokens::TYPE_LABEL_S;
    pub const MONO: TypeStyle = tokens::TYPE_MONO;

    pub const fn font(self) -> Font {
        Font {
            family: font::Family::Name(self.family.name()),
            weight: self.weight,
            ..Font::DEFAULT
        }
    }

    /// Line height as a multiple of the size, as iced's `LineHeight::Relative` takes.
    pub fn line_height_relative(self) -> f32 {
        self.line_height / self.size
    }
}
