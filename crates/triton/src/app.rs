//! The shell as one iced daemon: niri snapshots in, a bar on every output out.

use std::collections::BTreeMap;
use std::sync::Arc;

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
use triton_ui::measure::measure;
use triton_ui::surface::{self, layer_island};
use triton_ui::text;
use triton_ui::{Element, Renderer, Surface, Theme, Tone};

use crate::host::SurfaceTable;
use crate::msg;
use crate::niri::{self, Snapshot};
use crate::shell::{self, Role, SurfaceKey};

/// The launcher field's widget id, so it can be focused once its surface maps.
fn launcher_field() -> widget::Id {
    widget::Id::from("launcher-query")
}

/// The Thick panel's inner padding; the field's radius is `nested` against it.
const LAUNCHER_PAD: f32 = 14.0;

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
    /// An island's natural width, so its surface can fit it.
    Measured(Id, f32),
    FocusWorkspace(u64),
    /// The wave was pressed on this output: toggle the launcher there.
    Launcher(String),
    /// Esc or a scrim press: close the modal surface.
    Dismiss,
    LauncherInput(String),
    /// A `triton msg …` line.
    Cmd(String),
    Bell,
}

#[derive(Debug, Default)]
pub struct Shell {
    niri: Arc<Snapshot>,
    surfaces: SurfaceTable,
    widths: BTreeMap<SurfaceKey, u32>,
    /// The output the launcher is open on, if any.
    launcher: Option<String>,
    /// The launcher's query text.
    query: String,
    time: String,
    date: String,
    battery: Option<Battery>,
}

#[derive(Debug, Clone, Copy)]
struct Battery {
    percent: u8,
    charging: bool,
}

impl Battery {
    /// The first system battery in sysfs, if this machine has one.
    fn read() -> Option<Self> {
        let dir = std::fs::read_dir("/sys/class/power_supply").ok()?;
        dir.flatten()
            .map(|e| e.path())
            .find(|p| {
                std::fs::read_to_string(p.join("type")).is_ok_and(|t| t.trim() == "Battery")
                    && std::fs::read_to_string(p.join("scope"))
                        .map_or(true, |s| s.trim() != "Device")
            })
            .and_then(|p| {
                let percent = std::fs::read_to_string(p.join("capacity"))
                    .ok()?
                    .trim()
                    .parse::<u8>()
                    .ok()?;
                let charging = std::fs::read_to_string(p.join("status"))
                    .is_ok_and(|s| matches!(s.trim(), "Charging" | "Full"));
                Some(Self { percent, charging })
            })
    }

    fn icon(self) -> Icon {
        if self.charging {
            return Icon::BatteryChargingFull;
        }
        match self.percent {
            0..=5 => Icon::BatteryAlert,
            6..=15 => Icon::Battery1Bar,
            16..=30 => Icon::Battery2Bar,
            31..=45 => Icon::Battery3Bar,
            46..=60 => Icon::Battery4Bar,
            61..=75 => Icon::Battery5Bar,
            76..=90 => Icon::Battery6Bar,
            _ => Icon::BatteryFull,
        }
    }
}

impl Shell {
    pub fn boot() -> (Self, Task<Message>) {
        let mut shell = Self::default();
        shell.tick();
        (shell, Task::none())
    }

    fn tick(&mut self) {
        let now = chrono::Local::now();
        self.time = now.format("%H:%M").to_string();
        self.date = now.format("%a %-d %b").to_string();
        self.battery = Battery::read();
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Niri(snapshot) => {
                self.niri = snapshot;
                self.reconcile()
            }
            Message::Measured(id, width) => {
                let Some(key) = self.surfaces.key(id).cloned() else {
                    return Task::none();
                };
                self.widths.insert(key, width.ceil() as u32);
                self.reconcile()
            }
            Message::Minute => {
                self.tick();
                Task::none()
            }
            Message::Closed(id) => {
                self.surfaces.forget(id);
                Task::none()
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
                if self.launcher.take().is_none() {
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
            // The bell's notification center is not built yet; the layer-shell
            // variants are consumed by iced_layershell itself.
            _ => Task::none(),
        }
    }

    /// One `triton msg` line, on the focused output where no output is named.
    fn command(&mut self, line: &str) -> Task<Message> {
        let words: Vec<_> = line.split_whitespace().collect();
        match words.as_slice() {
            ["launcher", "toggle"] => {
                let output = self
                    .niri
                    .workspaces
                    .iter()
                    .find(|w| w.is_focused)
                    .and_then(|w| w.output.clone())
                    .or_else(|| self.niri.outputs.first().cloned());
                match output {
                    Some(output) => self.update(Message::Launcher(output)),
                    None => Task::none(),
                }
            }
            other => {
                tracing::warn!(?other, "msg: unknown command");
                Task::none()
            }
        }
    }

    fn reconcile(&mut self) -> Task<Message> {
        let outputs = self.niri.outputs.iter().map(String::as_str);
        let desired = shell::desired(outputs, &self.widths, self.launcher.as_deref());
        self.surfaces.reconcile(&desired)
    }

    pub fn view(&self, id: Id) -> Element<'_, Message> {
        let Some(key) = self.surfaces.key(id) else {
            return Space::new().into();
        };
        let content = match key.role {
            Role::IslandStart => self.bar_start(&key.output),
            Role::IslandEnd => self.bar_end(),
            Role::Zone => return Space::new().into(),
            Role::Scrim => return Self::scrim(),
            Role::Launcher => return self.launcher(),
        };
        measure(layer_island(content), move |w| Message::Measured(id, w)).into()
    }

    /// The dim a press lands on to dismiss the launcher (`Scrim` covers the
    /// output; the panel above it keeps its own presses).
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

    fn bar_end(&self) -> Element<'_, Message> {
        let mut right = Row::new()
            .spacing(bar::GAP)
            .align_y(Vertical::Center)
            .push(bar::clock(self.time.clone(), self.date.clone()));
        let status: Vec<Status> = self
            .battery
            .map(|b| Status {
                icon: b.icon(),
                label: Some(format!("{}%", b.percent)),
            })
            .into_iter()
            .collect();
        if let Some(pill) = bar::status(status) {
            right = right.push(pill);
        }
        right
            .push(bar::bell(false, false).on_press(Message::Bell))
            .into()
    }

    pub fn subscription(_shell: &Self) -> Subscription<Message> {
        Subscription::batch([
            niri::snapshots().map(Message::Niri),
            niri::minutes().map(|()| Message::Minute),
            msg::commands().map(Message::Cmd),
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
