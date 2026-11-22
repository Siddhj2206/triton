//! `TritonShell`: single-process `ShellApp` host with declarative
//! `(SurfaceRole, OutputName)` surface reconciliation.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tracing::warn;
use triton_design::color::{Scheme, Variant};
use triton_design::icon::{Icon, IconStyle};
use triton_design::material::Material;
use triton_design::shape::{RADII, SIZE, SPACE};
use triton_design::typography::TypeStyle;
use triton_engine::{
    AppCommand, Element, EventSender, ImageStore, NodeKey, OutputInfo, Rgba, ShellApp,
    SurfaceAnchor, SurfaceKeyboard, SurfaceLayer, SurfaceSize, SurfaceSpec, Theme, ViewContext,
    icon, row, text,
};
use triton_kit::{
    PillStatus, Progress, island, panel, pill, shadow_gutter, status_pill, wavy_progress,
};

use crate::ipc::{BarCommand, IpcCommand, IpcResponse, OsdCommand, ThemeCommand};
use crate::state::{
    BackendMode, BarMode, OsdPayload, OsdState, OutputKey, ShellEvent, ShellState,
    ShellStatusSnapshot, SurfaceRole, SurfaceSummary, variant_as_str,
};
use crate::wallpaper::{WallpaperImage, WallpaperSet, crossfade_duration, view_background};

/// Timer tag for dismissing the OSD pill after its hold period.
pub const TIMER_OSD_HIDE: u32 = 1;

/// Timer tag for releasing outgoing wallpaper textures once a crossfade settles.
pub const TIMER_WALLPAPER_SETTLE: u32 = 2;

/// Default OSD hold duration (`docs/design-spec.md`: 1.2 s).
pub const OSD_HOLD_DURATION: Duration = Duration::from_millis(1200);

/// Default logical width used when an output does not report a logical size yet.
const DEFAULT_OUTPUT_WIDTH: u32 = 1920;

/// Stable max extent height for the floating autohide bar (8 px top offset +
/// 56 px bar height + `ShadowToken::FLOATING` top/bottom gutters).
const BAR_AUTOHIDE_EXTENT_H: u32 = 224;

/// Stable max extent dimensions for the morph-in-place Glass OSD pill.
const OSD_EXTENT_W: u32 = 420;
const OSD_EXTENT_H: u32 = 224;

/// Copyable surface identifier combining a [`SurfaceRole`] with a stable
/// [`OutputKey`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SurfaceId {
    pub role: SurfaceRole,
    pub output: OutputKey,
}

/// UI interaction messages emitted by `triton-kit` widgets inside `TritonShell`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ShellMessage {
    /// No-op placeholder message.
    None,
    /// User clicked a workspace pill in the Bar.
    FocusWorkspace(u64),
    /// User clicked the Bar status pill.
    StatusPillClicked,
}

/// Main Triton desktop shell application implementing [`ShellApp`].
#[derive(Debug)]
pub struct TritonShell {
    state: ShellState,
    name_to_key: HashMap<String, OutputKey>,
    key_to_name: HashMap<OutputKey, String>,
    next_output_key: u32,
    wallpapers: WallpaperSet,
    event_sender: Option<EventSender<ShellEvent>>,
}

impl TritonShell {
    /// Construct a new shell daemon in the given [`BackendMode`].
    pub fn new(backend: BackendMode) -> Self {
        let state = ShellState::for_backend(backend);
        let mut shell = Self {
            state,
            name_to_key: HashMap::new(),
            key_to_name: HashMap::new(),
            next_output_key: 1,
            wallpapers: WallpaperSet::default(),
            event_sender: None,
        };
        let initial_outputs = shell.state.outputs.clone();
        shell.sync_output_registry(&initial_outputs);
        shell
    }

    /// Install the external event sender so wallpaper decodes can be spawned
    /// off-thread and delivered back as [`ShellEvent`]s.
    pub fn set_event_sender(&mut self, sender: EventSender<ShellEvent>) {
        self.event_sender = Some(sender);
    }

    /// Borrow the wallpaper registry (decoded cache + per-output fades).
    pub fn wallpapers(&self) -> &WallpaperSet {
        &self.wallpapers
    }

    /// Borrow the current [`ShellState`].
    pub fn state(&self) -> &ShellState {
        &self.state
    }

    /// Mutably borrow the current [`ShellState`].
    pub fn state_mut(&mut self) -> &mut ShellState {
        &mut self.state
    }

    /// Resolve an [`OutputKey`] back to its compositor output name.
    pub fn output_name(&self, key: OutputKey) -> Option<&str> {
        self.key_to_name.get(&key).map(String::as_str)
    }

    /// Resolve a [`SurfaceId`] to its `(SurfaceRole, OutputName)` pair.
    pub fn surface_descriptor(&self, id: SurfaceId) -> Option<(SurfaceRole, &str)> {
        self.output_name(id.output).map(|name| (id.role, name))
    }

    /// Build a serializable status snapshot for `triton msg status`.
    pub fn status_snapshot(&self) -> ShellStatusSnapshot {
        let active_surfaces = self
            .surfaces()
            .into_iter()
            .filter_map(|(id, spec)| {
                let (role, output) = self.surface_descriptor(id)?;
                Some(SurfaceSummary {
                    role: role.as_str().to_owned(),
                    output: output.to_owned(),
                    namespace: spec.namespace,
                })
            })
            .collect();

        let wallpaper = self
            .state
            .target_output_name()
            .and_then(|out| self.state.wallpaper.path_for(out))
            .or(self.state.wallpaper.default_path.as_ref())
            .map(|p| p.display().to_string());

        ShellStatusSnapshot {
            backend: self.state.backend.as_str().to_owned(),
            outputs: self.state.outputs.iter().map(|o| o.name.clone()).collect(),
            focused_output: self.state.focused_output.clone(),
            bar_mode: self.state.bar_mode,
            variant: variant_as_str(self.state.theme.variant).to_owned(),
            reduce_motion: self.state.reduce_motion,
            osd_active: self.state.osd.as_ref().is_some_and(|o| o.visible),
            active_surfaces,
            wallpaper,
            mean_luminance: self.state.wallpaper.mean_luminance,
            bright_backdrop: self.state.wallpaper.is_bright_backdrop(),
            workspace_count: self.state.workspaces.len(),
            focused_window_title: self.state.focused_window.as_ref().map(|w| w.title.clone()),
        }
    }

    /// Handle an external [`ShellEvent`] and return resulting [`AppCommand`]s.
    pub fn handle_event(&mut self, event: ShellEvent, now: Instant) -> Vec<AppCommand<SurfaceId>> {
        match event {
            ShellEvent::Ipc { command, reply } => {
                let (response, cmds) = self.handle_ipc_command(command, now);
                let _ = reply.send(response);
                cmds
            }
            ShellEvent::SetBarMode(mode) => {
                self.state.set_bar_mode(mode);
                vec![AppCommand::RedrawAll]
            }
            ShellEvent::ToggleBarHidden => {
                self.state.toggle_bar_hidden();
                vec![AppCommand::RedrawAll]
            }
            ShellEvent::ShowOsd { payload, output } => self.show_osd(payload, output),
            ShellEvent::HideOsd => {
                self.state.osd = None;
                vec![
                    AppCommand::CancelTimer(TIMER_OSD_HIDE),
                    AppCommand::RedrawAll,
                ]
            }
            ShellEvent::SetWallpaper { path, output } => self.set_wallpaper(path, output, now),
            ShellEvent::WallpaperReady { image } => self.wallpaper_ready(image, now),
            ShellEvent::WallpaperFailed { path, error } => {
                warn!(path = %path.display(), "wallpaper decode failed: {error}");
                self.wallpapers.on_decode_failed(&path);
                Vec::new()
            }
            ShellEvent::ApplyTheme {
                scheme,
                variant,
                mean_luminance,
            } => {
                let next_variant = variant.unwrap_or(self.state.theme.variant);
                let next_scheme = scheme.map_or_else(
                    || {
                        if next_variant == self.state.theme.variant {
                            self.state.theme.scheme
                        } else {
                            Scheme::reference(next_variant)
                        }
                    },
                    |boxed| *boxed,
                );
                self.state.theme = Theme {
                    scheme: next_scheme,
                    variant: next_variant,
                };
                if let Some(lum) = mean_luminance {
                    self.state.wallpaper.mean_luminance = lum;
                }
                vec![AppCommand::RedrawAll]
            }
            ShellEvent::SetReduceMotion(enabled) => {
                self.state.reduce_motion = enabled;
                self.wallpapers.set_reduce_motion(enabled);
                vec![AppCommand::SetReduceMotion(enabled), AppCommand::RedrawAll]
            }
            ShellEvent::OutputsChanged(outputs) => self.reconcile_outputs(&outputs, now),
            ShellEvent::FocusedOutputChanged(output) => {
                self.state.focused_output = output;
                self.refresh_active_wallpaper();
                vec![AppCommand::RedrawAll]
            }
            ShellEvent::WorkspacesUpdated(workspaces) => {
                self.state.workspaces = workspaces;
                vec![AppCommand::RedrawAll]
            }
            ShellEvent::FocusedWindowChanged(window) => {
                self.state.focused_window = window;
                vec![AppCommand::RedrawAll]
            }
            ShellEvent::Quit => vec![AppCommand::Quit],
        }
    }

    fn handle_ipc_command(
        &mut self,
        command: IpcCommand,
        now: Instant,
    ) -> (IpcResponse, Vec<AppCommand<SurfaceId>>) {
        match command {
            IpcCommand::Status => {
                let snapshot = self.status_snapshot();
                match serde_json::to_value(&snapshot) {
                    Ok(data) => (IpcResponse::ok_data(data), Vec::new()),
                    Err(err) => (
                        IpcResponse::error(format!("failed to serialize status: {err}")),
                        Vec::new(),
                    ),
                }
            }
            IpcCommand::Bar { action } => {
                let cmds = match action {
                    BarCommand::Autohide => {
                        self.handle_event(ShellEvent::SetBarMode(BarMode::Autohide), now)
                    }
                    BarCommand::Pinned => {
                        self.handle_event(ShellEvent::SetBarMode(BarMode::Pinned), now)
                    }
                    BarCommand::Hidden => {
                        self.handle_event(ShellEvent::SetBarMode(BarMode::Hidden), now)
                    }
                    BarCommand::Toggle => self.handle_event(ShellEvent::ToggleBarHidden, now),
                };
                let mode = self.state.bar_mode;
                (IpcResponse::ok(format!("bar mode set to {mode:?}")), cmds)
            }
            IpcCommand::Osd { action } => match action {
                OsdCommand::Show { payload, output } => {
                    let cmds = self.handle_event(ShellEvent::ShowOsd { payload, output }, now);
                    if self.state.osd.is_some() {
                        (IpcResponse::ok("osd shown"), cmds)
                    } else {
                        (IpcResponse::error("no connected output for osd"), cmds)
                    }
                }
                OsdCommand::Hide => {
                    let cmds = self.handle_event(ShellEvent::HideOsd, now);
                    (IpcResponse::ok("osd hidden"), cmds)
                }
            },
            IpcCommand::Wallpaper { path, output } => {
                let display = path.display().to_string();
                let cmds = self.handle_event(ShellEvent::SetWallpaper { path, output }, now);
                (
                    IpcResponse::ok(format!("wallpaper updated: {display}")),
                    cmds,
                )
            }
            IpcCommand::Theme { action } => match action {
                ThemeCommand::Dark => {
                    let cmds = self.handle_event(
                        ShellEvent::ApplyTheme {
                            scheme: Some(Box::new(Scheme::DARK)),
                            variant: Some(Variant::Dark),
                            mean_luminance: None,
                        },
                        now,
                    );
                    (IpcResponse::ok("theme set to dark"), cmds)
                }
                ThemeCommand::Light => {
                    let cmds = self.handle_event(
                        ShellEvent::ApplyTheme {
                            scheme: Some(Box::new(Scheme::LIGHT)),
                            variant: Some(Variant::Light),
                            mean_luminance: None,
                        },
                        now,
                    );
                    (IpcResponse::ok("theme set to light"), cmds)
                }
                ThemeCommand::ToggleVariant => {
                    let next = match self.state.theme.variant {
                        Variant::Dark => Variant::Light,
                        Variant::Light => Variant::Dark,
                    };
                    let cmds = self.handle_event(
                        ShellEvent::ApplyTheme {
                            scheme: Some(Box::new(Scheme::reference(next))),
                            variant: Some(next),
                            mean_luminance: None,
                        },
                        now,
                    );
                    (
                        IpcResponse::ok(format!("theme set to {}", variant_as_str(next))),
                        cmds,
                    )
                }
                ThemeCommand::ReduceMotion { enabled } => {
                    let cmds = self.handle_event(ShellEvent::SetReduceMotion(enabled), now);
                    (IpcResponse::ok(format!("reduce_motion={enabled}")), cmds)
                }
            },
            IpcCommand::Quit => (IpcResponse::ok("shutting down"), vec![AppCommand::Quit]),
        }
    }

    /// Update the wallpaper target globally or per output and kick off any
    /// needed off-thread decode.
    fn set_wallpaper(
        &mut self,
        path: PathBuf,
        output: Option<String>,
        now: Instant,
    ) -> Vec<AppCommand<SurfaceId>> {
        if let Some(output_name) = output {
            self.state.wallpaper.per_output.insert(output_name, path);
        } else {
            // A new global default retargets every output without an explicit
            // per-output override.
            self.state.wallpaper.default_path = Some(path);
        }
        self.refresh_wallpaper_targets(now);
        let mut cmds = vec![AppCommand::RedrawAll];
        if self.wallpapers.has_active_fades() {
            cmds.push(AppCommand::ScheduleTimer {
                tag: TIMER_WALLPAPER_SETTLE,
                delay: crossfade_duration(self.state.reduce_motion),
            });
        }
        cmds
    }

    /// A decoded wallpaper landed from the worker thread: queue its GPU upload
    /// (fades begin in `sync_images`) and arm the settle timer when an output
    /// is transitioning to it.
    fn wallpaper_ready(
        &mut self,
        image: Arc<WallpaperImage>,
        now: Instant,
    ) -> Vec<AppCommand<SurfaceId>> {
        let will_fade = self.wallpapers.will_transition_to(&image.path);
        self.wallpapers.on_decoded(image, now);
        self.refresh_active_wallpaper();
        let mut cmds = vec![AppCommand::RedrawAll];
        if will_fade || self.wallpapers.has_active_fades() {
            cmds.push(AppCommand::ScheduleTimer {
                tag: TIMER_WALLPAPER_SETTLE,
                delay: crossfade_duration(self.state.reduce_motion),
            });
        }
        cmds
    }

    fn show_osd(
        &mut self,
        payload: OsdPayload,
        output: Option<String>,
    ) -> Vec<AppCommand<SurfaceId>> {
        let target = output
            .filter(|name| self.state.outputs.iter().any(|o| o.name == *name))
            .or_else(|| self.state.target_output_name().map(str::to_owned));

        let Some(target_output) = target else {
            return Vec::new();
        };

        self.state.osd = Some(OsdState {
            payload,
            output: target_output,
            visible: true,
        });

        vec![
            AppCommand::ScheduleTimer {
                tag: TIMER_OSD_HIDE,
                delay: OSD_HOLD_DURATION,
            },
            AppCommand::RedrawAll,
        ]
    }

    fn reconcile_outputs(
        &mut self,
        outputs: &[OutputInfo],
        now: Instant,
    ) -> Vec<AppCommand<SurfaceId>> {
        self.state.outputs = outputs.to_vec();
        self.sync_output_registry(outputs);

        // Drop wallpaper display state for outputs that went away (releases
        // their texture refs), then re-target survivors for hotplug.
        self.wallpapers
            .retain_outputs(|key| self.key_to_name.contains_key(key));

        let connected: HashSet<&str> = outputs.iter().map(|o| o.name.as_str()).collect();

        // Keep focused_output valid across hotplug removal.
        let focused_valid = self
            .state
            .focused_output
            .as_deref()
            .is_some_and(|name| connected.contains(name));
        if !focused_valid {
            self.state.focused_output = outputs.first().map(|o| o.name.clone());
        }

        // Keep active OSD bound to a connected output (or clear if none left).
        if let Some(ref mut osd) = self.state.osd
            && !connected.contains(osd.output.as_str())
        {
            if let Some(ref fallback) = self.state.focused_output {
                osd.output.clone_from(fallback);
            } else {
                self.state.osd = None;
            }
        }

        self.refresh_wallpaper_targets(now);
        self.refresh_active_wallpaper();
        vec![AppCommand::RedrawAll]
    }

    /// Point every connected output at its effective wallpaper path
    /// (per-output override, else `default_path`) and spawn off-thread decodes
    /// for any path with no decoded pixels yet.
    fn refresh_wallpaper_targets(&mut self, now: Instant) {
        let targets: Vec<(OutputKey, PathBuf)> = self
            .state
            .outputs
            .iter()
            .filter_map(|o| {
                let key = *self.name_to_key.get(&o.name)?;
                let path = self.state.wallpaper.path_for(&o.name)?.clone();
                Some((key, path))
            })
            .collect();
        for (key, path) in targets {
            self.wallpapers.request(key, &path, now);
        }
        self.spawn_needed_decodes();
    }

    /// Spawn a `triton-wallpaper` decode worker for every desired path with no
    /// cached pixels and no decode already in flight. Without an event sender
    /// (headless tests inject `WallpaperReady` directly) this is a no-op.
    fn spawn_needed_decodes(&mut self) {
        let Some(sender) = self.event_sender.clone() else {
            return;
        };
        for path in self.wallpapers.undecoded_desires() {
            self.wallpapers.mark_decoding(&path);
            if let Err(err) = crate::wallpaper::spawn_decode(path.clone(), sender.clone()) {
                warn!(path = %path.display(), error = %err, "failed to spawn wallpaper decode");
                self.wallpapers.on_decode_failed(&path);
            }
        }
    }

    /// Refresh `mean_luminance` / `active_image` in shell state from the
    /// decoded image committed to the focused output — the freshest desired
    /// image once its decode landed, otherwise whatever is still displayed.
    fn refresh_active_wallpaper(&mut self) {
        let target_key = self
            .state
            .target_output_name()
            .and_then(|name| self.name_to_key.get(name).copied());
        let desired_image = target_key
            .and_then(|key| self.wallpapers.desired_path(key))
            .and_then(|path| self.wallpapers.image_for_path(&path));
        let image = desired_image
            .or_else(|| target_key.and_then(|key| self.wallpapers.displayed_image(key)));
        self.state.wallpaper.mean_luminance = image.as_ref().map_or(0.0, |img| img.mean_luminance);
        self.state.wallpaper.active_image = image;
    }

    fn sync_output_registry(&mut self, outputs: &[OutputInfo]) {
        let active_names: HashSet<&str> = outputs.iter().map(|o| o.name.as_str()).collect();
        self.name_to_key
            .retain(|name, _| active_names.contains(name.as_str()));
        self.key_to_name
            .retain(|_, name| active_names.contains(name.as_str()));

        for output in outputs {
            if !self.name_to_key.contains_key(&output.name) {
                let key = OutputKey(self.next_output_key);
                self.next_output_key += 1;
                self.name_to_key.insert(output.name.clone(), key);
                self.key_to_name.insert(key, output.name.clone());
            }
        }
    }

    fn background_spec(output: &OutputInfo) -> SurfaceSpec {
        SurfaceSpec {
            namespace: SurfaceRole::Background.namespace().to_owned(),
            layer: SurfaceLayer::Background,
            anchor: SurfaceAnchor::TOP
                | SurfaceAnchor::BOTTOM
                | SurfaceAnchor::LEFT
                | SurfaceAnchor::RIGHT,
            margin: (0, 0, 0, 0),
            keyboard: SurfaceKeyboard::None,
            exclusive_zone: -1,
            sizing: SurfaceSize::Stretch,
            shaped_input: false,
            output: Some(output.name.clone()),
        }
    }

    fn bar_spec(&self, output: &OutputInfo) -> Option<SurfaceSpec> {
        let width = output.logical_size.map_or(DEFAULT_OUTPUT_WIDTH, |(w, _)| w);
        match self.state.bar_mode {
            BarMode::Hidden => None,
            BarMode::Autohide => Some(SurfaceSpec {
                namespace: SurfaceRole::Bar.namespace().to_owned(),
                layer: SurfaceLayer::Top,
                anchor: SurfaceAnchor::TOP | SurfaceAnchor::LEFT | SurfaceAnchor::RIGHT,
                margin: (0, 0, 0, 0),
                keyboard: SurfaceKeyboard::None,
                exclusive_zone: 0,
                sizing: SurfaceSize::StableExtent {
                    width,
                    height: BAR_AUTOHIDE_EXTENT_H,
                },
                shaped_input: true,
                output: Some(output.name.clone()),
            }),
            BarMode::Pinned => Some(SurfaceSpec {
                namespace: SurfaceRole::Bar.namespace().to_owned(),
                layer: SurfaceLayer::Top,
                anchor: SurfaceAnchor::TOP | SurfaceAnchor::LEFT | SurfaceAnchor::RIGHT,
                margin: (0, 0, 0, 0),
                keyboard: SurfaceKeyboard::None,
                exclusive_zone: SIZE.bar_height as i32,
                sizing: SurfaceSize::Fixed(width, SIZE.bar_height as u32),
                shaped_input: false,
                output: Some(output.name.clone()),
            }),
        }
    }

    fn osd_spec(output_name: &str) -> SurfaceSpec {
        SurfaceSpec {
            namespace: SurfaceRole::Osd.namespace().to_owned(),
            layer: SurfaceLayer::Overlay,
            anchor: SurfaceAnchor::BOTTOM,
            margin: (0, 0, 48, 0),
            keyboard: SurfaceKeyboard::None,
            exclusive_zone: 0,
            sizing: SurfaceSize::StableExtent {
                width: OSD_EXTENT_W,
                height: OSD_EXTENT_H,
            },
            shaped_input: true,
            output: Some(output_name.to_owned()),
        }
    }

    fn view_bar(&self, id: SurfaceId, ctx: &mut ViewContext<'_>) -> Element<ShellMessage> {
        let theme = &self.state.theme;
        let output_name = self.output_name(id.output).unwrap_or("");
        let mut ws_row = row().gap(SPACE[0]).items_center();
        for ws in self
            .state
            .workspaces
            .iter()
            .filter(|w| w.output == output_name)
        {
            let label = ws.name.clone().unwrap_or_else(|| ws.idx.to_string());
            let ws_id = ws.id;
            let key = NodeKey::named(&format!("bar-ws-{output_name}-{ws_id}"));
            ws_row = ws_row.child(pill(key, label, ws.is_active, theme, ctx.anim, move || {
                ShellMessage::FocusWorkspace(ws_id)
            }));
        }

        let status = status_pill(
            NodeKey::named(&format!("bar-status-{output_name}")),
            PillStatus::Neutral,
            theme,
            ctx.anim,
            Some(Box::new(|| ShellMessage::StatusPillClicked)),
            vec![
                icon(
                    Icon::Wifi,
                    IconStyle::Outlined,
                    SIZE.icon_s,
                    theme.scheme.on_surface,
                ),
                text(
                    format!("{}%", self.state.status.volume_percent),
                    TypeStyle::LABEL_L,
                    theme.scheme.on_surface,
                ),
            ],
        );

        match self.state.bar_mode {
            BarMode::Pinned => panel(Material::Mica, RADII.tiled, theme)
                // Adaptive Mica: bright wallpapers (+0.08 dark / +0.06 light
                // alpha) keep pinned-bar text above AA contrast (#18).
                .bg(Rgba::from_color(Material::Mica.fill_adaptive(
                    &theme.scheme,
                    theme.variant,
                    self.state.wallpaper.is_bright_backdrop(),
                )))
                .w_full()
                .h(SIZE.bar_height)
                .px(SPACE[3])
                .justify_center()
                .child(
                    row()
                        .w_full()
                        .items_center()
                        .justify_between()
                        .child(ws_row)
                        .child(status),
                ),
            BarMode::Autohide | BarMode::Hidden => {
                let left = shadow_gutter(island(theme).child(ws_row));
                let right = shadow_gutter(island(theme).child(status));
                row()
                    .w_full()
                    .justify_between()
                    .items_center()
                    .child(left)
                    .child(right)
            }
        }
    }

    fn view_osd(&self, ctx: &mut ViewContext<'_>) -> Element<ShellMessage> {
        let theme = &self.state.theme;
        let content = match self.state.osd.as_ref().map(|o| &o.payload) {
            Some(OsdPayload::Volume { level, muted }) => {
                let glyph = if *muted {
                    Icon::VolumeOff
                } else {
                    Icon::VolumeUp
                };
                row()
                    .gap(SPACE[2])
                    .items_center()
                    .child(icon(
                        glyph,
                        IconStyle::Filled,
                        SIZE.icon_m,
                        theme.scheme.on_surface,
                    ))
                    .child(wavy_progress(
                        NodeKey::named("osd-progress"),
                        160.0,
                        4.0,
                        Progress::Determinate(level.clamp(0.0, 1.0)),
                        theme,
                        ctx.anim,
                    ))
            }
            Some(OsdPayload::Brightness { level }) => row()
                .gap(SPACE[2])
                .items_center()
                .child(icon(
                    Icon::BrightnessHigh,
                    IconStyle::Filled,
                    SIZE.icon_m,
                    theme.scheme.on_surface,
                ))
                .child(wavy_progress(
                    NodeKey::named("osd-progress"),
                    160.0,
                    4.0,
                    Progress::Determinate(level.clamp(0.0, 1.0)),
                    theme,
                    ctx.anim,
                )),
            Some(OsdPayload::Mic { muted }) => row()
                .gap(SPACE[2])
                .items_center()
                .child(icon(
                    if *muted { Icon::MicOff } else { Icon::Mic },
                    IconStyle::Filled,
                    SIZE.icon_m,
                    theme.scheme.on_surface,
                ))
                .child(text(
                    if *muted { "Mic muted" } else { "Mic live" },
                    TypeStyle::LABEL_L,
                    theme.scheme.on_surface,
                )),
            Some(OsdPayload::KeyboardLayout { name }) => row()
                .gap(SPACE[2])
                .items_center()
                .child(icon(
                    Icon::Keyboard,
                    IconStyle::Outlined,
                    SIZE.icon_m,
                    theme.scheme.on_surface,
                ))
                .child(text(
                    name.clone(),
                    TypeStyle::LABEL_L,
                    theme.scheme.on_surface,
                )),
            Some(OsdPayload::Workspace { label }) => {
                row().gap(SPACE[2]).items_center().child(text(
                    format!("Workspace {label}"),
                    TypeStyle::LABEL_L,
                    theme.scheme.on_surface,
                ))
            }
            Some(OsdPayload::LowBattery { percent }) => row()
                .gap(SPACE[2])
                .items_center()
                .child(icon(
                    Icon::BatteryAlert,
                    IconStyle::Filled,
                    SIZE.icon_m,
                    theme.scheme.error,
                ))
                .child(text(
                    format!("Battery {percent}%"),
                    TypeStyle::LABEL_L,
                    theme.scheme.on_surface,
                )),
            None => row(),
        };

        shadow_gutter(
            panel(Material::Glass, RADII.full, theme)
                .px(SPACE[4])
                .py(SPACE[2])
                .child(content),
        )
    }
}

impl ShellApp for TritonShell {
    type SurfaceId = SurfaceId;
    type Message = ShellMessage;

    fn surfaces(&self) -> Vec<(Self::SurfaceId, SurfaceSpec)> {
        let mut specs = Vec::new();
        for output in &self.state.outputs {
            let Some(&output_key) = self.name_to_key.get(&output.name) else {
                continue;
            };
            specs.push((
                SurfaceId {
                    role: SurfaceRole::Background,
                    output: output_key,
                },
                Self::background_spec(output),
            ));
            if let Some(bar_spec) = self.bar_spec(output) {
                specs.push((
                    SurfaceId {
                        role: SurfaceRole::Bar,
                        output: output_key,
                    },
                    bar_spec,
                ));
            }
        }

        if let Some(ref osd) = self.state.osd
            && osd.visible
            && let Some(&output_key) = self.name_to_key.get(&osd.output)
        {
            specs.push((
                SurfaceId {
                    role: SurfaceRole::Osd,
                    output: output_key,
                },
                Self::osd_spec(&osd.output),
            ));
        }

        specs
    }

    fn view(&self, id: Self::SurfaceId, ctx: &mut ViewContext<'_>) -> Element<Self::Message> {
        match id.role {
            SurfaceRole::Background => {
                view_background(&self.wallpapers, id.output, &self.state.theme, ctx.now)
            }
            SurfaceRole::Bar => self.view_bar(id, ctx),
            SurfaceRole::Osd => self.view_osd(ctx),
        }
    }

    fn update(
        &mut self,
        _id: Self::SurfaceId,
        msg: Self::Message,
        _now: Instant,
    ) -> Vec<AppCommand<Self::SurfaceId>> {
        match msg {
            ShellMessage::FocusWorkspace(ws_id) => {
                for ws in &mut self.state.workspaces {
                    ws.is_active = ws.id == ws_id;
                    ws.is_focused = ws.id == ws_id;
                }
                vec![AppCommand::RedrawAll]
            }
            ShellMessage::None | ShellMessage::StatusPillClicked => Vec::new(),
        }
    }

    fn is_animating(&self, id: Self::SurfaceId, now: Instant) -> bool {
        match id.role {
            SurfaceRole::Background => self.wallpapers.is_animating(id.output, now),
            SurfaceRole::Bar | SurfaceRole::Osd => false,
        }
    }

    fn on_outputs_changed(
        &mut self,
        outputs: &[OutputInfo],
        now: Instant,
    ) -> Vec<AppCommand<Self::SurfaceId>> {
        self.reconcile_outputs(outputs, now)
    }

    fn sync_images(&mut self, images: &mut ImageStore) {
        self.wallpapers.sync(images);
    }

    fn on_timer(&mut self, tag: u32, now: Instant) -> Vec<AppCommand<Self::SurfaceId>> {
        match tag {
            TIMER_OSD_HIDE => {
                self.state.osd = None;
                vec![AppCommand::RedrawAll]
            }
            TIMER_WALLPAPER_SETTLE => {
                // Release outgoing textures whose crossfade completed; re-arm
                // if a later fade is still running.
                let mut cmds = vec![AppCommand::RedrawAll];
                if let Some(remaining) = self.wallpapers.settle(now) {
                    cmds.push(AppCommand::ScheduleTimer {
                        tag: TIMER_WALLPAPER_SETTLE,
                        delay: remaining,
                    });
                }
                self.refresh_active_wallpaper();
                cmds
            }
            _ => Vec::new(),
        }
    }
}
