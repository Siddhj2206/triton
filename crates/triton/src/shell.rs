//! What the shell wants on screen, free of layer-shell types (ADR-0003).
//!
//! `desired` is a pure function of what niri reports, so lifecycle and hotplug are
//! tested without Wayland. The host turns the difference into surface operations.

use std::collections::BTreeMap;

use triton_design::SIZE;

/// What a surface is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Role {
    /// The left island: launcher, workspaces, focused window.
    IslandStart,
    /// The right island: clock, status, notifications.
    IslandEnd,
    /// The strip that reserves the bar's height.
    Zone,
}

/// One surface: a role on an output.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SurfaceKey {
    pub role: Role,
    pub output: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[expect(
    dead_code,
    reason = "the bar uses one value; panels and the lock use the rest"
)]
pub enum Layer {
    Background,
    Bottom,
    Top,
    Overlay,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "one per edge, as layer shell has it"
)]
pub struct Anchors {
    pub top: bool,
    pub bottom: bool,
    pub left: bool,
    pub right: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[expect(
    dead_code,
    reason = "the bar uses one value; panels and the lock use the rest"
)]
pub enum Keyboard {
    None,
    OnDemand,
    Exclusive,
}

/// How a surface sits on its output. Sizes are logical px; 0 stretches between
/// opposite anchors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurfaceSpec {
    pub namespace: &'static str,
    pub layer: Layer,
    pub anchors: Anchors,
    pub size: (u32, u32),
    pub exclusive_zone: i32,
    /// Top, right, bottom, left.
    pub margin: (i32, i32, i32, i32),
    pub keyboard: Keyboard,
    /// Only the top this many px take pointer input; `None` is the whole surface.
    /// Lets a shadow draw below a surface without catching clicks there.
    pub input_height: Option<u32>,
    /// Corner radius of the blur triton asks niri for (ext-background-effect),
    /// matching the surface's drawn shape. `None` is no background effect.
    pub blur_radius: Option<u32>,
}

/// The bar islands' layer-shell namespace; niri's layer rule rounds and shadows
/// them while the islands ask for their own blur (ADR-0004).
pub const BAR_NAMESPACE: &str = "triton-bar";
/// The bar's reserved strip; no layer rule matches it.
pub const BAR_ZONE_NAMESPACE: &str = "triton-bar-zone";

/// The islands sit `BAR_TOP` below the output's top edge, `BAR_SIDE` in from its
/// sides.
pub const BAR_TOP: f32 = 8.0;
pub const BAR_SIDE: f32 = 16.0;

/// Width of an island before it has been measured.
const UNMEASURED: u32 = 56;

/// One island, in a top corner, exactly its own size: niri clips and shadows a
/// layer surface as a whole, so the surface must be the pill and nothing more.
fn island(end: bool, width: u32) -> SurfaceSpec {
    SurfaceSpec {
        namespace: BAR_NAMESPACE,
        layer: Layer::Top,
        anchors: Anchors {
            top: true,
            left: !end,
            right: end,
            bottom: false,
        },
        size: (width.max(1), SIZE.bar_height as u32),
        // A corner-anchored surface cannot reserve space; the zone strip does.
        // -1: sit at the edge regardless of that strip's zone.
        exclusive_zone: -1,
        margin: (
            BAR_TOP as i32,
            if end { BAR_SIDE as i32 } else { 0 },
            0,
            if end { 0 } else { BAR_SIDE as i32 },
        ),
        keyboard: Keyboard::None,
        input_height: None,
        blur_radius: Some(SIZE.bar_height as u32 / 2),
    }
}

/// An invisible 1 px strip across the top that reserves the bar's height, so
/// windows start below the islands. It takes no input.
fn zone() -> SurfaceSpec {
    SurfaceSpec {
        namespace: BAR_ZONE_NAMESPACE,
        layer: Layer::Top,
        anchors: Anchors {
            top: true,
            left: true,
            right: true,
            bottom: false,
        },
        size: (0, 1),
        exclusive_zone: (BAR_TOP + SIZE.bar_height) as i32,
        margin: (0, 0, 0, 0),
        keyboard: Keyboard::None,
        input_height: Some(0),
        blur_radius: None,
    }
}

/// Every surface the shell wants, for the outputs niri has reported. `widths`
/// holds the islands' measured widths; an unmeasured island starts as a circle.
pub fn desired<'a>(
    outputs: impl IntoIterator<Item = &'a str>,
    widths: &BTreeMap<SurfaceKey, u32>,
) -> BTreeMap<SurfaceKey, SurfaceSpec> {
    let mut out = BTreeMap::new();
    for output in outputs {
        let key = |role| SurfaceKey {
            role,
            output: output.to_owned(),
        };
        out.insert(key(Role::Zone), zone());
        for (role, end) in [(Role::IslandStart, false), (Role::IslandEnd, true)] {
            let k = key(role);
            let width = widths.get(&k).copied().unwrap_or(UNMEASURED);
            out.insert(k, island(end, width));
        }
    }
    out
}
