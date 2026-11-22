//! `TritonShell`: single-process `ShellApp` host with declarative
//! `(SurfaceRole, OutputName)` surface reconciliation.

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use triton_design::color::{Scheme, Variant};
use triton_design::icon::{Icon, IconStyle};
use triton_design::material::Material;
use triton_design::shape::{RADII, SIZE, SPACE};
use triton_design::typography::TypeStyle;
use triton_engine::{
    AppCommand, Element, NodeKey, OutputInfo, ShellApp, SurfaceAnchor, SurfaceKeyboard,
    SurfaceLayer, SurfaceSize, SurfaceSpec, Theme, ViewContext, box_, icon, row, text,
};
use triton_kit::{PillStatus, island, panel, pill, shadow_gutter, status_pill};

use crate::ipc::{BarCommand, IpcCommand, IpcResponse, OsdCommand, ThemeCommand};
use crate::osd::{self, OSD_HOLD_DURATION, OsdPayload, OsdState, TIMER_OSD_HIDE, TIMER_OSD_REAP};
use crate::state::{
    BackendMode, BarMode, OutputKey, ShellEvent, ShellState, ShellStatusSnapshot, SurfaceRole,
    SurfaceSummary, variant_as_str,
};

/// Default logical width used when an output does not report a logical size yet.
const DEFAULT_OUTPUT_WIDTH: u32 = 1920;

/// Stable max extent height for the floating autohide bar (8 px top offset +
/// 56 px bar height + `ShadowToken::FLOATING` top/bottom gutters).
const BAR_AUTOHIDE_EXTENT_H: u32 = 224;

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
        };
        let initial_outputs = shell.state.outputs.clone();
        shell.sync_output_registry(&initial_outputs);
        shell
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
            ShellEvent::ShowOsd { payload, output } => self.show_osd(payload, output, now),
            ShellEvent::HideOsd => {
                self.state.osd = None;
                vec![
                    AppCommand::CancelTimer(TIMER_OSD_HIDE),
                    AppCommand::CancelTimer(TIMER_OSD_REAP),
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
                if let Some(ref mut osd) = self.state.osd {
                    osd.set_reduce_motion(enabled, now);
                }
                vec![AppCommand::SetReduceMotion(enabled), AppCommand::RedrawAll]
            }
            ShellEvent::OutputsChanged(outputs) => self.reconcile_outputs(&outputs),
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
        now: Instant,
    ) -> Vec<AppCommand<SurfaceId>> {
        let target = output
            .filter(|name| self.state.outputs.iter().any(|o| o.name == *name))
            .or_else(|| self.state.target_output_name().map(str::to_owned));

        let Some(target_output) = target else {
            return Vec::new();
        };

        // Payload switches and mid-exit revivals morph in place: the surface id
        // never changes, so the compositor keeps the same layer surface.
        match self.state.osd.as_mut() {
            Some(osd) => osd.show(payload, target_output, now),
            None => {
                self.state.osd = Some(OsdState::entering(
                    payload,
                    target_output,
                    self.state.reduce_motion,
                    now,
                ));
            }
        }

        // Re-arming the same tag refreshes the hold deadline on every event.
        vec![
            AppCommand::ScheduleTimer {
                tag: TIMER_OSD_HIDE,
                delay: OSD_HOLD_DURATION,
            },
            AppCommand::RedrawAll,
        ]
    }

    fn reconcile_outputs(&mut self, outputs: &[OutputInfo]) -> Vec<AppCommand<SurfaceId>> {
        self.state.outputs = outputs.to_vec();
        self.sync_output_registry(outputs);

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

        vec![AppCommand::RedrawAll]
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

    fn view_background(&self) -> Element<ShellMessage> {
        panel(Material::Solid, RADII.tiled, &self.state.theme)
            .w_full()
            .h_full()
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

        // The OSD stays mounted through its exit morph; the reap timer drops
        // `state.osd` once the morph settles, unmounting the surface cleanly.
        if let Some(ref osd) = self.state.osd
            && let Some(&output_key) = self.name_to_key.get(&osd.output)
        {
            specs.push((
                SurfaceId {
                    role: SurfaceRole::Osd,
                    output: output_key,
                },
                osd::surface_spec(&osd.output),
            ));
        }

        specs
    }

    fn view(&self, id: Self::SurfaceId, ctx: &mut ViewContext<'_>) -> Element<Self::Message> {
        match id.role {
            SurfaceRole::Background => self.view_background(),
            SurfaceRole::Bar => self.view_bar(id, ctx),
            SurfaceRole::Osd => match self.state.osd.as_ref() {
                Some(osd) => osd::view(osd, &self.state.theme, self.state.reduce_motion, ctx),
                None => box_(),
            },
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
            // The app-owned presence channel drives frames during the
            // enter/exit morph; once settled the surface goes idle (0% CPU).
            SurfaceRole::Osd => self.state.osd.as_ref().is_some_and(|o| !o.is_settled(now)),
            _ => false,
        }
    }

    fn on_outputs_changed(
        &mut self,
        outputs: &[OutputInfo],
        _now: Instant,
    ) -> Vec<AppCommand<Self::SurfaceId>> {
        self.reconcile_outputs(outputs)
    }

    fn on_timer(&mut self, tag: u32, now: Instant) -> Vec<AppCommand<Self::SurfaceId>> {
        match tag {
            TIMER_OSD_HIDE => {
                let Some(osd) = self.state.osd.as_mut() else {
                    return Vec::new();
                };
                if osd.is_exiting() {
                    return Vec::new();
                }
                osd.dismiss(now);
                vec![
                    AppCommand::ScheduleTimer {
                        tag: TIMER_OSD_REAP,
                        delay: osd::OSD_REAP_INTERVAL,
                    },
                    AppCommand::RedrawAll,
                ]
            }
            TIMER_OSD_REAP => {
                let Some(osd) = self.state.osd.as_ref() else {
                    return Vec::new();
                };
                if !osd.is_exiting() {
                    // Revived mid-exit: a stale reap is a no-op.
                    return Vec::new();
                }
                if osd.is_settled(now) {
                    self.state.osd = None;
                    vec![AppCommand::RedrawAll]
                } else {
                    vec![AppCommand::ScheduleTimer {
                        tag: TIMER_OSD_REAP,
                        delay: osd::OSD_REAP_INTERVAL,
                    }]
                }
            }
            _ => Vec::new(),
        }
    }
}
