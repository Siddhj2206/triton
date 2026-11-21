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
    /// The dim behind a modal panel. Declared before the modal roles so the
    /// scrim maps first and stacks below them within the overlay layer.
    Scrim,
    /// The launcher panel (`Launcher` board).
    Launcher,
    /// The notification center (`Notifications` board).
    Notifications,
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
/// The modal scrim; no layer rule matches it (transparent input surface).
pub const SCRIM_NAMESPACE: &str = "triton-scrim";
/// The launcher's namespace; a layer rule rounds and shadows it.
pub const LAUNCHER_NAMESPACE: &str = "triton-launcher";
/// The notification center's namespace; a layer rule rounds and shadows it.
pub const NOTIFICATIONS_NAMESPACE: &str = "triton-notifications";

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

/// The launcher panel, fixed at the board's size for now: 900 wide, centered.
/// Real result lists will drive the height through measurement later; 700
/// fits the placeholder content (field + hits + hints) without clipping.
pub const LAUNCHER_SIZE: (u32, u32) = (900, 700);

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

/// The modal scrim behind the launcher: covers its output so a press anywhere
/// outside the panel dismisses it (ADR-0003). Transparent — modal by input.
fn scrim() -> SurfaceSpec {
    SurfaceSpec {
        namespace: SCRIM_NAMESPACE,
        layer: Layer::Overlay,
        anchors: Anchors {
            top: true,
            bottom: true,
            left: true,
            right: true,
        },
        size: (0, 0),
        // Auto: covers the whole output including exclusive areas, so a press
        // over the bar strip dismisses the modal too.
        exclusive_zone: -1,
        margin: (0, 0, 0, 0),
        keyboard: Keyboard::None,
        input_height: None,
        blur_radius: None,
    }
}

/// The launcher: a Thick Glass panel, modal over the scrim. Anchored to no edge,
/// so the compositor centers it; it grabs the keyboard while open.
fn launcher() -> SurfaceSpec {
    SurfaceSpec {
        namespace: LAUNCHER_NAMESPACE,
        layer: Layer::Overlay,
        anchors: Anchors::default(),
        size: LAUNCHER_SIZE,
        exclusive_zone: -1,
        margin: (0, 0, 0, 0),
        keyboard: Keyboard::Exclusive,
        input_height: None,
        blur_radius: Some(triton_design::shape::RADII.floating_window as u32),
    }
}

/// The notification center (`Notifications` board): a 440 px Glass column at
/// the top-right, as tall as its content. Its top edge clears the bar when the
/// bar is up and pulls to the output's top when autohide has it hidden.
fn notifications(height: u32, bar_shown: bool) -> SurfaceSpec {
    let top = if bar_shown {
        // The islands' bottom edge is BAR_TOP + bar_height; a gap below.
        (BAR_TOP + SIZE.bar_height + 12.0) as i32
    } else {
        20
    };
    SurfaceSpec {
        namespace: NOTIFICATIONS_NAMESPACE,
        layer: Layer::Overlay,
        anchors: Anchors {
            top: true,
            right: true,
            bottom: false,
            left: false,
        },
        size: (440, height.max(1)),
        exclusive_zone: -1,
        margin: (top, 20, 0, 0),
        keyboard: Keyboard::Exclusive,
        input_height: None,
        blur_radius: Some(triton_design::shape::RADII.floating_window as u32),
    }
}

/// The modal surfaces open on an output, if any. One field per panel; the scrim
/// comes free while any of them is set.
#[derive(Debug, Default, Clone, Copy)]
pub struct Modals<'a> {
    pub launcher: Option<&'a str>,
    pub notifications: Option<&'a str>,
}

/// The measured extents of content-sized surfaces: the islands by width, the
/// notification center by height. Unmeasured surfaces get a sensible seed.
#[derive(Debug, Default)]
pub struct Sizes {
    pub widths: BTreeMap<SurfaceKey, u32>,
    pub heights: BTreeMap<SurfaceKey, u32>,
}

/// Seed height for the center before its first measure lands.
const UNMEASURED_PANEL_HEIGHT: u32 = 96;

/// Every surface the shell wants, for the outputs niri has reported. `sizes`
/// holds the measured extents of content-sized surfaces. `modals` names the
/// output each open panel lives on, if any. `bar_shown` reports whether the bar
/// is visible on an output — pinned or autohide-revealed — so modals can clear
/// it (autohide feeds this once it exists; today the bar is always up).
pub fn desired<'a>(
    outputs: impl IntoIterator<Item = &'a str>,
    sizes: &Sizes,
    modals: &Modals<'a>,
    bar_shown: impl Fn(&str) -> bool,
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
            let width = sizes.widths.get(&k).copied().unwrap_or(UNMEASURED);
            out.insert(k, island(end, width));
        }
        if modals.launcher == Some(output) || modals.notifications == Some(output) {
            out.insert(key(Role::Scrim), scrim());
        }
        if modals.launcher == Some(output) {
            out.insert(key(Role::Launcher), launcher());
        }
        if modals.notifications == Some(output) {
            let k = key(Role::Notifications);
            let height = sizes
                .heights
                .get(&k)
                .copied()
                .unwrap_or(UNMEASURED_PANEL_HEIGHT);
            out.insert(k, notifications(height, bar_shown(output)));
        }
    }
    out
}
