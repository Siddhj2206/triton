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
    /// The pinned bar: one full-width Mica strip (`BarStates` board).
    Strip,
    /// The invisible 2 px top edge that wakes the left island in autohide.
    RevealStart,
    /// The same for the right island.
    RevealEnd,
    /// The transparent click-catcher behind a modal panel. Declared before the
    /// modal roles so it maps first and stacks below them within the layer.
    Scrim,
    /// The launcher panel (`Launcher` board).
    Launcher,
    /// The notification center (`Notifications` board).
    Notifications,
    /// The wallpaper image on the background layer (`Wallpaper` board).
    Wallpaper,
    /// The transient pill above the bottom edge (`OSD` board).
    Osd,
    /// The top-right toast stack (`Notifications` board).
    Toasts,
    /// The quick settings panel (`QuickSettings` board).
    QuickSettings,
    /// The centered power menu (`Power` board).
    PowerMenu,
    /// The polkit authentication dialog (solid, centered).
    Polkit,
}

impl Role {
    /// Whether this surface hugs the output's right edge.
    fn end(self) -> bool {
        matches!(self, Self::IslandEnd | Self::RevealEnd)
    }
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
    /// Corner radius of the blur triton asks niri for (ext-background-effect),
    /// matching the surface's drawn shape. `None` is no background effect.
    pub blur_radius: Option<u32>,
    /// Whether pointer events fall through to what is below (the OSD: "Focus:
    /// never takes it. Pointer events pass through"). Create-time only.
    pub input_passthrough: bool,
}

/// The bar islands' layer-shell namespace; niri's layer rule rounds and shadows
/// them while the islands ask for their own blur (ADR-0004).
pub const BAR_NAMESPACE: &str = "triton-bar";
/// The pinned bar strip; no layer rule matches it (square, unshadowed chrome).
pub const BAR_STRIP_NAMESPACE: &str = "triton-bar-strip";
/// The autohide reveal edge; no layer rule matches it (transparent input strip).
pub const BAR_REVEAL_NAMESPACE: &str = "triton-bar-reveal";
/// The modal scrim; no layer rule matches it (transparent input surface).
pub const SCRIM_NAMESPACE: &str = "triton-scrim";
/// The launcher's namespace; a layer rule rounds and shadows it.
pub const LAUNCHER_NAMESPACE: &str = "triton-launcher";
/// The notification center's namespace; a layer rule rounds and shadows it.
pub const NOTIFICATIONS_NAMESPACE: &str = "triton-notifications";
/// The wallpaper's namespace; background layer, no rule matches it.
pub const WALLPAPER_NAMESPACE: &str = "triton-wallpaper";
/// The OSD's namespace; a layer rule rounds and shadows it.
pub const OSD_NAMESPACE: &str = "triton-osd";
/// The toast stack's namespace; a layer rule rounds and shadows it.
pub const TOASTS_NAMESPACE: &str = "triton-toasts";
/// Quick settings' namespace; a layer rule rounds and shadows it.
pub const QUICK_SETTINGS_NAMESPACE: &str = "triton-quicksettings";
/// The power menu's namespace; a layer rule rounds and shadows it.
pub const POWER_NAMESPACE: &str = "triton-power";
/// The polkit dialog's namespace; a layer rule rounds and shadows it.
pub const POLKIT_NAMESPACE: &str = "triton-polkit";

/// The islands sit `BAR_TOP` below the output's top edge, `BAR_SIDE` in from its
/// sides.
pub const BAR_TOP: f32 = 8.0;
pub const BAR_SIDE: f32 = 16.0;

/// Width of an island before it has been measured.
const UNMEASURED: u32 = 56;

/// One island, in a top corner, exactly its own size: niri clips and shadows a
/// layer surface as a whole, so the surface must be the pill and nothing more.
fn island(role: Role, width: u32) -> SurfaceSpec {
    let end = role.end();
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
        // A corner-anchored surface cannot reserve space; the pinned strip's
        // own zone does. -1: sit at the edge regardless of it.
        exclusive_zone: -1,
        margin: (
            BAR_TOP as i32,
            if end { BAR_SIDE as i32 } else { 0 },
            0,
            if end { 0 } else { BAR_SIDE as i32 },
        ),
        keyboard: Keyboard::None,
        blur_radius: Some(SIZE.bar_height as u32 / 2),
        input_passthrough: false,
    }
}

/// The launcher panel, fixed at the board's size for now: 900 wide, centered.
/// Real result lists will drive the height through measurement later; 700
/// fits the placeholder content (field + hits + hints) without clipping.
pub const LAUNCHER_SIZE: (u32, u32) = (900, 700);

/// The pinned bar: a square Mica strip edge to edge. Its exclusive zone is the
/// reservation — windows start below it (`BarStates`: "Docked").
fn strip() -> SurfaceSpec {
    SurfaceSpec {
        namespace: BAR_STRIP_NAMESPACE,
        layer: Layer::Top,
        anchors: Anchors {
            top: true,
            left: true,
            right: true,
            bottom: false,
        },
        size: (0, SIZE.bar_height as u32),
        exclusive_zone: SIZE.bar_height as i32,
        margin: (0, 0, 0, 0),
        keyboard: Keyboard::None,
        // Rectangle region: whole-surface blur suits a strip.
        blur_radius: Some(0),
        input_passthrough: false,
    }
}

/// The 2 px top edge over an island's footprint that wakes it in autohide
/// (`BarStates`: "the only trace is an invisible 2 px strip"). One edge per
/// island so they reveal independently. It reserves nothing and stays mapped
/// while the island is up so hover state stays continuous.
fn reveal(role: Role, width: u32) -> SurfaceSpec {
    let end = role.end();
    SurfaceSpec {
        namespace: BAR_REVEAL_NAMESPACE,
        layer: Layer::Top,
        anchors: Anchors {
            top: true,
            left: !end,
            right: end,
            bottom: false,
        },
        size: (width.max(1), 2),
        exclusive_zone: 0,
        margin: (0, 0, 0, 0),
        keyboard: Keyboard::None,
        blur_radius: None,
        input_passthrough: false,
    }
}

/// The transparent click-catcher behind a modal panel: covers its output so a
/// press anywhere outside the panel dismisses it (ADR-0003).
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
        blur_radius: None,
        input_passthrough: false,
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
        blur_radius: Some(triton_design::shape::RADII.floating_window as u32),
        input_passthrough: false,
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
        blur_radius: Some(triton_design::shape::RADII.floating_window as u32),
        input_passthrough: false,
    }
}

/// The wallpaper: the whole output on the background layer. No blur, no input —
/// it is the backdrop everything else floats over.
fn wallpaper() -> SurfaceSpec {
    SurfaceSpec {
        namespace: WALLPAPER_NAMESPACE,
        layer: Layer::Background,
        anchors: Anchors {
            top: true,
            bottom: true,
            left: true,
            right: true,
        },
        size: (0, 0),
        exclusive_zone: -1,
        margin: (0, 0, 0, 0),
        keyboard: Keyboard::None,
        blur_radius: None,
        input_passthrough: true,
    }
}

/// The OSD (`OSD` board): a Glass pill centered 48 px above the bottom edge.
/// Anchored only at the bottom so the compositor centers it horizontally.
/// Pointer-transparent and never focused.
fn osd(width: u32, height: u32) -> SurfaceSpec {
    SurfaceSpec {
        namespace: OSD_NAMESPACE,
        layer: Layer::Overlay,
        anchors: Anchors {
            bottom: true,
            ..Anchors::default()
        },
        size: (width.max(1), height.max(1)),
        exclusive_zone: -1,
        margin: (0, 0, 48, 0),
        keyboard: Keyboard::None,
        blur_radius: Some(32),
        input_passthrough: true,
    }
}

/// The toast stack (`Notifications` board): one top-right column holding up to
/// four cards and the overflow chip. Its top edge clears the bar, like the
/// notification center's.
fn toasts(height: u32, bar_shown: bool) -> SurfaceSpec {
    let top = if bar_shown {
        (BAR_TOP + SIZE.bar_height + 12.0) as i32
    } else {
        20
    };
    SurfaceSpec {
        namespace: TOASTS_NAMESPACE,
        layer: Layer::Overlay,
        anchors: Anchors {
            top: true,
            right: true,
            bottom: false,
            left: false,
        },
        size: (400, height.max(1)),
        exclusive_zone: -1,
        margin: (top, 20, 0, 0),
        keyboard: Keyboard::None,
        blur_radius: Some(triton_design::shape::RADII.floating_window as u32),
        input_passthrough: false,
    }
}

/// Quick settings (`QuickSettings` board): a top-right Glass panel, 420 wide,
/// as tall as its page. Grabs the keyboard while open (Esc, drill-in nav).
fn quick_settings(height: u32, bar_shown: bool) -> SurfaceSpec {
    let top = if bar_shown {
        (BAR_TOP + SIZE.bar_height + 12.0) as i32
    } else {
        20
    };
    SurfaceSpec {
        namespace: QUICK_SETTINGS_NAMESPACE,
        layer: Layer::Overlay,
        anchors: Anchors {
            top: true,
            right: true,
            bottom: false,
            left: false,
        },
        size: (420, height.max(1)),
        exclusive_zone: -1,
        margin: (top, 20, 0, 0),
        keyboard: Keyboard::Exclusive,
        blur_radius: Some(triton_design::shape::RADII.floating_window as u32),
        input_passthrough: false,
    }
}

/// A centered modal of measured size: the power menu (`Power` board) and the
/// polkit dialog. Anchored to no edge, so the compositor centers it.
fn centered(namespace: &'static str, width: u32, height: u32) -> SurfaceSpec {
    SurfaceSpec {
        namespace,
        layer: Layer::Overlay,
        anchors: Anchors::default(),
        size: (width.max(1), height.max(1)),
        exclusive_zone: -1,
        margin: (0, 0, 0, 0),
        keyboard: Keyboard::Exclusive,
        blur_radius: Some(triton_design::shape::RADII.floating_window as u32),
        input_passthrough: false,
    }
}

/// How the bar presents (`BarStates` board). The user picks a mode in settings
/// later; `triton msg bar mode …` switches it for now.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum BarMode {
    /// Two floating Glass islands that hide until the 2 px top edge (or
    /// `bar toggle`) wakes them. Nothing is reserved; windows use the whole
    /// screen and the islands cover what is under them.
    #[default]
    AutoHide,
    /// A square, full-width Mica strip with an exclusive zone — windows start
    /// below it.
    Pinned,
    /// Nothing until `bar toggle`; no reveal edge.
    Hidden,
}

/// The modal surfaces open on an output, if any. One field per panel; the scrim
/// comes free while any of them is set.
#[derive(Debug, Default, Clone, Copy)]
pub struct Modals<'a> {
    pub launcher: Option<&'a str>,
    pub notifications: Option<&'a str>,
    pub quick_settings: Option<&'a str>,
    /// The centered power menu (`Power` board) and the polkit dialog share the
    /// centered-modal slot: at most one is ever up.
    pub centered: Option<Centered<'a>>,
}

/// Which centered modal is up, on which output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Centered<'a> {
    Power(&'a str),
    Polkit(&'a str),
}

impl<'a> Centered<'a> {
    pub fn output(self) -> &'a str {
        match self {
            Self::Power(o) | Self::Polkit(o) => o,
        }
    }
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

/// The island's reveal edge covers its side margin plus its own measured width,
/// so the whole footprint wakes it.
fn reveal_width(edge: Role, sizes: &Sizes, key: impl Fn(Role) -> SurfaceKey) -> SurfaceSpec {
    let island = if edge.end() {
        Role::IslandEnd
    } else {
        Role::IslandStart
    };
    let width = sizes
        .widths
        .get(&key(island))
        .copied()
        .unwrap_or(UNMEASURED);
    reveal(edge, width + BAR_SIDE as u32)
}

/// Every surface the shell wants, for the outputs niri has reported. `sizes`
/// holds the measured extents of content-sized surfaces. `modals` names the
/// output each open panel lives on, if any. `mode` is the bar mode; `bar_shown`
/// reports whether an island is up (ignored when pinned — the strip always is).
pub fn desired<'a>(
    outputs: impl IntoIterator<Item = &'a str>,
    sizes: &Sizes,
    modals: &Modals<'a>,
    mode: BarMode,
    bar_shown: impl Fn(&SurfaceKey) -> bool,
) -> BTreeMap<SurfaceKey, SurfaceSpec> {
    let mut out = BTreeMap::new();
    for output in outputs {
        let key = |role| SurfaceKey {
            role,
            output: output.to_owned(),
        };
        if mode == BarMode::Pinned {
            out.insert(key(Role::Strip), strip());
        } else {
            for (island_role, edge) in [
                (Role::IslandStart, Role::RevealStart),
                (Role::IslandEnd, Role::RevealEnd),
            ] {
                if mode == BarMode::AutoHide {
                    out.insert(key(edge), reveal_width(edge, sizes, key));
                }
                let k = key(island_role);
                if bar_shown(&k) {
                    let width = sizes.widths.get(&k).copied().unwrap_or(UNMEASURED);
                    out.insert(k, island(island_role, width));
                }
            }
        }
        if modals.launcher == Some(output)
            || modals.notifications == Some(output)
            || modals.quick_settings == Some(output)
            || modals.centered.is_some_and(|c| c.output() == output)
        {
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
            // The panel sits top-right: it clears the right island's state.
            let up = mode == BarMode::Pinned || bar_shown(&key(Role::IslandEnd));
            out.insert(k, notifications(height, up));
        }
        if modals.quick_settings == Some(output) {
            let k = key(Role::QuickSettings);
            let height = sizes
                .heights
                .get(&k)
                .copied()
                .unwrap_or(UNMEASURED_PANEL_HEIGHT);
            let up = mode == BarMode::Pinned || bar_shown(&key(Role::IslandEnd));
            out.insert(k, quick_settings(height, up));
        }
        if modals.centered.is_some_and(|c| c.output() == output) {
            let (role, namespace) = match modals.centered {
                Some(Centered::Power(_)) => (Role::PowerMenu, POWER_NAMESPACE),
                Some(Centered::Polkit(_)) => (Role::Polkit, POLKIT_NAMESPACE),
                None => unreachable!("checked above"),
            };
            let k = key(role);
            let height = sizes
                .heights
                .get(&k)
                .copied()
                .unwrap_or(UNMEASURED_PANEL_HEIGHT);
            let width = sizes.widths.get(&k).copied().unwrap_or(640);
            out.insert(k, centered(namespace, width, height));
        }
    }
    out
}

/// The transient surfaces the shell wants beyond `desired`'s modals: the
/// wallpaper on every output, plus the OSD and toast stack on `active`.
/// They are not modals (no scrim, no focus) so they live in their own pass —
/// the app merges the two maps.
pub fn ambient<'a>(
    outputs: impl IntoIterator<Item = &'a str>,
    sizes: &Sizes,
    active: Option<&'a str>,
    has_osd: bool,
    has_toasts: bool,
    bar_shown: impl Fn(&SurfaceKey) -> bool,
    pinned: bool,
) -> BTreeMap<SurfaceKey, SurfaceSpec> {
    let mut out = BTreeMap::new();
    for output in outputs {
        let key = |role| SurfaceKey {
            role,
            output: output.to_owned(),
        };
        out.insert(key(Role::Wallpaper), wallpaper());
        let here = active == Some(output);
        if here && has_osd {
            let k = key(Role::Osd);
            let w = sizes.widths.get(&k).copied().unwrap_or(320);
            let h = sizes.heights.get(&k).copied().unwrap_or(64);
            out.insert(k, osd(w, h));
        }
        if here && has_toasts {
            let k = key(Role::Toasts);
            let h = sizes.heights.get(&k).copied().unwrap_or(64);
            let up = pinned || bar_shown(&key(Role::IslandEnd));
            out.insert(k, toasts(h, up));
        }
    }
    out
}
