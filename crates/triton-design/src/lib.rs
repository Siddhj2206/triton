//! Material 3 Futura, Triton's design language, as code.
//!
//! Every value comes from `tokens.json`, the token file of the design canvas, via
//! `build.rs`. Nothing here restates a number: to change the design, replace
//! `tokens.json` and rebuild. Every surface (shell, lock, greeter, settings) and every
//! generated theme (GTK CSS, niri rules) reads these types, so they cannot disagree.
//!
//! The crate has zero runtime dependencies: no renderer, no widgets, no Wayland.

pub mod color;
pub mod fonts;
pub mod icon;
pub mod material;
pub mod motion;
pub mod shape;
pub mod typography;
pub mod wave;

pub use color::{Color, Scheme, StateLayer, Variant};
pub use icon::{Icon, IconStyle};
pub use material::{Material, MaterialSpec, ShadowToken};
pub use motion::{Animated, Motion, Spring};
pub use shape::{Point, RADII, Radii, SIZE, SPACE, Size};
pub use typography::{Family, TypeStyle, Weight};

/// The constants generated from `tokens.json`.
mod tokens {
    use crate::color::{Color, Scheme, StateLayer};
    use crate::material::{MaterialSpec, ShadowToken};
    use crate::motion::Spring;
    use crate::shape::{Radii, Size};
    use crate::typography::{Family, TypeStyle, Weight};

    include!(concat!(env!("OUT_DIR"), "/tokens.rs"));
}
