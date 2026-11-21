//! The shell as one iced daemon: niri snapshots in, a bar on every output out.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use iced_core::alignment::Vertical;
use iced_core::keyboard::{self, Key, key};
use iced_core::widget::{self, operation::focusable};
use iced_core::window::{self, Id};
use iced_core::{Color, Event, Length, Padding};
use iced_futures::{Subscription, event};
use iced_layershell::reexport::Task;
use iced_layershell::to_layer_message;
use iced_widget::{Column, Row, Space, mouse_area};
use triton_design::material::Material;
use triton_design::{Icon, TypeStyle};
use triton_ui::bar::{self, Status};
use triton_ui::button::{self, chip};
use triton_ui::field::{self, Field};
use triton_ui::measure::{Dim, measure, measure_height};
use triton_ui::surface::{self, layer_island};
use triton_ui::text;
use triton_ui::{Element, Renderer, StatusTone, Surface, Theme, Tone};

use crate::host::SurfaceTable;
use crate::msg;
use crate::niri::{self, Snapshot};
use crate::services::{self, Connectivity, Statuses};
use crate::shell::{self, BarMode, Centered, Modals, Role, Sizes, SurfaceKey};

/// Which centered modal is up; maps to `shell::Centered` with its output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CenteredRole {
    Power,
    Polkit,
}

/// The launcher field's widget id, so it can be focused once its surface maps.
fn launcher_field() -> widget::Id {
    widget::Id::from("launcher-query")
}

/// The polkit dialog's field id (focused when the dialog maps).
pub fn polkit_field() -> widget::Id {
    widget::Id::from("polkit-password")
}

/// Copy `text` to the clipboard — `wl-copy` spawn, the transitional backend
/// until the launcher grows a proper data-control client.
fn copy_text(text: String) {
    std::thread::spawn(move || {
        if let Err(e) = std::process::Command::new("wl-copy").arg(&text).spawn() {
            tracing::debug!(%e, "wl-copy missing; result not copied");
        }
    });
}

/// Ids for the shell's own synthesized notifications, above the D-Bus
/// server's range so they cannot collide.
fn notif_gen() -> u32 {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1 << 30);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// What the pipeline worker hands back: the wallpaper image and both schemes.
#[derive(Debug)]
pub struct PipelineDone {
    pub wallpaper: Option<iced_widget::image::Handle>,
    /// Mean wallpaper luminance → Mica's adaptive tint.
    pub luminance: f32,
    pub dark: triton_design::Scheme,
    pub light: triton_design::Scheme,
    /// Targets the last `apply` finished (for its toast).
    pub applied: Vec<&'static str>,
}

/// The `>`-scope command palette — the shell's own actions as hits.
/// `id` is the `triton msg` line each runs.
const LAUNCH_COMMANDS: &[crate::launch::CommandRef<'static>] = &[
    crate::launch::CommandRef {
        title: "Lock screen",
        supporting: Some("Lock the session"),
        id: "lock",
    },
    crate::launch::CommandRef {
        title: "Suspend",
        supporting: Some("Suspend the machine"),
        id: "suspend",
    },
    crate::launch::CommandRef {
        title: "Log out",
        supporting: Some("End the niri session"),
        id: "power toggle",
    },
    crate::launch::CommandRef {
        title: "Night light",
        supporting: Some("Toggle the warm gamma"),
        id: "nightlight toggle",
    },
    crate::launch::CommandRef {
        title: "Do not disturb",
        supporting: Some("Silence notifications"),
        id: "dnd toggle",
    },
];

/// The Thick panel's inner padding; the field's radius is `nested` against it.
const LAUNCHER_PAD: f32 = 14.0;

/// An empty results set for the launcher's pre-search frame.
static EMPTY_RESULTS: crate::launch::Results = crate::launch::Results { groups: Vec::new() };

/// `BarStates` timings: 120 ms on the top edge wakes the bar, 600 ms of no
/// pointer over bar chrome puts it away. (The fullscreen 300 ms dwell waits on
/// a fullscreen flag in niri's IPC.)
const REVEAL_DELAY: Duration = Duration::from_millis(120);
const HIDE_DELAY: Duration = Duration::from_millis(600);

/// The result groups, as list sections under eyebrow labels.
fn launcher_results(results: &crate::launch::Results) -> Column<'_, Message, Theme, Renderer> {
    let mut col = Column::new().spacing(6);
    for (group, hits) in &results.groups {
        let label = match group {
            crate::launch::Group::Apps => "Apps",
            crate::launch::Group::Windows => "Windows",
            crate::launch::Group::Commands => "Commands",
        };
        let items: Vec<surface::ListItem<'_, Message>> = hits.iter().map(|h| hit_row(h)).collect();
        col = col
            .push(text::eyebrow(label))
            .push(surface::list(items, false));
    }
    col
}

fn hit_row(h: &crate::launch::Hit) -> surface::ListItem<'_, Message> {
    let icon = match h.kind {
        crate::launch::HitKind::App { .. } => Icon::Apps,
        crate::launch::HitKind::Window { .. } | crate::launch::HitKind::Command(_) => {
            Icon::Terminal
        }
        crate::launch::HitKind::Math { .. } => Icon::Calculate,
    };
    surface::ListItem {
        leading: Some(surface::icon_disc(icon).into()),
        title: h.title.as_str(),
        supporting: h.supporting.as_deref(),
        trailing: h.tag.as_deref().map(|t| surface::kbd(t).into()),
        selected: false,
        on_press: Some(Message::LaunchHit(h.kind.clone())),
    }
}

/// The key-hint footer.
fn launcher_hints<'a>() -> Row<'a, Message, Theme, Renderer> {
    let hint = |key, label| {
        Row::new()
            .spacing(8)
            .align_y(Vertical::Center)
            .push(surface::kbd(key))
            .push(text::text(label, TypeStyle::LABEL_S).class(Tone::OnSurfaceVariant))
    };
    Row::new()
        .spacing(18)
        .push(hint("↑↓", "move"))
        .push(hint("↵", "open"))
        .push(hint("Tab", "actions"))
        .push(hint("Alt↵", "reveal"))
        .push(hint("Esc", "close"))
}

#[to_layer_message(multi)]
#[derive(Debug, Clone)]
pub enum Message {
    Niri(Arc<Snapshot>),
    Minute,
    Closed(Id),
    /// A surface's first configure (its UI exists from here).
    Opened(Id),
    /// A surface's natural extent on one axis, so the surface can fit it.
    Measured(Id, Dim, f32),
    FocusWorkspace(u64),
    /// The wave was pressed on this output: toggle the launcher there.
    Launcher(String),
    /// Esc or a scrim press: close the modal surface.
    Dismiss,
    LauncherInput(String),
    /// A `triton msg …` line.
    Cmd(String),
    /// The bell was pressed on this output: toggle the notification center.
    Bell(String),
    /// The center's DND toggle.
    Dnd,
    /// The pointer entered an island's chrome (its edge or the island itself).
    BarEnter(SurfaceKey),
    /// The pointer left it.
    BarExit(SurfaceKey),
    /// The reveal dwell elapsed; show the island if the pointer is still there.
    BarReveal(SurfaceKey),
    /// The hide delay elapsed; hide the island if the pointer is still away.
    BarHide(SurfaceKey),
    /// `triton msg bar toggle`: flip the bar on this output.
    BarToggle(String),
    /// `triton msg bar mode …`: how the bar presents.
    BarSetMode(BarMode),
    /// A new services snapshot: status group, QS pages, OSD sources.
    Services(Arc<services::State>),
    /// A `notify` module event: a notification arrived or closed.
    Notify(crate::notify::Event),
    /// The OSD's hold elapsed; hides only if the same OSD is still up (the
    /// generation bumps on every show).
    OsdHide(u64),
    /// A notification expired in the store.
    NotifExpired(u32),
    /// A polkit agent event (a prompt opened or was cancelled).
    Polkit(crate::polkit::Event),
    /// An ext-idle-notify event from `wayl`.
    Idle(crate::wayl::IdleEvent),
    /// A launcher search finished on its thread.
    Launch(Arc<crate::launch::Results>),
    /// A result row was pressed, or Enter picked the top hit.
    LaunchHit(crate::launch::HitKind),
    /// Enter in the launcher field: activate the first hit (or the math row).
    LauncherEnter,
    /// Wallpaper load + scheme pipeline finished on its thread.
    Pipeline(Arc<PipelineDone>),
    /// The battery-low OSD's action: power saver on.
    OsdSaver,
    /// A power-menu action was picked (click or digit jump); Enter re-picks the
    /// focused one through `PowerConfirm`.
    PowerPick(crate::panels::PowerAction),
    PowerToggle,
    PowerMove(i8),
    PowerConfirm,
    /// Quick settings.
    QsPage(crate::panels::QsPage),
    QsToggleWifi,
    QsToggleBt,
    QsToggleNight,
    QsToggleDark,
    QsToggleSaver,
    QsBrightness(f32),
    QsBrightnessCommit,
    QsVolume(f32),
    QsVolumeCommit,
    QsMicMute,
    QsAllSettings,
    /// A Bluetooth device row: connect it, or disconnect the connected one.
    BtDevice {
        address: String,
        connected: bool,
    },
    /// Notification center / toasts.
    NotifActivate(u32),
    NotifGroupToggle(String),
    NotifClearApp(String),
    /// Polkit dialog.
    PolkitInput(String),
    PolkitSubmit,
    PolkitCancel,
}

/// Idle timeouts the shell registers (`wayl::idles`): stage 1 locks, 2 suspends.
/// Stage 0 (dim) is reserved for a gamma dim that needs `wayl` first.
const IDLE_TIMEOUTS: [std::time::Duration; 2] = [
    std::time::Duration::from_mins(5),
    std::time::Duration::from_mins(10),
];

/// The `Power` board's time-of-day greeting.
fn greeting() -> String {
    use chrono::Timelike;
    let user = std::env::var("USER").unwrap_or_else(|_| "there".into());
    let hello = match chrono::Local::now().hour() {
        5..=11 => "Good morning",
        12..=17 => "Good afternoon",
        _ => "Good evening",
    };
    format!("{hello}, {user}")
}

/// "Up 3 days, 4 hours" from /proc/uptime (first field, seconds).
fn uptime() -> String {
    let secs: u64 = std::fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|s| {
            s.split_whitespace()
                .next()
                .and_then(|n| n.parse::<f64>().ok())
        })
        .map_or(0, |n| n as u64);
    let (d, h) = (secs / 86400, secs % 86400 / 3600);
    match (d, h) {
        (0, 0) => "Up just now".into(),
        (0, h) => format!("Up {h}h"),
        (d, h) => format!("Up {d}d {h}h"),
    }
}

/// "now" / "12 min" / "3 h" — the card's relative time.
fn ago(when: std::time::Instant) -> String {
    match when.elapsed().as_secs() {
        s if s < 60 => "now".into(),
        s if s < 3600 => format!("{} min", s / 60),
        s => format!("{} h", s / 3600),
    }
}

/// The shell's switches outnumber clippy's bool budget by design — each is a
/// different surface's visibility or a user's toggle.
#[expect(clippy::struct_excessive_bools)]
#[derive(Debug, Default)]
pub struct Shell {
    niri: Arc<Snapshot>,
    surfaces: SurfaceTable,
    /// Measured extents of content-sized surfaces (island widths, panel heights).
    sizes: Sizes,
    /// The output the launcher is open on, if any.
    launcher: Option<String>,
    /// The output the notification center is open on, if any.
    notifications: Option<String>,
    /// The output quick settings is open on, if any.
    quick_settings: Option<String>,
    /// The centered modal (power menu or polkit) and its output.
    centered: Option<(CenteredRole, String)>,
    /// Do Not Disturb; the bell and the center's toggle show it.
    dnd: bool,
    /// The OSD showing right now, and its hold deadline.
    osd: Option<crate::panels::Osd>,
    /// Centered power menu: the focused action's index into `POWER_ACTIONS`.
    power_focus: usize,
    /// The quick settings page showing.
    qs_page: crate::panels::QsPage,
    /// Wallpaper drawn on every output's background surface, if one is set.
    wallpaper: Option<iced_widget::image::Handle>,
    /// Mean wallpaper luminance → Mica's adaptive tint.
    wallpaper_luminance: f32,
    /// The active color variant; `qs` dark tile and the pipeline set it.
    variant: triton_design::Variant,
    /// The live scheme (pipeline output); `None` keeps the token reference.
    scheme: Option<triton_design::Scheme>,
    /// Notification store (filled by `notify` once its server lands).
    notifs: crate::notify::Store,
    /// Apps expanded in the notification center.
    notif_expanded: HashSet<String>,
    /// Polkit prompt waiting for a password, if any.
    polkit: Option<crate::polkit::Prompt>,
    polkit_password: String,
    /// Night light is on (gamma goes warm).
    night: bool,
    /// Idle-notification timeouts index → what fires at it (see `idles`).
    idle_stage: usize,
    /// Bumped on every `show_osd`; stale hide timers check it.
    osd_gen: u64,
    /// The launcher's last search results.
    results: Option<Arc<crate::launch::Results>>,
    /// Desktop entries, scanned lazily on the launcher's first search.
    index: Option<crate::launch::Index>,
    /// Live slider values (applied on release).
    volume: f32,
    brightness: f32,
    /// Wi-Fi networks and Bluetooth devices for the QS pages (service-fed).
    aps: Vec<(String, String, bool, bool)>,
    bt_devices: Vec<(String, String, String, bool)>,
    /// The last merged services snapshot (wifi/bt toggles read it).
    svc: Option<Arc<services::State>>,
    /// wlr-gamma-control handle; `night` toggles the temperature on it.
    night_gamma: Option<crate::wayl::Night>,
    /// Power saver is on (also fed by the services snapshot once it lands).
    saver: bool,
    /// How the bar presents (`BarStates`); `triton msg bar mode …` sets it.
    bar_mode: BarMode,
    /// Islands that are up right now (ignored while pinned). Keys carry the
    /// island roles — the two ends come and go independently.
    bar_shown: HashSet<SurfaceKey>,
    /// Islands whose chrome (edge or surface) the pointer is over.
    bar_hover: HashSet<SurfaceKey>,
    /// The launcher's query text.
    query: String,
    time: String,
    date: String,
    /// The power menu's greeting and uptime, refreshed on the minute tick.
    greeting: String,
    uptime: String,
    /// Battery + connectivity for the status group, from `services`.
    statuses: Statuses,
    /// `--real-services`: watch real UPower/NetworkManager, not the fake.
    real_services: bool,
}

impl Shell {
    pub fn boot(real_services: bool) -> (Self, Task<Message>) {
        let mut shell = Self {
            real_services,
            night_gamma: crate::wayl::start(),
            volume: 0.62,
            brightness: 0.7,
            ..Self::default()
        };
        shell.tick();
        let task = crate::pipeline::current_wallpaper()
            .map_or_else(Task::none, |path| shell.set_wallpaper(path));
        (shell, task)
    }

    fn tick(&mut self) {
        let now = chrono::Local::now();
        self.time = now.format("%H:%M").to_string();
        self.date = now.format("%a %-d %b").to_string();
        self.greeting = greeting();
        self.uptime = uptime();
    }

    /// The message enum is wide by design; each arm stays small.
    #[expect(
        clippy::too_many_lines,
        reason = "one arm per message; splitting hides the table"
    )]
    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Niri(snapshot) => {
                self.niri = snapshot;
                self.reconcile()
            }
            Message::Measured(id, dim, extent) => {
                let Some(key) = self.surfaces.key(id).cloned() else {
                    return Task::none();
                };
                let map = match dim {
                    Dim::Width => &mut self.sizes.widths,
                    Dim::Height => &mut self.sizes.heights,
                };
                map.insert(key, extent.ceil() as u32);
                self.reconcile()
            }
            Message::Minute => {
                self.tick();
                Task::none()
            }
            Message::Services(state) => {
                self.statuses.clone_from(&state.statuses);
                self.volume = f32::from(state.audio.volume).min(150.0) / 100.0;
                if let Some(b) = state.brightness {
                    self.brightness = f32::from(b) / 100.0;
                }
                self.aps = state
                    .net
                    .aps
                    .iter()
                    .map(|ap| {
                        (
                            ap.ssid.clone(),
                            format!(
                                "{}%{}",
                                ap.strength,
                                if ap.secured { " · secured" } else { "" }
                            ),
                            ap.secured,
                            ap.connected,
                        )
                    })
                    .collect();
                self.bt_devices = state
                    .bt
                    .devices
                    .iter()
                    .map(|d| {
                        let status = if d.connected {
                            "Connected"
                        } else if d.paired {
                            "Paired"
                        } else {
                            "Available"
                        };
                        (
                            d.name.clone(),
                            status.to_owned(),
                            d.address.clone(),
                            d.connected,
                        )
                    })
                    .collect();
                self.saver = state.profile == services::Profile::PowerSaver;
                self.svc = Some(state);
                Task::none()
            }
            Message::Closed(id) => {
                self.surfaces.forget(id);
                // Reconciled again (ADR-0003): recreates it if still desired.
                self.reconcile()
            }
            Message::FocusWorkspace(id) => {
                niri::focus_workspace(id);
                Task::none()
            }
            Message::Launcher(output) => {
                if self.launcher.as_deref() == Some(output.as_str()) {
                    self.launcher = None;
                    self.reconcile()
                } else {
                    self.query.clear();
                    self.launcher = Some(output);
                    let t = self.search();
                    Task::batch([t, self.reconcile()])
                }
            }
            Message::Dismiss => {
                if let Some((CenteredRole::Polkit, _)) = self.centered {
                    if let Some(p) = self.polkit.take() {
                        crate::polkit::cancel(&p.cookie);
                    }
                    self.polkit_password.clear();
                }
                let closed = self.launcher.take().is_some()
                    | self.notifications.take().is_some()
                    | self.quick_settings.take().is_some()
                    | self.centered.take().is_some();
                if !closed {
                    return Task::none();
                }
                self.reconcile()
            }
            Message::Opened(id) => match self.surfaces.key(id) {
                // Its UI exists now: put the caret in the field.
                Some(key) if key.role == Role::Launcher => {
                    iced_runtime::task::widget(focusable::focus(launcher_field()))
                }
                Some(key) if key.role == Role::Polkit => {
                    iced_runtime::task::widget(focusable::focus(polkit_field()))
                }
                _ => Task::none(),
            },
            Message::LauncherInput(query) => {
                self.query = query;
                self.search()
            }
            Message::Cmd(line) => self.command(&line),
            Message::Bell(output) => {
                self.notifications = if self.notifications.as_deref() == Some(output.as_str()) {
                    None
                } else {
                    Some(output)
                };
                self.reconcile()
            }
            Message::Dnd => {
                self.dnd = !self.dnd;
                Task::none()
            }
            Message::Notify(event) => match event {
                crate::notify::Event::Arrived(n) => {
                    let id = n.id;
                    let hold = n.timeout;
                    self.notifs.upsert(n);
                    Self::after(hold, Message::NotifExpired(id))
                }
                crate::notify::Event::Closed(id) => {
                    self.notifs.close(id);
                    self.reconcile()
                }
            },
            Message::NotifExpired(id) => {
                self.notifs.close(id);
                crate::notify::closed(id, 1);
                self.reconcile()
            }
            Message::NotifActivate(id) => {
                self.notifs.close(id);
                crate::notify::action_invoked(id, "default");
                crate::notify::closed(id, 2);
                self.reconcile()
            }
            Message::NotifGroupToggle(app) => {
                if !self.notif_expanded.remove(&app) {
                    self.notif_expanded.insert(app);
                }
                Task::none()
            }
            Message::NotifClearApp(app) => {
                self.notifs.clear_app(&app);
                self.reconcile()
            }
            Message::OsdHide(g) => {
                if g == self.osd_gen {
                    self.osd = None;
                    return self.reconcile();
                }
                Task::none()
            }
            Message::OsdSaver => {
                self.power_saver(true);
                Task::none()
            }
            Message::PowerToggle => self.toggle_centered(CenteredRole::Power),
            Message::PowerMove(dir) => {
                let n = crate::panels::POWER_ACTIONS.len() as i8;
                self.power_focus = ((self.power_focus as i8 + dir).rem_euclid(n)) as usize;
                Task::none()
            }
            Message::PowerPick(action) => {
                self.power_focus = crate::panels::POWER_ACTIONS
                    .iter()
                    .position(|a| *a == action)
                    .unwrap_or(0);
                self.power_confirm()
            }
            Message::PowerConfirm => self.power_confirm(),
            Message::QsPage(page) => {
                self.qs_page = page;
                if page == crate::panels::QsPage::Wifi {
                    services::wifi_rescan();
                }
                self.reconcile()
            }
            Message::BtDevice { address, connected } => {
                if connected {
                    services::bt_disconnect(&address);
                } else {
                    services::bt_connect(&address);
                }
                Task::none()
            }
            Message::QsToggleWifi => {
                services::set_wifi(!self.svc_wifi());
                Task::none()
            }
            Message::QsToggleBt => {
                services::bt_power(!self.svc_bt());
                Task::none()
            }
            Message::QsToggleNight => {
                self.night = !self.night;
                self.apply_night();
                Task::none()
            }
            Message::QsToggleDark => {
                self.variant = match self.variant {
                    triton_design::Variant::Dark => triton_design::Variant::Light,
                    triton_design::Variant::Light => triton_design::Variant::Dark,
                };
                self.apply_scheme();
                Task::none()
            }
            Message::QsToggleSaver => {
                self.power_saver(!self.saver);
                Task::none()
            }
            Message::QsBrightness(v) => {
                self.brightness = v;
                Task::none()
            }
            Message::QsBrightnessCommit => {
                services::set_brightness(self.brightness * 100.0);
                Task::none()
            }
            Message::QsVolume(v) => {
                self.volume = v;
                Task::none()
            }
            Message::QsVolumeCommit => {
                services::set_volume(self.volume * 100.0);
                Task::none()
            }
            Message::QsMicMute => {
                services::set_mic_mute(true);
                Task::none()
            }
            Message::QsAllSettings => {
                // Phase 3: open triton-settings on the right page.
                tracing::info!("quick settings hand-off waits for triton-settings");
                Task::none()
            }
            Message::Polkit(event) => match event {
                crate::polkit::Event::Prompt(prompt) => {
                    self.polkit_password.clear();
                    let output = self.focused_output().unwrap_or_default();
                    self.polkit = Some(prompt);
                    self.centered = Some((CenteredRole::Polkit, output));
                    self.reconcile()
                }
                crate::polkit::Event::Cancelled { cookie } => {
                    if self.polkit.as_ref().is_some_and(|p| p.cookie == cookie) {
                        self.polkit = None;
                        self.polkit_password.clear();
                        self.centered = None;
                        return self.reconcile();
                    }
                    Task::none()
                }
            },
            Message::PolkitInput(value) => {
                self.polkit_password = value;
                Task::none()
            }
            Message::PolkitSubmit => {
                if let Some(p) = self.polkit.take() {
                    crate::polkit::respond(&p.cookie, &p.identity, self.polkit_password.clone());
                    self.polkit_password.clear();
                    self.centered = None;
                    self.reconcile()
                } else {
                    Task::none()
                }
            }
            Message::PolkitCancel => {
                if let Some(p) = self.polkit.take() {
                    crate::polkit::cancel(&p.cookie);
                    self.polkit_password.clear();
                    self.centered = None;
                    return self.reconcile();
                }
                Task::none()
            }
            Message::Idle(event) => self.idle(event),
            Message::Launch(results) => {
                self.results = Some(results);
                Task::none()
            }
            Message::LaunchHit(kind) => self.activate_hit(&kind),
            Message::LauncherEnter => {
                let first = self
                    .results
                    .as_ref()
                    .and_then(|r| r.groups.first())
                    .and_then(|(_, hits)| hits.first())
                    .map(|h| h.kind.clone());
                match first {
                    Some(kind) => self.activate_hit(&kind),
                    None => Task::none(),
                }
            }
            Message::Pipeline(done) => {
                self.wallpaper.clone_from(&done.wallpaper);
                self.wallpaper_luminance = done.luminance;
                self.scheme = Some(match self.variant {
                    triton_design::Variant::Dark => done.dark,
                    triton_design::Variant::Light => done.light,
                });
                if !done.applied.is_empty() {
                    let targets = done.applied.join(", ");
                    self.toast(format!("Scheme applied to {targets}"));
                }
                self.reconcile()
            }
            // Bar hover and mode plumbing lives in `bar`.
            Message::BarEnter(_)
            | Message::BarExit(_)
            | Message::BarReveal(_)
            | Message::BarHide(_)
            | Message::BarToggle(_)
            | Message::BarSetMode(_) => self.bar(message),
            // The layer-shell variants are consumed by iced_layershell itself.
            _ => Task::none(),
        }
    }

    /// The bar's state machine (`BarStates`): edge hover and the dwell timers
    /// drive `bar_shown`; `bar toggle` flips it directly; `bar mode` re-arms it.
    fn bar(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::BarEnter(output) => {
                self.bar_hover.insert(output.clone());
                Self::after(REVEAL_DELAY, Message::BarReveal(output))
            }
            Message::BarExit(output) => {
                self.bar_hover.remove(&output);
                Self::after(HIDE_DELAY, Message::BarHide(output))
            }
            Message::BarReveal(output) => {
                if self.bar_mode == BarMode::AutoHide
                    && self.bar_hover.contains(&output)
                    && self.bar_shown.insert(output)
                {
                    return self.reconcile();
                }
                Task::none()
            }
            Message::BarHide(output) => {
                if self.bar_mode == BarMode::AutoHide
                    && !self.bar_hover.contains(&output)
                    && self.bar_shown.remove(&output)
                {
                    return self.reconcile();
                }
                Task::none()
            }
            Message::BarToggle(output) => {
                // Toggle flips the bar as a whole: anything up hides both ends.
                let key = |role| SurfaceKey {
                    role,
                    output: output.clone(),
                };
                if self.bar_shown.contains(&key(Role::IslandStart))
                    || self.bar_shown.contains(&key(Role::IslandEnd))
                {
                    self.bar_shown.remove(&key(Role::IslandStart));
                    self.bar_shown.remove(&key(Role::IslandEnd));
                } else {
                    self.bar_shown.insert(key(Role::IslandStart));
                    self.bar_shown.insert(key(Role::IslandEnd));
                }
                self.reconcile()
            }
            Message::BarSetMode(mode) => {
                self.bar_mode = mode;
                if mode == BarMode::Pinned {
                    self.bar_shown.clear();
                }
                self.reconcile()
            }
            _ => Task::none(),
        }
    }

    /// Flip `role` on/off on the focused output (centered modals only).
    fn toggle_centered(&mut self, role: CenteredRole) -> Task<Message> {
        if let Some((r, _)) = self.centered
            && std::mem::discriminant(&r) == std::mem::discriminant(&role)
        {
            self.centered = None;
            return self.reconcile();
        }
        let Some(output) = self.focused_output() else {
            return Task::none();
        };
        self.centered = Some((role, output));
        if role == CenteredRole::Power {
            self.power_focus = 0;
        }
        self.reconcile()
    }

    /// The output the keyboard is on (niri's focused output; falls back to the
    /// first for the one-output preview).
    fn focused_output(&self) -> Option<String> {
        self.niri
            .focus_output()
            .or_else(|| self.niri.outputs.first().cloned())
    }

    /// Send `message` after `delay`; the bar's dwell timers and the OSD hold.
    fn after(delay: std::time::Duration, message: Message) -> Task<Message> {
        Task::perform(tokio::time::sleep(delay), move |()| message)
    }

    /// Run the focused power action (`Power` board: Enter confirms).
    fn power_confirm(&mut self) -> Task<Message> {
        self.centered = None;
        match crate::panels::POWER_ACTIONS[self.power_focus] {
            crate::panels::PowerAction::Lock => Self::lock(),
            crate::panels::PowerAction::Sleep => services::suspend(),
            crate::panels::PowerAction::LogOut => niri::quit(),
            crate::panels::PowerAction::Restart => services::reboot(),
            crate::panels::PowerAction::ShutDown => services::power_off(),
        }
        self.reconcile()
    }

    /// Spawn `triton-lock` next to this binary (or on PATH) — a crash there
    /// must not take the shell down (roadmap §3).
    fn lock() {
        let exe = std::env::current_exe().ok();
        let sib = exe
            .as_deref()
            .and_then(|e| e.parent())
            .map(|p| p.join("triton-lock"));
        let prog = sib
            .filter(|p| p.exists())
            .unwrap_or_else(|| std::path::PathBuf::from("triton-lock"));
        if let Err(e) = std::process::Command::new(&prog).spawn() {
            tracing::warn!(path = %prog.display(), %e, "cannot spawn triton-lock");
        }
    }

    /// Show the OSD and schedule its hide (`OSD` board: replaces the current one).
    fn show_osd(&mut self, osd: crate::panels::Osd) -> Task<Message> {
        let hold = osd.hold();
        self.osd_gen += 1;
        self.osd = Some(osd);
        let g = self.osd_gen;
        Task::batch([Self::after(hold, Message::OsdHide(g)), self.reconcile()])
    }

    /// A synthesized notification — pipelines and demos feed this ("Triton").
    fn toast(&mut self, summary: String) {
        self.notifs.upsert(crate::notify::Notification {
            id: notif_gen(),
            app_name: "Triton".to_owned(),
            icon: None,
            summary,
            body: String::new(),
            actions: Vec::new(),
            urgency: crate::notify::Urgency::Normal,
            timeout: std::time::Duration::from_secs(6),
            arrived: std::time::Instant::now(),
        });
    }

    /// What happens at each idle stage (roadmap: dim → lock → suspend). The
    /// timeouts come from `wayl::idles` (`IDLE_TIMEOUTS`); resume resets.
    fn idle(&mut self, event: crate::wayl::IdleEvent) -> Task<Message> {
        match event {
            crate::wayl::IdleEvent::Idle(1) => {
                self.idle_stage = 1;
                Self::lock();
                Task::none()
            }
            crate::wayl::IdleEvent::Idle(i) if i >= 2 => {
                self.idle_stage = 2;
                services::suspend();
                Task::none()
            }
            crate::wayl::IdleEvent::Idle(_) | crate::wayl::IdleEvent::Resumed(_) => {
                self.idle_stage = 0;
                Task::none()
            }
        }
    }

    /// Turn power saver on/off; the services module drives the real backend.
    fn power_saver(&mut self, on: bool) {
        self.saver = on;
        services::set_profile(if on {
            services::Profile::PowerSaver
        } else {
            services::Profile::Balanced
        });
    }

    /// ~4500 K when the toggle is on; 6500 K is the neutral daylight point.
    fn apply_night(&self) {
        if let Some(night) = &self.night_gamma {
            night.set_temperature(if self.night { 4500 } else { 6500 });
        }
    }

    /// Re-render the scheme-dependent files (niri colors, gtk css) and let
    /// `theme` pick `scheme`/`variant` up on the next frame.
    fn apply_scheme(&self) {
        let scheme = self
            .scheme
            .unwrap_or_else(|| triton_design::Scheme::reference(self.variant));
        if let Err(e) = crate::niri_config::install(&scheme) {
            tracing::warn!(%e, "cannot re-write niri colors");
        }
        crate::pipeline::apply(&scheme, self.variant);
    }

    fn svc_wifi(&self) -> bool {
        self.svc.as_ref().is_some_and(|s| s.net.wifi_enabled)
    }
    fn svc_bt(&self) -> bool {
        self.svc.as_ref().is_some_and(|s| s.bt.powered)
    }

    /// Run the query through the launch engine. Cheap enough to stay
    /// synchronous: a hundred entries against a subsequence scorer.
    fn search(&mut self) -> Task<Message> {
        if self.index.is_none() {
            self.index = Some(crate::launch::Index::scan());
        }
        let windows: Vec<crate::launch::WindowRef> = self
            .niri
            .windows
            .values()
            .map(|w| crate::launch::WindowRef {
                id: w.id,
                title: w.title.clone().unwrap_or_default(),
                app_id: w.app_id.clone(),
                workspace: None,
            })
            .collect();
        self.results = Some(Arc::new(crate::launch::search(
            self.index.as_ref().expect("scanned above"),
            &windows,
            LAUNCH_COMMANDS,
            &self.query,
        )));
        Task::none()
    }

    /// Activate a result (`Message::LaunchHit`): apps spawn, windows focus,
    /// commands run as `triton msg`, math copies its result.
    fn activate_hit(&mut self, kind: &crate::launch::HitKind) -> Task<Message> {
        match kind {
            crate::launch::HitKind::App { app, action } => {
                if let Some(index) = &self.index
                    && let Err(e) = crate::launch::launch_app(index, *app, *action)
                {
                    tracing::warn!(%e, "launch failed");
                }
            }
            crate::launch::HitKind::Window { id } => niri::focus_window(*id),
            crate::launch::HitKind::Command(i) => {
                if let Some(cmd) = LAUNCH_COMMANDS.get(*i) {
                    return self.command(cmd.id);
                }
            }
            crate::launch::HitKind::Math { result, .. } => {
                copy_text(result.clone());
            }
        }
        self.launcher = None;
        self.reconcile()
    }

    /// One `triton msg` line, on the focused output where no output is named.
    #[expect(
        clippy::too_many_lines,
        reason = "the command table is flat and reads as one"
    )]
    fn command(&mut self, line: &str) -> Task<Message> {
        let words: Vec<_> = line.split_whitespace().collect();
        let mut toggle = |message: fn(String) -> Message| match self.focused_output() {
            Some(output) => self.update(message(output)),
            None => Task::none(),
        };
        match words.as_slice() {
            ["launcher", "toggle"] => toggle(Message::Launcher),
            ["notifications", "toggle"] => toggle(Message::Bell),
            ["quicksettings", "toggle"] => {
                self.quick_settings = match self.quick_settings.take() {
                    Some(_) => None,
                    _ => self.focused_output(),
                };
                self.reconcile()
            }
            ["power", "toggle"] => self.update(Message::PowerToggle),
            ["bar", "toggle"] => toggle(Message::BarToggle),
            ["bar", "mode", mode] => match *mode {
                "autohide" => self.update(Message::BarSetMode(BarMode::AutoHide)),
                "pinned" => self.update(Message::BarSetMode(BarMode::Pinned)),
                "hidden" => self.update(Message::BarSetMode(BarMode::Hidden)),
                other => {
                    tracing::warn!(?other, "msg: unknown bar mode");
                    Task::none()
                }
            },
            ["lock"] => {
                Self::lock();
                Task::none()
            }
            ["suspend"] => {
                services::suspend();
                Task::none()
            }
            ["volume", arg] => match *arg {
                "up" => self.volume_step(0.05),
                "down" => self.volume_step(-0.05),
                "mute" => {
                    services::set_mute(true);
                    self.show_osd(crate::panels::Osd::Volume {
                        percent: (self.volume * 100.0) as u8,
                        muted: true,
                    })
                }
                set => set.parse::<u8>().ok().map_or_else(
                    || {
                        tracing::warn!(?set, "msg: unknown volume arg");
                        Task::none()
                    },
                    |v| self.volume_set(v),
                ),
            },
            ["brightness", arg] => match *arg {
                "up" => self.brightness_step(0.05),
                "down" => self.brightness_step(-0.05),
                set => set.parse::<u8>().ok().map_or_else(
                    || {
                        tracing::warn!(?set, "msg: unknown brightness arg");
                        Task::none()
                    },
                    |v| self.brightness_set(v),
                ),
            },
            ["mic", "mute"] => {
                services::set_mic_mute(true);
                self.show_osd(crate::panels::Osd::MicMuted)
            }
            ["dnd", "toggle"] => self.update(Message::Dnd),
            ["nightlight", "toggle"] => self.update(Message::QsToggleNight),
            ["wallpaper", rest @ ..] => {
                let path = rest.join(" ");
                if path.is_empty() {
                    Task::none()
                } else {
                    self.set_wallpaper(std::path::PathBuf::from(path))
                }
            }
            ["notify", rest @ ..] => {
                let text = rest.join(" ");
                if !text.is_empty() {
                    self.toast(text);
                    return self.reconcile();
                }
                Task::none()
            }
            ["osd", kind @ ..] => match kind {
                ["volume"] => self.show_osd(crate::panels::Osd::Volume {
                    percent: (self.volume * 100.0) as u8,
                    muted: false,
                }),
                ["brightness"] => self.show_osd(crate::panels::Osd::Brightness {
                    percent: (self.brightness * 100.0) as u8,
                }),
                ["mic"] => self.show_osd(crate::panels::Osd::MicMuted),
                ["layout"] => self.show_osd(crate::panels::Osd::Layout {
                    code: "DE".into(),
                    name: "Deutsch".into(),
                }),
                ["battery"] => self.show_osd(crate::panels::Osd::Battery {
                    percent: 18,
                    left: "≈ 45 min left".into(),
                }),
                other => {
                    tracing::warn!(?other, "msg: unknown osd kind");
                    Task::none()
                }
            },
            ["polkit", "demo"] => {
                self.polkit_password.clear();
                self.polkit = Some(crate::polkit::Prompt {
                    cookie: "demo".into(),
                    action_id: "org.triton.demo".into(),
                    message: "Authentication is required to demo the dialog".into(),
                    icon: None,
                    user: std::env::var("USER").unwrap_or_else(|_| "user".into()),
                    identity: String::new(),
                });
                self.centered = Some((
                    CenteredRole::Polkit,
                    self.focused_output().unwrap_or_default(),
                ));
                self.reconcile()
            }
            other => {
                tracing::warn!(?other, "msg: unknown command");
                Task::none()
            }
        }
    }

    /// Nudge the volume by a step and show the OSD (`msg volume up|down`).
    fn volume_step(&mut self, step: f32) -> Task<Message> {
        self.volume = (self.volume + step).clamp(0.0, 1.5);
        services::set_volume(self.volume * 100.0);
        self.show_osd(crate::panels::Osd::Volume {
            percent: (self.volume * 100.0) as u8,
            muted: false,
        })
    }

    fn volume_set(&mut self, percent: u8) -> Task<Message> {
        self.volume = (f32::from(percent) / 100.0).clamp(0.0, 1.5);
        services::set_volume(self.volume * 100.0);
        self.show_osd(crate::panels::Osd::Volume {
            percent,
            muted: false,
        })
    }

    fn brightness_step(&mut self, step: f32) -> Task<Message> {
        self.brightness = (self.brightness + step).clamp(0.01, 1.0);
        services::set_brightness(self.brightness * 100.0);
        self.show_osd(crate::panels::Osd::Brightness {
            percent: (self.brightness * 100.0) as u8,
        })
    }

    fn brightness_set(&mut self, percent: u8) -> Task<Message> {
        self.brightness = f32::from(percent.max(1)) / 100.0;
        services::set_brightness(self.brightness * 100.0);
        self.show_osd(crate::panels::Osd::Brightness {
            percent: percent.max(1),
        })
    }

    /// Kick the wallpaper→scheme pipeline on a worker thread for `path`.
    fn set_wallpaper(&mut self, path: std::path::PathBuf) -> Task<Message> {
        let variant = self.variant;
        Task::perform(
            async move { crate::pipeline::run(&path, variant) },
            |done| Message::Pipeline(Arc::new(done)),
        )
    }

    fn reconcile(&mut self) -> Task<Message> {
        let outputs = self.niri.outputs.iter().map(String::as_str);
        let desired = shell::desired(
            outputs,
            &self.sizes,
            &Modals {
                launcher: self.launcher.as_deref(),
                notifications: self.notifications.as_deref(),
                quick_settings: self.quick_settings.as_deref(),
                centered: self.centered.as_ref().map(|(r, o)| match r {
                    CenteredRole::Power => Centered::Power(o.as_str()),
                    CenteredRole::Polkit => Centered::Polkit(o.as_str()),
                }),
            },
            self.bar_mode,
            |k| self.bar_shown.contains(k),
        );
        let has_toasts = !self
            .notifs
            .toasts(std::time::Instant::now(), self.dnd)
            .is_empty();
        let mut ambient = shell::ambient(
            self.niri.outputs.iter().map(String::as_str),
            &self.sizes,
            self.focused_output().as_deref(),
            self.osd.is_some(),
            has_toasts,
            |k| self.bar_shown.contains(k),
            self.bar_mode == BarMode::Pinned,
        );
        ambient.extend(desired);
        self.surfaces.reconcile(&ambient)
    }

    pub fn view(&self, id: Id) -> Element<'_, Message> {
        let Some(key) = self.surfaces.key(id) else {
            return Space::new().into();
        };
        let content = match key.role {
            Role::IslandStart => self.bar_start(&key.output),
            Role::IslandEnd => self.bar_end(&key.output),
            Role::Strip => return self.strip(&key.output),
            Role::RevealStart => return Self::reveal(key, Role::IslandStart),
            Role::RevealEnd => return Self::reveal(key, Role::IslandEnd),
            Role::Scrim => return Self::scrim(),
            Role::Launcher => return self.launcher(),
            Role::Notifications => {
                return measure_height(self.notifications(), move |d, h| {
                    Message::Measured(id, d, h)
                })
                .into();
            }
            Role::QuickSettings => {
                return measure_height(self.quick_settings_view(), move |d, h| {
                    Message::Measured(id, d, h)
                })
                .into();
            }
            Role::PowerMenu => {
                return measure(
                    measure_height(self.power_menu_view(), {
                        move |d, h| Message::Measured(id, d, h)
                    }),
                    move |d, w| Message::Measured(id, d, w),
                )
                .into();
            }
            Role::Polkit => {
                return measure_height(
                    crate::panels::polkit_dialog(
                        self.polkit
                            .as_ref()
                            .map_or("Authentication required", |p| p.message.as_str()),
                        self.polkit.as_ref().map_or("", |p| p.user.as_str()),
                        &self.polkit_password,
                    ),
                    move |d, h| Message::Measured(id, d, h),
                )
                .into();
            }
            Role::Toasts => {
                return measure_height(self.toasts_view(), move |d, h| Message::Measured(id, d, h))
                    .into();
            }
            Role::Osd => {
                return measure(
                    measure_height(self.osd_view(), move |d, h| Message::Measured(id, d, h)),
                    move |d, w| Message::Measured(id, d, w),
                )
                .into();
            }
            Role::Wallpaper => {
                let img: Element<'_, Message> = match &self.wallpaper {
                    Some(handle) => iced_widget::image(handle)
                        .content_fit(iced_core::ContentFit::Cover)
                        .width(Length::Fill)
                        .height(Length::Fill)
                        .into(),
                    None => Space::new().width(Length::Fill).height(Length::Fill).into(),
                };
                return surface::surface(img, Surface::Material(Material::Solid))
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into();
            }
        };
        let island = measure(layer_island(content), move |d, w| {
            Message::Measured(id, d, w)
        });
        // In autohide the islands double as hover region: pointer timing here
        // keeps them up and puts them away.
        if self.bar_mode == BarMode::AutoHide {
            let k = key.clone();
            return mouse_area(island)
                .on_enter(Message::BarEnter(k.clone()))
                .on_exit(Message::BarExit(k))
                .into();
        }
        island.into()
    }

    /// The pinned bar: the same parts, laid flat in one Mica strip.
    fn strip<'a>(&'a self, output: &'a str) -> Element<'a, Message> {
        surface::surface(
            Row::new()
                .spacing(bar::GAP)
                .align_y(Vertical::Center)
                .push(self.bar_start(output))
                .push(Space::new().width(Length::Fill))
                .push(self.bar_end(output)),
            Surface::Material(Material::Mica),
        )
        .padding(Padding::from([0.0, 16.0]))
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }

    /// The 2 px wake edge: nothing drawn, only hover. It stands in for `island`
    /// — hover and dwell timers are tracked against the island's key.
    fn reveal(key: &SurfaceKey, island: Role) -> Element<'static, Message> {
        let k = SurfaceKey {
            role: island,
            output: key.output.clone(),
        };
        mouse_area(Space::new().width(Length::Fill).height(Length::Fill))
            .on_enter(Message::BarEnter(k.clone()))
            .on_exit(Message::BarExit(k))
            .into()
    }

    /// The transparent surface a press lands on to dismiss the modal (`Scrim`
    /// covers the output; the panel above it keeps its own presses).
    fn scrim() -> Element<'static, Message> {
        mouse_area(
            surface::surface(
                Space::new().width(Length::Fill).height(Length::Fill),
                Surface::Scrim,
            )
            .width(Length::Fill)
            .height(Length::Fill),
        )
        .on_press(Message::Dismiss)
        .into()
    }

    /// The `Launcher` board's main state, as plumbing: the field and the layout
    /// it will live in are real, the results are placeholders until search lands.
    fn launcher(&self) -> Element<'_, Message> {
        // The field is `LAUNCHER_PAD` inside the 28 px panel: nested corners,
        // not a pill (post-board refinement).
        let field = field::search(
            "Search",
            &self.query,
            Field::Nested {
                outer: triton_design::shape::RADII.floating_window,
                inset: LAUNCHER_PAD,
            },
        )
        .id(launcher_field())
        .on_input(Message::LauncherInput)
        .on_submit(Message::LauncherEnter)
        .size(24.0)
        .padding(Padding::from([22.0, 20.0]));
        let scopes = Row::new()
            .spacing(8)
            .push(chip("All", None, true))
            .push(chip("Apps", None, false))
            .push(chip("Files", None, false))
            .push(chip("Windows", None, false));
        // `=` scope swaps the whole results list for the math card; the top
        // hit is the first hit of the first group, with the app's actions.
        let body: Element<'_, Message> =
            if let crate::launch::Scope::Math(q) = crate::launch::scope(&self.query) {
                match crate::launch::math(q) {
                    Some((expr, result)) => surface::card(
                        Row::new()
                            .spacing(14)
                            .align_y(Vertical::Center)
                            .push(surface::icon_disc(Icon::Calculate))
                            .push(
                                Column::new()
                                    .push(text::text(result.clone(), TypeStyle::TITLE_M))
                                    .push(
                                        text::text(expr.clone(), TypeStyle::BODY_S)
                                            .class(Tone::OnSurfaceVariant),
                                    ),
                            ),
                    )
                    .padding(14)
                    .width(Length::Fill)
                    .into(),
                    None => text::text("Not an expression", TypeStyle::BODY_M)
                        .class(Tone::OnSurfaceVariant)
                        .into(),
                }
            } else {
                let first = self
                    .results
                    .as_ref()
                    .and_then(|r| r.groups.first())
                    .and_then(|(_, hits)| hits.first());
                let mut col = Column::new().spacing(14);
                if let Some(hit) = first {
                    col = col
                        .push(text::eyebrow("Top hit"))
                        .push(self.top_hit_card(hit));
                }
                col.push(launcher_results(
                    self.results.as_deref().unwrap_or(&EMPTY_RESULTS),
                ))
                .into()
            };
        surface::surface(
            Column::new()
                .spacing(14)
                .push(field)
                .push(scopes)
                .push(body)
                .push(launcher_hints()),
            Surface::LayerPanel(Material::Thick),
        )
        .padding(LAUNCHER_PAD)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }

    /// The board's top-hit card: the first result blown up, with the app's
    /// desktop actions as trailing buttons when it has any.
    fn top_hit_card<'a>(&'a self, hit: &'a crate::launch::Hit) -> Element<'a, Message> {
        let mut row = Row::new()
            .spacing(14)
            .align_y(Vertical::Center)
            .push(surface::icon_disc(Icon::Apps))
            .push(
                Column::new()
                    .push(text::text(hit.title.clone(), TypeStyle::TITLE_M))
                    .push(
                        text::text(
                            hit.supporting.clone().unwrap_or_default(),
                            TypeStyle::BODY_S,
                        )
                        .class(Tone::OnSurfaceVariant),
                    ),
            )
            .push(Space::new().width(Length::Fill))
            .push(
                button::button("Open", None, button::Kind::Filled, button::Size::S)
                    .on_press(Message::LaunchHit(hit.kind.clone())),
            );
        // Desktop actions ("New window", "Incognito") become Text buttons.
        if let crate::launch::HitKind::App { app, .. } = hit.kind
            && let Some(app_obj) = self.index.as_ref().and_then(|i| i.apps.get(app))
        {
            for (ai, action) in app_obj.actions.iter().enumerate() {
                row = row.push(
                    button::button(
                        action.name.as_str(),
                        None,
                        button::Kind::Text,
                        button::Size::S,
                    )
                    .on_press(Message::LaunchHit(
                        crate::launch::HitKind::App {
                            app,
                            action: Some(ai),
                        },
                    )),
                );
            }
        }
        surface::card(row).padding(14).width(Length::Fill).into()
    }

    fn bar_start<'a>(&'a self, output: &'a str) -> Element<'a, Message> {
        let mut left = Row::new()
            .spacing(bar::GAP)
            .align_y(Vertical::Center)
            .push(bar::launcher().on_press(Message::Launcher(output.to_owned())));
        for ws in self.niri.workspaces_on(output) {
            let label = ws.name.clone().unwrap_or_else(|| ws.idx.to_string());
            left = left
                .push(bar::workspace(label, ws.is_active).on_press(Message::FocusWorkspace(ws.id)));
        }
        if let Some(window) = self.niri.active_window(output) {
            let title = window
                .title
                .as_deref()
                .or(window.app_id.as_deref())
                .unwrap_or_default();
            left = left.push(bar::focused(title));
        }
        left.into()
    }

    /// The notification center: app groups the `Notifications` board shows.
    fn notifications(&self) -> Element<'_, Message> {
        let groups: Vec<(String, Vec<crate::panels::NotifView>)> = self
            .notifs
            .groups()
            .into_iter()
            .map(|g| {
                (
                    g.app.clone(),
                    g.items.iter().map(|n| self.notif_view(n)).collect(),
                )
            })
            .collect();
        let expanded = self.notif_expanded.clone();
        crate::panels::notification_center(&groups, move |app| expanded.contains(app), self.dnd)
    }

    /// Map a store row onto the card's plain inputs (icon hint → glyph fallback).
    fn notif_view(&self, n: &crate::notify::Notification) -> crate::panels::NotifView {
        let _ = self;
        crate::panels::NotifView {
            id: n.id,
            app: n.app_name.clone(),
            title: n.summary.clone(),
            body: n.body.clone(),
            icon: None,
            critical: n.urgency == crate::notify::Urgency::Critical,
            when: ago(n.arrived),
        }
    }

    /// The toast stack on the focused output (up to four + overflow chip).
    fn toasts_view(&self) -> Element<'_, Message> {
        let list = self
            .notifs
            .toasts(std::time::Instant::now(), self.dnd)
            .into_iter()
            .take(4)
            .map(|n| self.notif_view(n))
            .collect::<Vec<_>>();
        let overflow = self
            .notifs
            .toasts(std::time::Instant::now(), self.dnd)
            .len()
            .saturating_sub(4);
        crate::panels::toasts(list, overflow)
    }

    /// The OSD's content, if one is showing.
    fn osd_view(&self) -> Element<'_, Message> {
        match &self.osd {
            Some(osd) => crate::panels::osd(osd),
            None => Space::new().width(1).height(1).into(),
        }
    }

    /// Quick settings, fed from the live slider values and service flags.
    fn quick_settings_view(&self) -> Element<'_, Message> {
        crate::panels::quick_settings(crate::panels::Qs {
            page: self.qs_page,
            wifi_on: self.svc_wifi(),
            wifi_label: if self.svc_wifi() { "On" } else { "Off" },
            bt_on: self.svc_bt(),
            bt_label: if self.svc_bt() { "On" } else { "Off" },
            sound_label: "Speakers",
            night_on: self.night,
            night_label: if self.night { "On" } else { "Off" },
            brightness: self.brightness,
            volume: self.volume,
            dnd: self.dnd,
            dark: self.variant == triton_design::Variant::Dark,
            saver: self.saver,
            aps: &self.aps,
            bt_devices: &self.bt_devices,
        })
    }

    /// The `Power` board, fed the greeting, uptime and focus index.
    fn power_menu_view(&self) -> Element<'_, Message> {
        crate::panels::power_menu(
            &self.greeting,
            &self.uptime,
            crate::panels::POWER_ACTIONS[self.power_focus],
            Some("Unsaved work will be lost"),
        )
    }

    fn bar_end<'a>(&'a self, output: &'a str) -> Element<'a, Message> {
        let mut right = Row::new()
            .spacing(bar::GAP)
            .align_y(Vertical::Center)
            .push(bar::clock(self.time.clone(), self.date.clone()));
        let mut items = Vec::new();
        if let Some(c) = self.statuses.connectivity {
            items.push(Status {
                icon: match c {
                    Connectivity::Online => Icon::Wifi,
                    Connectivity::Offline => Icon::WifiOff,
                },
                label: None,
            });
        }
        if let Some(b) = self.statuses.battery {
            items.push(Status {
                icon: b.icon(),
                label: Some(format!("{}%", b.percent)),
            });
        }
        // Warm colors mean attention: red for a low battery, rust for
        // charging, offline or DND, blue while all is well (`BarStates`).
        let tone = if self.statuses.battery.is_some_and(|b| b.low) {
            StatusTone::Alert
        } else if self.statuses.battery.is_some_and(|b| b.charging)
            || self.statuses.connectivity == Some(Connectivity::Offline)
            || self.dnd
        {
            StatusTone::Warn
        } else {
            StatusTone::Normal
        };
        if let Some(pill) = bar::status(items, tone) {
            right = right.push(pill);
        }
        right
            .push(bar::bell(false, self.dnd).on_press(Message::Bell(output.to_owned())))
            .into()
    }

    pub fn subscription(shell: &Self) -> Subscription<Message> {
        Subscription::batch([
            niri::snapshots().map(Message::Niri),
            niri::minutes().map(|()| Message::Minute),
            msg::commands().map(Message::Cmd),
            services::state(shell.real_services).map(Message::Services),
            iced_runtime::window::close_events().map(Message::Closed),
            crate::notify::server().map(Message::Notify),
            crate::polkit::agent(shell.real_services).map(Message::Polkit),
            crate::wayl::idles(&IDLE_TIMEOUTS).map(Message::Idle),
            event::listen_with(|event, _status, id| match event {
                Event::Window(window::Event::Opened { .. }) => Some(Message::Opened(id)),
                Event::Keyboard(keyboard::Event::KeyPressed { key, .. }) => match key {
                    Key::Named(key::Named::Escape) => Some(Message::Dismiss),
                    Key::Named(key::Named::ArrowLeft) => Some(Message::PowerMove(-1)),
                    Key::Named(key::Named::ArrowRight) => Some(Message::PowerMove(1)),
                    Key::Named(key::Named::Enter) => Some(Message::PowerConfirm),
                    Key::Character(c) => c
                        .chars()
                        .next()
                        .and_then(|c| c.to_digit(10))
                        .and_then(|n| (n as usize).checked_sub(1))
                        .filter(|i| *i < crate::panels::POWER_ACTIONS.len())
                        .map(|i| Message::PowerPick(crate::panels::POWER_ACTIONS[i])),
                    _ => None,
                },
                _ => None,
            }),
        ])
    }

    pub fn theme(shell: &Self, _id: Id) -> Theme {
        Theme {
            scheme: shell
                .scheme
                .unwrap_or_else(|| triton_design::Scheme::reference(shell.variant)),
            variant: shell.variant,
            reduce_motion: false,
            // Mica brightens when the wallpaper behind it is light.
            bright_backdrop: shell.wallpaper_luminance > 0.5,
        }
    }

    /// The surface itself is clear: only the islands paint (and niri blurs).
    pub fn style(_shell: &Self, theme: &Theme) -> iced_core::theme::Style {
        iced_core::theme::Style {
            background_color: Color::TRANSPARENT,
            text_color: theme.scheme.on_surface,
        }
    }
}
