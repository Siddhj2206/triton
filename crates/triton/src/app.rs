//! The shell as one iced daemon: niri snapshots in, a bar on every output out.

use std::collections::BTreeMap;
use std::sync::Arc;

use iced_core::Color;
use iced_core::alignment::Vertical;
use iced_core::window::Id;
use iced_futures::Subscription;
use iced_layershell::reexport::Task;
use iced_layershell::to_layer_message;
use iced_widget::{Row, Space};
use triton_design::Icon;
use triton_ui::bar::{self, Status};
use triton_ui::measure::measure;
use triton_ui::surface::layer_island;
use triton_ui::{Element, Theme};

use crate::host::SurfaceTable;
use crate::niri::{self, Snapshot};
use crate::shell::{self, Role, SurfaceKey};

#[to_layer_message(multi)]
#[derive(Debug, Clone)]
pub enum Message {
    Niri(Arc<Snapshot>),
    Minute,
    Closed(Id),
    /// An island's natural width, so its surface can fit it.
    Measured(Id, f32),
    FocusWorkspace(u64),
    Launcher,
    Bell,
}

#[derive(Debug, Default)]
pub struct Shell {
    niri: Arc<Snapshot>,
    surfaces: SurfaceTable,
    widths: BTreeMap<SurfaceKey, u32>,
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
            // The launcher and notification center are not built yet; the
            // layer-shell variants are consumed by iced_layershell itself.
            _ => Task::none(),
        }
    }

    fn reconcile(&mut self) -> Task<Message> {
        let outputs = self.niri.outputs.iter().map(String::as_str);
        let desired = shell::desired(outputs, &self.widths);
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
        };
        measure(layer_island(content), move |w| Message::Measured(id, w)).into()
    }

    fn bar_start<'a>(&'a self, output: &'a str) -> Element<'a, Message> {
        let mut left = Row::new()
            .spacing(bar::GAP)
            .align_y(Vertical::Center)
            .push(bar::launcher().on_press(Message::Launcher));
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
            iced_runtime::window::close_events().map(Message::Closed),
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
