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
use crate::shell::{self, BarMode, Modals, Role, Sizes, SurfaceKey};

/// The launcher field's widget id, so it can be focused once its surface maps.
fn launcher_field() -> widget::Id {
    widget::Id::from("launcher-query")
}

/// The Thick panel's inner padding; the field's radius is `nested` against it.
const LAUNCHER_PAD: f32 = 14.0;

/// `BarStates` timings: 120 ms on the top edge wakes the bar, 600 ms of no
/// pointer over bar chrome puts it away. (The fullscreen 300 ms dwell waits on
/// a fullscreen flag in niri's IPC.)
const REVEAL_DELAY: Duration = Duration::from_millis(120);
const HIDE_DELAY: Duration = Duration::from_millis(600);

/// Placeholder result groups until search lands.
fn launcher_results<'a>() -> Column<'a, Message, Theme, Renderer> {
    let row = |glyph, title, sub| surface::ListItem {
        leading: Some(surface::icon_disc(glyph).into()),
        title,
        supporting: Some(sub),
        trailing: None,
        selected: false,
    };
    Column::new()
        .spacing(6)
        .push(text::eyebrow("Apps"))
        .push(surface::list(
            vec![
                row(Icon::Contacts, "Contacts", "Address book"),
                row(Icon::SettingsRemote, "Connections", "Remote desktops"),
            ],
            false,
        ))
        .push(text::eyebrow("Windows"))
        .push(surface::list(
            vec![row(Icon::Terminal, "Console", "~/niri-shell — Work")],
            false,
        ))
        .push(text::eyebrow("Files"))
        .push(surface::list(
            vec![row(Icon::Description, "config.kdl", "~/.config/niri")],
            false,
        ))
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
    /// A new services snapshot (battery, connectivity) for the status group.
    Statuses(Arc<Statuses>),
}

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
    /// Do Not Disturb; the bell and the center's toggle show it.
    dnd: bool,
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
    /// Battery + connectivity for the status group, from `services`.
    statuses: Statuses,
    /// `--real-services`: watch real UPower/NetworkManager, not the fake.
    real_services: bool,
}

impl Shell {
    pub fn boot(real_services: bool) -> (Self, Task<Message>) {
        let mut shell = Self {
            real_services,
            ..Self::default()
        };
        shell.tick();
        (shell, Task::none())
    }

    fn tick(&mut self) {
        let now = chrono::Local::now();
        self.time = now.format("%H:%M").to_string();
        self.date = now.format("%a %-d %b").to_string();
    }

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
            Message::Statuses(statuses) => {
                self.statuses = (*statuses).clone();
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
                self.launcher = if self.launcher.as_deref() == Some(output.as_str()) {
                    None
                } else {
                    self.query.clear();
                    Some(output)
                };
                self.reconcile()
            }
            Message::Dismiss => {
                if self.launcher.take().is_none() && self.notifications.take().is_none() {
                    return Task::none();
                }
                self.reconcile()
            }
            Message::Opened(id) => match self.surfaces.key(id) {
                // Its UI exists now: put the caret in the field.
                Some(key) if key.role == Role::Launcher => {
                    iced_runtime::task::widget(focusable::focus(launcher_field()))
                }
                _ => Task::none(),
            },
            Message::LauncherInput(query) => {
                self.query = query;
                Task::none()
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
                // A mode switch starts the bar hidden: pinned needs nothing, and
                // autohide/hidden wake on the edge or `bar toggle`.
                self.bar_shown.clear();
                self.bar_hover.clear();
                self.reconcile()
            }
            _ => Task::none(),
        }
    }

    /// A message `d` from now — the bar's reveal dwell and hide delay.
    fn after(d: Duration, message: Message) -> Task<Message> {
        Task::perform(
            async move {
                tokio::time::sleep(d).await;
            },
            move |()| message.clone(),
        )
    }

    /// The output a `triton msg` toggle acts on: the focused workspace's, else
    /// the first niri reports.
    fn focused_output(&self) -> Option<String> {
        self.niri
            .workspaces
            .iter()
            .find(|w| w.is_focused)
            .and_then(|w| w.output.clone())
            .or_else(|| self.niri.outputs.first().cloned())
    }

    /// One `triton msg` line, on the focused output where no output is named.
    fn command(&mut self, line: &str) -> Task<Message> {
        let words: Vec<_> = line.split_whitespace().collect();
        let mut toggle = |message: fn(String) -> Message| match self.focused_output() {
            Some(output) => self.update(message(output)),
            None => Task::none(),
        };
        match words.as_slice() {
            ["launcher", "toggle"] => toggle(Message::Launcher),
            ["notifications", "toggle"] => toggle(Message::Bell),
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
            other => {
                tracing::warn!(?other, "msg: unknown command");
                Task::none()
            }
        }
    }

    fn reconcile(&mut self) -> Task<Message> {
        let outputs = self.niri.outputs.iter().map(String::as_str);
        let desired = shell::desired(
            outputs,
            &self.sizes,
            &Modals {
                launcher: self.launcher.as_deref(),
                notifications: self.notifications.as_deref(),
            },
            self.bar_mode,
            |k| self.bar_shown.contains(k),
        );
        self.surfaces.reconcile(&desired)
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
        .size(24.0)
        .padding(Padding::from([22.0, 20.0]));
        let scopes = Row::new()
            .spacing(8)
            .push(chip("All", None, true))
            .push(chip("Apps", None, false))
            .push(chip("Files", None, false))
            .push(chip("Windows", None, false));
        let top_hit = surface::card(
            Row::new()
                .spacing(14)
                .align_y(Vertical::Center)
                .push(surface::icon_disc(Icon::Terminal))
                .push(
                    Column::new()
                        .push(text::text("Console", TypeStyle::TITLE_M))
                        .push(
                            text::text("Terminal emulator · GNOME", TypeStyle::BODY_S)
                                .class(Tone::OnSurfaceVariant),
                        ),
                )
                .push(Space::new().width(Length::Fill))
                .push(button::button(
                    "Open",
                    None,
                    button::Kind::Filled,
                    button::Size::S,
                ))
                .push(button::button(
                    "New window",
                    None,
                    button::Kind::Text,
                    button::Size::S,
                )),
        )
        .padding(14)
        .width(Length::Fill);
        surface::surface(
            Column::new()
                .spacing(14)
                .push(field)
                .push(scopes)
                .push(text::eyebrow("Top hit"))
                .push(top_hit)
                .push(launcher_results())
                .push(launcher_hints()),
            Surface::LayerPanel(Material::Thick),
        )
        .padding(LAUNCHER_PAD)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
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

    /// The notification center's header and empty state, as plumbing: the
    /// surface and its DND toggle are real, groups arrive with the notification
    /// server in Phase 1.
    fn notifications(&self) -> Element<'_, Message> {
        let header = Row::new()
            .spacing(10)
            .align_y(Vertical::Center)
            .push(text::text("Notifications", TypeStyle::TITLE_M))
            .push(Space::new().width(Length::Fill))
            .push(
                button::icon_button(
                    Icon::DarkMode,
                    button::IconKind::Veil,
                    button::Size::S,
                    self.dnd,
                )
                .on_press(Message::Dnd),
            )
            .push(text::text("DND", TypeStyle::LABEL_L).class(Tone::OnSurfaceVariant));
        let empty = Column::new()
            .spacing(10)
            .align_x(iced_core::alignment::Horizontal::Center)
            .width(Length::Fill)
            .push(surface::icon_disc(Icon::Notifications))
            .push(text::text("Nothing new", TypeStyle::BODY_M).class(Tone::OnSurfaceVariant));
        // The surface is measured for height: no Fill anywhere inside, or it
        // would stretch forever.
        surface::surface(
            Column::new()
                .push(header)
                .push(Space::new().height(48))
                .push(empty)
                .push(Space::new().height(56)),
            Surface::LayerPanel(Material::Glass),
        )
        .padding(Padding::from([18.0, 16.0]))
        .width(Length::Fill)
        .into()
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
            services::statuses(shell.real_services).map(Message::Statuses),
            iced_runtime::window::close_events().map(Message::Closed),
            event::listen_with(|event, _status, id| match event {
                Event::Window(window::Event::Opened { .. }) => Some(Message::Opened(id)),
                Event::Keyboard(keyboard::Event::KeyPressed {
                    key: Key::Named(key::Named::Escape),
                    ..
                }) => Some(Message::Dismiss),
                _ => None,
            }),
        ])
    }

    pub fn theme(_shell: &Self, _id: Id) -> Theme {
        Theme::dark()
    }

    /// The surface itself is clear: only the islands paint (and niri blurs).
    pub fn style(_shell: &Self, theme: &Theme) -> iced_core::theme::Style {
        iced_core::theme::Style {
            background_color: Color::TRANSPARENT,
            text_color: theme.scheme.on_surface,
        }
    }
}
