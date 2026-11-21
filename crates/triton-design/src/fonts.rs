//! The font files every Triton process loads, embedded so rendering is the same on
//! every machine and in tests. Rebuild with `mise run fonts`; licenses are in
//! `fonts/licenses/`.
//!
//! Each family ships as static instances at 400, 500 and 600. cosmic-text picks a
//! family's face only on an exact weight match, and fontdb sees a variable font as a
//! single weight, so variable files would make every other weight fall back to a
//! different family. Text fonts are OFL 1.1 instances of the upstream fonts; the
//! icon fonts are a modified Material Symbols Rounded (Apache-2.0).

macro_rules! font {
    ($file:literal) => {
        include_bytes!(concat!("../fonts/", $file)).as_slice()
    };
}

pub const JOST: [&[u8]; 3] = [
    font!("Jost-Regular.ttf"),
    font!("Jost-Medium.ttf"),
    font!("Jost-SemiBold.ttf"),
];
pub const GOOGLE_SANS_FLEX: [&[u8]; 3] = [
    font!("GoogleSansFlex-Regular.ttf"),
    font!("GoogleSansFlex-Medium.ttf"),
    font!("GoogleSansFlex-SemiBold.ttf"),
];
pub const JETBRAINS_MONO: [&[u8]; 3] = [
    font!("JetBrainsMono-Regular.ttf"),
    font!("JetBrainsMono-Medium.ttf"),
    font!("JetBrainsMono-SemiBold.ttf"),
];
pub const SYMBOLS: &[u8] = font!("TritonSymbols.ttf");
pub const SYMBOLS_FILLED: &[u8] = font!("TritonSymbols-Filled.ttf");

/// Every font, for iced's `Settings::fonts` or `font::load`.
pub const ALL: [&[u8]; 11] = [
    JOST[0],
    JOST[1],
    JOST[2],
    GOOGLE_SANS_FLEX[0],
    GOOGLE_SANS_FLEX[1],
    GOOGLE_SANS_FLEX[2],
    JETBRAINS_MONO[0],
    JETBRAINS_MONO[1],
    JETBRAINS_MONO[2],
    SYMBOLS,
    SYMBOLS_FILLED,
];
