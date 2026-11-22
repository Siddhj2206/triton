//! Shell domain state, events, and backend mode definitions.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::mpsc;

use serde::{Deserialize, Serialize};
use triton_design::color::{Scheme, Variant};
use triton_engine::{OutputInfo, Theme};

use crate::ipc::{IpcCommand, IpcResponse};
use crate::osd::{OsdPayload, OsdState};

/// Selects whether the daemon runs against the safe fake dev backend or live
/// system integrations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BackendMode {
    /// Safe dev backend with synthetic outputs, workspaces, window title, and
    /// status state. Never reads or writes host `~/.config/niri` or invokes
    /// host `gsettings`.
    #[default]
    Fake,
    /// Live system integrations (niri IPC, owned niri config, `gsettings`).
    RealServices,
}

impl BackendMode {
    /// Human-readable identifier used in status snapshots and logs.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Fake => "fake",
            Self::RealServices => "real-services",
        }
    }
}

/// Top bar presentation mode (design spec: Autohide, Pinned, Hidden).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BarMode {
    #[default]
    Autohide,
    Pinned,
    Hidden,
}

/// Declarative surface roles reconciled per connected output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum SurfaceRole {
    /// Per-output background wallpaper surface (`Layer::Background`).
    Background,
    /// Per-output top bar surface (`Layer::Top`).
    Bar,
    /// Single focused-output OSD overlay (`Layer::Overlay`).
    Osd,
}

impl SurfaceRole {
    /// Wayland layer-shell namespace matching `layer-rules.kdl` conventions.
    pub const fn namespace(self) -> &'static str {
        match self {
            Self::Background => "triton-wallpaper",
            Self::Bar => "triton-bar",
            Self::Osd => "triton-osd",
        }
    }

    /// Short role identifier for status snapshots.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Background => "background",
            Self::Bar => "bar",
            Self::Osd => "osd",
        }
    }
}

/// Stable `Copy` key identifying a connected Wayland output within `TritonShell`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct OutputKey(pub u32);

/// Workspace summary displayed by the Bar and updated via niri IPC (#15).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceInfo {
    pub id: u64,
    pub idx: u8,
    pub name: Option<String>,
    pub output: String,
    pub is_active: bool,
    pub is_focused: bool,
    pub is_urgent: bool,
    pub active_window_id: Option<u64>,
}

/// Focused window summary displayed by the Bar and used for fullscreen timing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FocusedWindow {
    pub id: u64,
    pub title: String,
    pub app_id: Option<String>,
    pub is_fullscreen: bool,
    pub output: Option<String>,
}

/// Aggregate system status summary rendered in the Bar status pill & bell.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusSummary {
    pub network_connected: bool,
    pub volume_percent: u8,
    pub muted: bool,
    pub battery_percent: Option<u8>,
    pub charging: bool,
    pub unread_notifications: u32,
}

impl Default for StatusSummary {
    fn default() -> Self {
        Self {
            network_connected: true,
            volume_percent: 65,
            muted: false,
            battery_percent: Some(84),
            charging: false,
            unread_notifications: 0,
        }
    }
}

/// Per-output wallpaper paths and adaptive Mica luminance state (#18).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct WallpaperState {
    pub per_output: BTreeMap<String, PathBuf>,
    pub default_path: Option<PathBuf>,
    /// Mean relative luminance (`0.0..=1.0`); values `> 0.6` trigger adaptive Mica alpha.
    pub mean_luminance: f32,
}

impl WallpaperState {
    /// Returns the active wallpaper path for `output`, falling back to `default_path`.
    pub fn path_for(&self, output: &str) -> Option<&PathBuf> {
        self.per_output.get(output).or(self.default_path.as_ref())
    }

    /// Returns `true` when the active wallpaper exceeds the bright-backdrop threshold (`0.6`).
    pub fn is_bright_backdrop(&self) -> bool {
        self.mean_luminance > 0.6
    }
}

/// Complete domain state owned by [`crate::TritonShell`].
#[derive(Debug, Clone)]
pub struct ShellState {
    pub backend: BackendMode,
    pub outputs: Vec<OutputInfo>,
    pub focused_output: Option<String>,
    pub bar_mode: BarMode,
    pub last_visible_bar_mode: BarMode,
    pub workspaces: Vec<WorkspaceInfo>,
    pub focused_window: Option<FocusedWindow>,
    pub status: StatusSummary,
    pub osd: Option<OsdState>,
    pub wallpaper: WallpaperState,
    pub theme: Theme,
    pub reduce_motion: bool,
}

impl ShellState {
    /// Safe fake dev backend state with synthetic output, workspaces, focused
    /// window, and status summary. Never touches host config or `gsettings`.
    pub fn fake_dev() -> Self {
        let output_name = "eDP-1".to_owned();
        let outputs = vec![OutputInfo::new(output_name.clone(), (1920, 1080))];
        let workspaces = (1..=4_u8)
            .map(|idx| WorkspaceInfo {
                id: u64::from(idx),
                idx,
                name: None,
                output: output_name.clone(),
                is_active: idx == 1,
                is_focused: idx == 1,
                is_urgent: false,
                active_window_id: (idx <= 2).then_some(u64::from(idx) * 10),
            })
            .collect();
        let focused_window = Some(FocusedWindow {
            id: 10,
            title: "Triton — Dev Session".to_owned(),
            app_id: Some("triton.dev".to_owned()),
            is_fullscreen: false,
            output: Some(output_name.clone()),
        });
        Self {
            backend: BackendMode::Fake,
            outputs,
            focused_output: Some(output_name),
            bar_mode: BarMode::Autohide,
            last_visible_bar_mode: BarMode::Autohide,
            workspaces,
            focused_window,
            status: StatusSummary::default(),
            osd: None,
            wallpaper: WallpaperState::default(),
            theme: Theme::dark(),
            reduce_motion: false,
        }
    }

    /// Live-services initial state: starts with an empty output inventory that
    /// Wayland (`on_outputs_changed`) and niri IPC (#15) populate.
    pub fn real_services() -> Self {
        Self {
            backend: BackendMode::RealServices,
            outputs: Vec::new(),
            focused_output: None,
            bar_mode: BarMode::Autohide,
            last_visible_bar_mode: BarMode::Autohide,
            workspaces: Vec::new(),
            focused_window: None,
            status: StatusSummary::default(),
            osd: None,
            wallpaper: WallpaperState::default(),
            theme: Theme::dark(),
            reduce_motion: false,
        }
    }

    /// Construct initial state for `backend`.
    pub fn for_backend(backend: BackendMode) -> Self {
        match backend {
            BackendMode::Fake => Self::fake_dev(),
            BackendMode::RealServices => Self::real_services(),
        }
    }

    /// Set the bar mode, remembering the last visible mode for `Hidden` toggle.
    pub fn set_bar_mode(&mut self, mode: BarMode) {
        if mode != BarMode::Hidden {
            self.last_visible_bar_mode = mode;
        }
        self.bar_mode = mode;
    }

    /// Toggle between `BarMode::Hidden` and the last visible mode (`Super+B`).
    pub fn toggle_bar_hidden(&mut self) -> BarMode {
        if self.bar_mode == BarMode::Hidden {
            let restored = match self.last_visible_bar_mode {
                BarMode::Hidden => BarMode::Autohide,
                other => other,
            };
            self.bar_mode = restored;
        } else {
            self.last_visible_bar_mode = self.bar_mode;
            self.bar_mode = BarMode::Hidden;
        }
        self.bar_mode
    }

    /// Returns the name of the currently focused output, falling back to the
    /// first connected output if none is explicitly focused.
    pub fn target_output_name(&self) -> Option<&str> {
        if let Some(ref focused) = self.focused_output
            && self.outputs.iter().any(|o| o.name == *focused)
        {
            return Some(focused.as_str());
        }
        self.outputs.first().map(|o| o.name.as_str())
    }
}

/// Summary of one reconciled surface in [`ShellStatusSnapshot`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceSummary {
    pub role: String,
    pub output: String,
    pub namespace: String,
}

/// Serializable daemon status returned by `triton msg status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShellStatusSnapshot {
    pub backend: String,
    pub outputs: Vec<String>,
    pub focused_output: Option<String>,
    pub bar_mode: BarMode,
    pub variant: String,
    pub reduce_motion: bool,
    pub osd_active: bool,
    pub active_surfaces: Vec<SurfaceSummary>,
    pub wallpaper: Option<String>,
    pub workspace_count: usize,
    pub focused_window_title: Option<String>,
}

/// External typed events delivered into `TritonShell` via
/// `triton_engine::EventSender<ShellEvent>`.
#[derive(Debug)]
#[non_exhaustive]
pub enum ShellEvent {
    /// Structured IPC command from `triton msg` with a one-shot reply channel.
    Ipc {
        command: IpcCommand,
        reply: mpsc::Sender<IpcResponse>,
    },
    /// Change the top bar presentation mode.
    SetBarMode(BarMode),
    /// Toggle bar visibility (`Hidden` <-> last visible mode).
    ToggleBarHidden,
    /// Show or update the OSD pill on `output` (or the focused output if `None`).
    ShowOsd {
        payload: OsdPayload,
        output: Option<String>,
    },
    /// Dismiss the active OSD pill immediately.
    HideOsd,
    /// Request a wallpaper change globally or for a specific output.
    SetWallpaper {
        path: PathBuf,
        output: Option<String>,
    },
    /// Apply a theme update (scheme, variant, and/or adaptive Mica luminance).
    ApplyTheme {
        scheme: Option<Box<Scheme>>,
        variant: Option<Variant>,
        mean_luminance: Option<f32>,
    },
    /// Toggle or set reduced-motion accessibility mode.
    SetReduceMotion(bool),
    /// Output inventory changed (e.g. from a background watcher or test harness).
    OutputsChanged(Vec<OutputInfo>),
    /// Focused output changed (from niri IPC #15).
    FocusedOutputChanged(Option<String>),
    /// Workspace list updated (from niri IPC #15).
    WorkspacesUpdated(Vec<WorkspaceInfo>),
    /// Focused window updated (from niri IPC #15).
    FocusedWindowChanged(Option<FocusedWindow>),
    /// Shut down the daemon cleanly.
    Quit,
}

/// Helper to format a [`Variant`] as a wire-friendly lowercase string.
pub const fn variant_as_str(variant: Variant) -> &'static str {
    match variant {
        Variant::Dark => "dark",
        Variant::Light => "light",
    }
}
