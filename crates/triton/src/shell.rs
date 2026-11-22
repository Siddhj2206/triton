//! `TritonShell`: single-process `ShellApp` host with declarative
//! `(SurfaceRole, OutputName)` surface reconciliation.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use triton_design::color::{Scheme, Variant};
use triton_design::icon::{Icon, IconStyle};
use triton_design::material::Material;
use triton_design::shape::{RADII, SIZE, SPACE};
use triton_design::typography::TypeStyle;
use triton_engine::{
    AppCommand, Element, NodeKey, OutputInfo, ShellApp, SurfaceAnchor, SurfaceKeyboard,
    SurfaceLayer, SurfaceSize, SurfaceSpec, Theme, ViewContext, icon, row, text,
};
use triton_kit::{Progress, panel, shadow_gutter, wavy_progress};

use crate::bar::{BarClock, BarController};
use crate::ipc::{BarCommand, IpcCommand, IpcResponse, OsdCommand, ThemeCommand};
use crate::niri_ipc::{NiriAction, NiriActionSender};
use crate::state::{
    BackendMode, BarMode, OsdPayload, OsdState, OutputKey, ShellEvent, ShellState,
    ShellStatusSnapshot, SurfaceRole, SurfaceSummary, variant_as_str,
};

/// Timer tag for dismissing the OSD pill after its hold period.
pub const TIMER_OSD_HIDE: u32 = 1;

/// Default OSD hold duration (`docs/design-spec.md`: 1.2 s).
pub const OSD_HOLD_DURATION: Duration = Duration::from_millis(1200);

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
    /// User clicked the Bar notification bell.
    NotificationBellClicked,
}

/// Main Triton desktop shell application implementing [`ShellApp`].
#[derive(Debug)]
pub struct TritonShell {
    state: ShellState,
    bar: BarController,
    name_to_key: HashMap<String, OutputKey>,
    key_to_name: HashMap<OutputKey, String>,
    next_output_key: u32,
    /// Non-blocking niri action dispatch (set only under `--real-services`).
    niri_sink: Option<NiriActionSender>,
}

impl TritonShell {
    /// Construct a new shell daemon in the given [`BackendMode`].
    pub fn new(backend: BackendMode) -> Self {
        let now = Instant::now();
        let state = ShellState::for_backend(backend);
        let mut shell = Self {
            state,
            bar: BarController::new(now),
            name_to_key: HashMap::new(),
            key_to_name: HashMap::new(),
            next_output_key: 1,
            niri_sink: None,
        };
        let initial_outputs = shell.state.outputs.clone();
        shell.sync_output_registry(&initial_outputs, now);
        shell
    }

    /// Borrow the active [`BarController`].
    pub fn bar(&self) -> &BarController {
        &self.bar
    }

    /// Mutably borrow the active [`BarController`].
    pub fn bar_mut(&mut self) -> &mut BarController {
        &mut self.bar
    }

    /// Override the Bar clock (useful for deterministic headless testing).
    pub fn set_bar_clock(&mut self, clock: BarClock) {
        self.bar.set_clock(clock);
    }

    /// Configure the normal (non-fullscreen) Autohide bar reveal delay.
    pub fn set_bar_reveal_delay(&mut self, delay: Duration) {
        self.bar.set_reveal_delay(delay);
    }

    /// Borrow the current [`ShellState`].
    pub fn state(&self) -> &ShellState {
        &self.state
    }

    /// Inject the niri action sink used to dispatch compositor actions
    /// (workspace pill clicks) off the UI thread under `--real-services`.
    pub fn set_niri_action_sink(&mut self, sink: NiriActionSender) {
        self.niri_sink = Some(sink);
    }

    /// Current niri action sink, if live compositor integration is active.
    pub fn niri_action_sink(&self) -> Option<&NiriActionSender> {
        self.niri_sink.as_ref()
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
                let mut cmds = self.bar.on_mode_changed(mode, now);
                cmds.push(AppCommand::RedrawAll);
                cmds
            }
            ShellEvent::ToggleBarHidden => {
                let mode = self.state.toggle_bar_hidden();
                let mut cmds = self.bar.on_mode_changed(mode, now);
                cmds.push(AppCommand::RedrawAll);
                cmds
            }
            ShellEvent::ShowOsd { payload, output } => self.show_osd(payload, output),
            ShellEvent::HideOsd => {
                self.state.osd = None;
                vec![
                    AppCommand::CancelTimer(TIMER_OSD_HIDE),
                    AppCommand::RedrawAll,
                ]
            }
            ShellEvent::SetWallpaper { path, output } => {
                if let Some(output_name) = output {
                    self.state.wallpaper.per_output.insert(output_name, path);
                } else {
                    self.state.wallpaper.default_path = Some(path);
                }
                vec![AppCommand::RedrawAll]
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
                self.bar.set_reduce_motion(enabled, now);
                vec![AppCommand::SetReduceMotion(enabled), AppCommand::RedrawAll]
            }
            ShellEvent::OutputsChanged(outputs) => self.reconcile_outputs(&outputs, now),
            ShellEvent::FocusedOutputChanged(output) => {
                self.state.focused_output = output;
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
            ShellEvent::StatusUpdated(status) => {
                self.state.status = status;
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
        let mut cmds = self.sync_output_registry(outputs, now);

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

        cmds.push(AppCommand::RedrawAll);
        cmds
    }

    fn sync_output_registry(
        &mut self,
        outputs: &[OutputInfo],
        now: Instant,
    ) -> Vec<AppCommand<SurfaceId>> {
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

        let active_keys: HashSet<OutputKey> = self.key_to_name.keys().copied().collect();
        self.bar
            .sync_outputs(&active_keys, self.state.reduce_motion, now)
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
        BarController::surface_spec(self.state.bar_mode, output)
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

    fn view_background(&self) -> Element<ShellMessage> {
        panel(Material::Solid, RADII.tiled, &self.state.theme)
            .w_full()
            .h_full()
    }

    fn view_bar(&self, id: SurfaceId, ctx: &mut ViewContext<'_>) -> Element<ShellMessage> {
        let output_name = self.output_name(id.output).unwrap_or("");
        self.bar.view(id, output_name, &self.state, ctx)
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
            SurfaceRole::Background => self.view_background(),
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
                if let Some(sink) = &self.niri_sink {
                    sink.send(NiriAction::FocusWorkspace(ws_id));
                }
                for ws in &mut self.state.workspaces {
                    ws.is_active = ws.id == ws_id;
                    ws.is_focused = ws.id == ws_id;
                }
                vec![AppCommand::RedrawAll]
            }
            ShellMessage::None
            | ShellMessage::StatusPillClicked
            | ShellMessage::NotificationBellClicked => Vec::new(),
        }
    }

    fn is_animating(&self, id: Self::SurfaceId, now: Instant) -> bool {
        match id.role {
            SurfaceRole::Bar => self.bar.is_animating(id.output, now),
            SurfaceRole::Background | SurfaceRole::Osd => false,
        }
    }

    fn on_start(&mut self, now: Instant) -> Vec<AppCommand<Self::SurfaceId>> {
        self.bar.on_start(now)
    }

    fn on_outputs_changed(
        &mut self,
        outputs: &[OutputInfo],
        now: Instant,
    ) -> Vec<AppCommand<Self::SurfaceId>> {
        self.reconcile_outputs(outputs, now)
    }

    fn on_pointer_enter(
        &mut self,
        id: Self::SurfaceId,
        _pos: (f32, f32),
        now: Instant,
    ) -> Vec<AppCommand<Self::SurfaceId>> {
        let Some(output_name) = self.output_name(id.output).map(str::to_owned) else {
            return Vec::new();
        };
        self.bar
            .on_pointer_enter(id, &self.state, &output_name, now)
    }

    fn on_pointer_leave(
        &mut self,
        id: Self::SurfaceId,
        now: Instant,
    ) -> Vec<AppCommand<Self::SurfaceId>> {
        self.bar.on_pointer_leave(id, &self.state, now)
    }

    fn on_timer(&mut self, tag: u32, now: Instant) -> Vec<AppCommand<Self::SurfaceId>> {
        if tag == TIMER_OSD_HIDE {
            self.state.osd = None;
            return vec![AppCommand::RedrawAll];
        }
        if let Some(cmds) = self.bar.on_timer(tag, &self.state, now) {
            return cmds;
        }
        Vec::new()
    }
}
