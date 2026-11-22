//! The Phase 1 panels, as pure views: state in, elements out. `app` owns the
//! state and maps service/store types onto these plain inputs.

use std::time::Duration;

use iced_core::alignment::{Horizontal, Vertical};
use iced_core::{Length, Padding};
use iced_widget::{Column, Row, Space, mouse_area};
use triton_design::material::Material;
use triton_design::{Icon, IconStyle, TypeStyle};
use triton_ui::button::{self, Kind, Size};
use triton_ui::field::{self, Field};
use triton_ui::surface::{self, Surface};
use triton_ui::{Element, Tone, text};

use crate::app::Message;

// ── OSD ──────────────────────────────────────────────────────────────────────

/// What the OSD shows (`OSD` board). Numbers always accompany a bar — "a bar
/// alone is not precise enough".
#[derive(Debug, Clone)]
pub enum Osd {
    /// Volume level 0..=150; icon steps off/low/mid/high. Muted draws the wave.
    Volume { percent: u8, muted: bool },
    /// Screen brightness percent.
    Brightness { percent: u8 },
    /// Microphone off — error container, "stays until you move on".
    MicMuted,
    /// Keyboard layout code + language name ("DE / Deutsch").
    Layout { code: String, name: String },
    /// Low battery: percent + optional hint line (6 s hold).
    Battery { percent: u8, left: String },
    /// A workspace switch (from a `Mod+N` bind): its name or index.
    Workspace { label: String },
}

impl Osd {
    /// Hold time per the board: 1.2 s, battery low 6 s.
    pub fn hold(&self) -> Duration {
        match self {
            Self::Battery { .. } => Duration::from_millis(6000),
            _ => Duration::from_millis(1200),
        }
    }
}

fn osd_level_icon(percent: u8, muted: bool) -> Icon {
    if muted || percent == 0 {
        Icon::VolumeOff
    } else if percent < 34 {
        Icon::VolumeMute
    } else if percent < 67 {
        Icon::VolumeDown
    } else {
        Icon::VolumeUp
    }
}

/// A volume-style row: icon disc (or wave when "off"), level bar, number.
fn osd_meter(icon: Icon, level: f32, number: String, off: bool) -> Element<'static, Message> {
    let mark: Element<'static, Message> = if off {
        // The board: off states turn the circle into a wave in a warm color.
        triton_ui::progress::wave_shape(40.0, triton_ui::progress::WaveTone::Tertiary)
    } else {
        surface::icon_disc(icon).into()
    };
    Row::new()
        .spacing(14)
        .align_y(Vertical::Center)
        .push(mark)
        .push(
            iced_widget::container(triton_ui::progress::flat_progress(level))
                .width(140)
                .center_y(Length::Fill),
        )
        .push(text::text(number, TypeStyle::TITLE_L))
        .padding(Padding::from([12.0, 8.0]))
        .into()
}

/// The OSD pill content. Pass-through: nothing here is pressable.
pub fn osd(osd: &Osd) -> Element<'_, Message> {
    let row = match osd {
        Osd::Volume { percent, muted } => osd_meter(
            osd_level_icon(*percent, *muted),
            f32::from((*percent).min(100)) / 100.0,
            percent.to_string(),
            *muted,
        ),
        Osd::Brightness { percent } => osd_meter(
            Icon::BrightnessHigh,
            f32::from(*percent) / 100.0,
            percent.to_string(),
            false,
        ),
        Osd::MicMuted => osd_meter(Icon::MicOff, 0.0, "Microphone off".to_owned(), true),
        Osd::Layout { code, name } => Row::new()
            .spacing(14)
            .align_y(Vertical::Center)
            .push(surface::icon_disc(Icon::Keyboard))
            .push(
                Column::new()
                    .push(text::text(code.clone(), TypeStyle::TITLE_M))
                    .push(
                        text::text(name.clone(), TypeStyle::BODY_S).class(Tone::OnSurfaceVariant),
                    ),
            )
            .padding(Padding::from([14.0, 8.0]))
            .into(),
        Osd::Battery { percent, left } => {
            let mut lines = Column::new().push(text::text(
                format!("Battery {percent}%"),
                TypeStyle::TITLE_M,
            ));
            if !left.is_empty() {
                lines = lines.push(
                    text::text(left.clone(), TypeStyle::BODY_S).class(Tone::OnSurfaceVariant),
                );
            }
            Row::new()
                .spacing(14)
                .align_y(Vertical::Center)
                .push(surface::icon_disc(Icon::BatteryAlert))
                .push(lines)
                .padding(Padding::from([14.0, 8.0]))
                .into()
        }
        Osd::Workspace { label } => Row::new()
            .spacing(14)
            .align_y(Vertical::Center)
            .push(surface::icon_disc(Icon::Apps))
            .push(text::text(label.clone(), TypeStyle::TITLE_M))
            .padding(Padding::from([14.0, 8.0]))
            .into(),
    };
    surface::surface(row, Surface::LayerPanel(Material::Glass))
        .padding(Padding::from([6.0, 8.0]))
        .into()
}

// ── Power menu ────────────────────────────────────────────────────────────────

/// One action in the `Power` board's row, in board order (Lock focused first).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerAction {
    Lock,
    Sleep,
    LogOut,
    Restart,
    ShutDown,
}

pub const POWER_ACTIONS: [PowerAction; 5] = [
    PowerAction::Lock,
    PowerAction::Sleep,
    PowerAction::LogOut,
    PowerAction::Restart,
    PowerAction::ShutDown,
];

impl PowerAction {
    fn label(self) -> &'static str {
        match self {
            Self::Lock => "Lock",
            Self::Sleep => "Sleep",
            Self::LogOut => "Log out",
            Self::Restart => "Restart",
            Self::ShutDown => "Shut down",
        }
    }

    fn icon(self) -> Icon {
        match self {
            Self::Lock => Icon::Lock,
            Self::Sleep => Icon::Bedtime,
            Self::LogOut => Icon::Logout,
            Self::Restart => Icon::RestartAlt,
            Self::ShutDown => Icon::PowerSettingsNew,
        }
    }
}

/// The `Power` board: greeting, uptime, the five-action row (focused action
/// grows and gains its label), an optional warning note, key hints, footer.
pub fn power_menu<'a>(
    greeting: &'a str,
    uptime: &'a str,
    focused: PowerAction,
    note: Option<&'a str>,
) -> Element<'a, Message> {
    // One tile per action. The focused action is the one Enter confirms; the
    // board draws it wider with its "Press Enter" hint underneath.
    let mut row = Row::new().spacing(10).align_y(Vertical::Center);
    for action in POWER_ACTIONS {
        let on = action == focused;
        let tile = button::button(
            action.label(),
            Some(action.icon()),
            match action {
                PowerAction::Lock => Kind::Filled,
                PowerAction::Restart | PowerAction::ShutDown => Kind::Tonal,
                _ => Kind::Veil,
            },
            Size::Xl,
        )
        .on_press(Message::PowerPick(action));
        if on {
            row =
                row.push(Column::new().align_x(Horizontal::Center).push(tile).push(
                    text::text("Press Enter", TypeStyle::LABEL_S).class(Tone::OnSurfaceVariant),
                ));
        } else {
            row = row.push(tile);
        }
    }
    let mut col = Column::new()
        .spacing(18)
        .align_x(Horizontal::Center)
        .push(text::text(greeting.to_owned(), TypeStyle::HEADLINE_L))
        .push(text::text(uptime.to_owned(), TypeStyle::BODY_M).class(Tone::OnSurfaceVariant))
        .push(row);
    if let Some(note) = note {
        col = col.push(surface::pill(Icon::Warning, note));
    }
    col =
        col.push(
            Row::new()
                .spacing(18)
                .push(
                    Row::new()
                        .spacing(6)
                        .push(surface::kbd("←"))
                        .push(surface::kbd("→"))
                        .push(text::text("move", TypeStyle::LABEL_S).class(Tone::OnSurfaceVariant)),
                )
                .push(
                    Row::new().spacing(6).push(surface::kbd("Enter")).push(
                        text::text("confirm", TypeStyle::LABEL_S).class(Tone::OnSurfaceVariant),
                    ),
                )
                .push(
                    Row::new()
                        .spacing(6)
                        .push(surface::kbd("1 – 5"))
                        .push(text::text("jump", TypeStyle::LABEL_S).class(Tone::OnSurfaceVariant)),
                )
                .push(
                    Row::new().spacing(6).push(surface::kbd("Esc")).push(
                        text::text("cancel", TypeStyle::LABEL_S).class(Tone::OnSurfaceVariant),
                    ),
                ),
        );
    surface::surface(col, Surface::LayerPanel(Material::Glass))
        .padding(Padding::from([28.0, 32.0]))
        .into()
}

// ── Quick settings ────────────────────────────────────────────────────────────

/// The panel's pages; chevrons drill in place (`QuickSettings` board).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum QsPage {
    #[default]
    Root,
    Wifi,
    Bluetooth,
    Sound,
}

/// One Wi-Fi page row, shaped by `app` from `services::AccessPoint`.
#[derive(Debug)]
pub struct QsAp {
    pub ssid: String,
    /// "82% · secured" under the SSID.
    pub supporting: String,
    pub secured: bool,
    pub connected: bool,
}

/// One Bluetooth page row, shaped by `app` from `services::Device`.
#[derive(Debug)]
pub struct QsBt {
    pub name: String,
    /// "Connected · 82%" / "Paired" / "Available".
    pub supporting: String,
    pub address: String,
    pub connected: bool,
}

/// The board's 2×2 tile grid, then the two sliders, then the footer.
/// Inputs arrive already shaped by `app` from `services::State`.
/// Six tiles, six bools — the shape mirrors the board.
#[expect(clippy::struct_excessive_bools)]
pub struct Qs<'a> {
    pub page: QsPage,
    pub wifi_on: bool,
    pub wifi_label: &'a str,
    pub bt_on: bool,
    pub bt_label: &'a str,
    pub sound_label: &'a str,
    pub night_on: bool,
    pub night_label: &'a str,
    pub brightness: f32,
    pub volume: f32,
    pub dnd: bool,
    pub dark: bool,
    pub saver: bool,
    /// The Sound page's mic control: hidden when no source exists.
    pub mic_present: bool,
    pub mic_muted: bool,
    /// The Wi-Fi page's access points.
    pub aps: &'a [QsAp],
    /// The Bluetooth page's devices.
    pub bt_devices: &'a [QsBt],
}

fn tile<'a>(
    icon: Icon,
    title: &'a str,
    sub: &'a str,
    on: bool,
    press: Message,
    page: Option<QsPage>,
) -> Element<'a, Message> {
    let mut row = Row::new()
        .spacing(12)
        .align_y(Vertical::Center)
        .push(text::icon(icon, IconStyle::Outlined, 22.0))
        .push(
            Column::new()
                .push(text::text(title, TypeStyle::LABEL_L))
                .push(text::text(sub, TypeStyle::BODY_S).class(Tone::OnSurfaceVariant)),
        )
        .push(Space::new().width(Length::Fill));
    if page.is_some() {
        row = row.push(text::icon(Icon::ChevronRight, IconStyle::Outlined, 18.0));
    }
    let t = mouse_area(
        surface::surface(
            row,
            if on {
                Surface::Card
            } else {
                Surface::Wrap {
                    material: Material::Glass,
                    inner: 16.0,
                    inset: 0.0,
                }
            },
        )
        .padding(Padding::from([14.0, 16.0]))
        .width(Length::Fill),
    )
    .on_press(press);
    t.into()
}

/// `qs` is owned so its `&'a` fields keep the `Element<'a>` valid.
#[expect(
    clippy::needless_pass_by_value,
    reason = "fields borrow self-lived data; moving is required"
)]
#[expect(
    clippy::too_many_lines,
    reason = "four small pages share one layout pass"
)]
pub fn quick_settings(qs: Qs<'_>) -> Element<'_, Message> {
    let content: Element<'_, Message> = match qs.page {
        QsPage::Root => {
            let grid = Column::new()
                .spacing(10)
                .push(
                    Row::new()
                        .spacing(10)
                        .push(tile(
                            Icon::Wifi,
                            "Wi-Fi",
                            qs.wifi_label,
                            qs.wifi_on,
                            Message::QsToggleWifi,
                            Some(QsPage::Wifi),
                        ))
                        .push(tile(
                            Icon::Bluetooth,
                            "Bluetooth",
                            qs.bt_label,
                            qs.bt_on,
                            Message::QsToggleBt,
                            Some(QsPage::Bluetooth),
                        )),
                )
                .push(
                    Row::new()
                        .spacing(10)
                        .push(tile(
                            Icon::VolumeUp,
                            "Sound",
                            qs.sound_label,
                            true,
                            Message::QsPage(QsPage::Sound),
                            Some(QsPage::Sound),
                        ))
                        .push(tile(
                            Icon::Bedtime,
                            "Night light",
                            qs.night_label,
                            qs.night_on,
                            Message::QsToggleNight,
                            None,
                        )),
                );
            let sliders = Column::new()
                .spacing(4)
                .push(
                    Row::new()
                        .spacing(12)
                        .align_y(Vertical::Center)
                        .push(text::icon(Icon::BrightnessHigh, IconStyle::Outlined, 20.0))
                        .push(
                            triton_ui::slider::slider(qs.brightness, Message::QsBrightness)
                                .on_glass(true)
                                .on_release(Message::QsBrightnessCommit),
                        ),
                )
                .push(
                    Row::new()
                        .spacing(12)
                        .align_y(Vertical::Center)
                        .push(text::icon(Icon::VolumeUp, IconStyle::Outlined, 20.0))
                        .push(
                            triton_ui::slider::slider(qs.volume, Message::QsVolume)
                                .on_glass(true)
                                .on_release(Message::QsVolumeCommit),
                        ),
                );
            let toggles = Row::new()
                .spacing(8)
                .push(
                    button::icon_button(Icon::DarkMode, button::IconKind::Veil, Size::M, qs.dnd)
                        .on_press(Message::Dnd),
                )
                .push(
                    button::icon_button(Icon::Contrast, button::IconKind::Veil, Size::M, qs.dark)
                        .on_press(Message::QsToggleDark),
                )
                .push(
                    button::icon_button(
                        Icon::BatterySaver,
                        button::IconKind::Veil,
                        Size::M,
                        qs.saver,
                    )
                    .on_press(Message::QsToggleSaver),
                );
            Column::new()
                .spacing(14)
                .push(grid)
                .push(sliders)
                .push(toggles)
                .push(
                    Row::new()
                        .spacing(10)
                        .push(
                            button::button("All settings", None, Kind::Filled, Size::M)
                                .on_press(Message::QsAllSettings),
                        )
                        .push(Space::new().width(Length::Fill))
                        .push(
                            button::icon_button(
                                Icon::PowerSettingsNew,
                                button::IconKind::Veil,
                                Size::M,
                                false,
                            )
                            .on_press(Message::PowerToggle),
                        ),
                )
                .into()
        }
        QsPage::Wifi => {
            let items: Vec<surface::ListItem<'_, Message>> = qs
                .aps
                .iter()
                .map(|ap| surface::ListItem {
                    leading: Some(surface::icon_disc(Icon::Wifi).into()),
                    title: ap.ssid.as_str(),
                    supporting: Some(ap.supporting.as_str()),
                    trailing: Some(
                        if ap.connected {
                            text::icon(Icon::Check, IconStyle::Outlined, 20.0)
                        } else if ap.secured {
                            text::icon(Icon::Lock, IconStyle::Outlined, 18.0)
                        } else {
                            text::icon(Icon::Wifi, IconStyle::Outlined, 18.0)
                        }
                        .into(),
                    ),
                    selected: ap.connected,
                    // The connected row is inert; others join the network.
                    on_press: (!ap.connected).then(|| Message::WifiJoin(ap.ssid.clone())),
                })
                .collect();
            Column::new()
                .spacing(14)
                .push(surface::list(items, false))
                .push(
                    button::button(
                        "All network settings",
                        Some(Icon::Lan),
                        Kind::Filled,
                        Size::M,
                    )
                    .on_press(Message::QsAllSettings),
                )
                .into()
        }
        QsPage::Bluetooth => {
            let items: Vec<surface::ListItem<'_, Message>> = qs
                .bt_devices
                .iter()
                .map(|d| surface::ListItem {
                    leading: Some(surface::icon_disc(Icon::Bluetooth).into()),
                    title: d.name.as_str(),
                    supporting: Some(d.supporting.as_str()),
                    trailing: d.connected.then(|| {
                        text::icon(Icon::BluetoothConnected, IconStyle::Outlined, 20.0).into()
                    }),
                    selected: d.connected,
                    on_press: Some(Message::BtDevice {
                        address: d.address.clone(),
                        connected: d.connected,
                    }),
                })
                .collect();
            Column::new()
                .spacing(14)
                .push(surface::list(items, false))
                .push(
                    button::button(
                        "All bluetooth settings",
                        Some(Icon::Bluetooth),
                        Kind::Filled,
                        Size::M,
                    )
                    .on_press(Message::QsAllSettings),
                )
                .into()
        }
        QsPage::Sound => {
            let mut col = Column::new().spacing(14).push(
                Row::new()
                    .spacing(12)
                    .align_y(Vertical::Center)
                    .push(text::icon(Icon::VolumeUp, IconStyle::Outlined, 20.0))
                    .push(
                        triton_ui::slider::slider(qs.volume, Message::QsVolume)
                            .on_glass(true)
                            .on_release(Message::QsVolumeCommit),
                    ),
            );
            if qs.mic_present {
                col = col.push(
                    Row::new()
                        .spacing(12)
                        .align_y(Vertical::Center)
                        .push(text::icon(Icon::Mic, IconStyle::Outlined, 20.0))
                        .push(text::text("Microphone", TypeStyle::BODY_M))
                        .push(Space::new().width(Length::Fill))
                        .push(
                            button::button(
                                if qs.mic_muted { "Unmute" } else { "Mute" },
                                None,
                                Kind::Tonal,
                                Size::S,
                            )
                            .on_press(Message::QsMicMute),
                        ),
                );
            }
            col.push(
                button::button(
                    "All sound settings",
                    Some(Icon::Tune),
                    Kind::Filled,
                    Size::M,
                )
                .on_press(Message::QsAllSettings),
            )
            .into()
        }
    };
    // Header: back + title on pages, title + gear on root.
    let header = match qs.page {
        QsPage::Root => Row::new()
            .align_y(Vertical::Center)
            .push(text::text("Quick settings", TypeStyle::TITLE_M))
            .push(Space::new().width(Length::Fill))
            .push(
                button::icon_button(Icon::Settings, button::IconKind::Veil, Size::S, false)
                    .on_press(Message::QsAllSettings),
            ),
        page => {
            let title = match page {
                QsPage::Wifi => "Wi-Fi",
                QsPage::Bluetooth => "Bluetooth",
                QsPage::Sound => "Sound",
                QsPage::Root => unreachable!(),
            };
            Row::new()
                .spacing(8)
                .align_y(Vertical::Center)
                .push(
                    button::icon_button(Icon::ArrowBack, button::IconKind::Veil, Size::S, false)
                        .on_press(Message::QsPage(QsPage::Root)),
                )
                .push(text::text(title, TypeStyle::TITLE_M))
        }
    };
    surface::surface(
        Column::new().spacing(16).push(header).push(content),
        Surface::LayerPanel(Material::Glass),
    )
    .padding(Padding::from([18.0, 16.0]))
    .width(Length::Fill)
    .into()
}

// ── Toasts + notification center ──────────────────────────────────────────────

/// One toast/card input.
#[derive(Debug, Clone)]
pub struct NotifView {
    pub id: u32,
    pub app: String,
    pub title: String,
    pub body: String,
    pub icon: Option<Icon>,
    pub critical: bool,
    /// Relative time label ("now", "12 min").
    pub when: String,
}

fn notif_card(n: &NotifView, toast: bool) -> Element<'static, Message> {
    let glyph: Element<'static, Message> = n.icon.map_or_else(
        || surface::icon_disc(Icon::Notifications).into(),
        |i| surface::icon_disc(i).into(),
    );
    let row = Row::new()
        .spacing(12)
        .align_y(Vertical::Center)
        .push(glyph)
        .push(
            Column::new()
                .width(Length::Fill)
                .push(
                    Row::new()
                        .push(text::text(n.app.clone(), TypeStyle::LABEL_L))
                        .push(Space::new().width(Length::Fill))
                        .push(
                            text::text(n.when.clone(), TypeStyle::LABEL_S)
                                .class(Tone::OnSurfaceVariant),
                        ),
                )
                .push(text::text(n.title.clone(), TypeStyle::BODY_M))
                .push(text::text(n.body.clone(), TypeStyle::BODY_S).class(Tone::OnSurfaceVariant)),
        );
    let card = mouse_area(
        surface::surface(
            row,
            if n.critical {
                Surface::Status(surface::StatusTone::Alert)
            } else if toast {
                Surface::Panel(Material::Glass)
            } else {
                Surface::Row
            },
        )
        .padding(Padding::from([10.0, 12.0]))
        .width(Length::Fill),
    )
    .on_press(Message::NotifActivate(n.id));
    card.into()
}

/// The toast stack: up to four cards plus the overflow chip ("2 more").
/// `hover` pauses timers — handled by `mouse_area` in app.
pub fn toasts(list: Vec<NotifView>, overflow: usize) -> Element<'static, Message> {
    let mut col = Column::new().spacing(10);
    for n in list.into_iter().take(4) {
        col = col.push(notif_card(&n, true));
    }
    if overflow > 0 {
        col = col.push(
            surface::pill(Icon::Stacks, format!("{overflow} more"))
                .align_x(Horizontal::Right)
                .width(Length::Fill),
        );
    }
    col.into()
}

/// The notification center: header (title, DND), then app groups — each a
/// header row (app, count, expand, Clear) and its cards (or "Show N more").
pub fn notification_center(
    groups: &[(String, Vec<NotifView>)],
    expanded: impl Fn(&str) -> bool,
    dnd: bool,
) -> Element<'static, Message> {
    let header = Row::new()
        .spacing(10)
        .align_y(Vertical::Center)
        .push(text::text("Notifications", TypeStyle::TITLE_M))
        .push(Space::new().width(Length::Fill))
        .push(
            button::icon_button(Icon::DarkMode, button::IconKind::Veil, Size::S, dnd)
                .on_press(Message::Dnd),
        )
        .push(text::text("DND", TypeStyle::LABEL_L).class(Tone::OnSurfaceVariant));
    let mut col = Column::new().spacing(14).push(header);
    if groups.is_empty() {
        col = col.push(
            Column::new()
                .spacing(10)
                .align_x(Horizontal::Center)
                .width(Length::Fill)
                .push(surface::icon_disc(Icon::Notifications))
                .push(text::text("Nothing new", TypeStyle::BODY_M).class(Tone::OnSurfaceVariant)),
        );
    }
    for (app, items) in groups {
        let is_open = expanded(app.as_str());
        let group_header = Row::new()
            .spacing(8)
            .align_y(Vertical::Center)
            .push(surface::icon_disc(Icon::Notifications))
            .push(text::text(app.clone(), TypeStyle::TITLE_M))
            .push(
                text::text(format!("{} new", items.len()), TypeStyle::LABEL_S)
                    .class(Tone::OnSurfaceVariant),
            )
            .push(Space::new().width(Length::Fill))
            .push(
                button::icon_button(
                    if is_open {
                        Icon::ExpandLess
                    } else {
                        Icon::ExpandMore
                    },
                    button::IconKind::Veil,
                    Size::S,
                    false,
                )
                .on_press(Message::NotifGroupToggle(app.clone())),
            )
            .push(
                button::button("Clear", None, Kind::Text, Size::S)
                    .on_press(Message::NotifClearApp(app.clone())),
            );
        let mut group = Column::new().spacing(6).push(group_header);
        let (shown, hidden) = if is_open || items.len() <= 2 {
            (items.len(), 0)
        } else {
            (2, items.len() - 2)
        };
        for n in items.iter().take(shown) {
            group = group.push(notif_card(n, false));
        }
        if hidden > 0 {
            group = group.push(
                button::button(format!("Show {hidden} more"), None, Kind::Text, Size::S)
                    .on_press(Message::NotifGroupToggle(app.clone())),
            );
        }
        col = col.push(group);
    }
    surface::surface(col, Surface::LayerPanel(Material::Glass))
        .padding(Padding::from([18.0, 16.0]))
        .width(Length::Fill)
        .into()
}

// ── Polkit dialog ─────────────────────────────────────────────────────────────

/// The solid authentication dialog: message, the shared field, confirm/cancel.
pub fn polkit_dialog<'a>(message: &'a str, user: &'a str, value: &'a str) -> Element<'a, Message> {
    let field = field::search(
        "Password",
        value,
        Field::Nested {
            outer: triton_design::shape::RADII.floating_window,
            inset: crate::app::LAUNCHER_PAD,
        },
    )
    .secure(true)
    .id(crate::app::polkit_field())
    .on_input(Message::PolkitInput)
    .on_submit(Message::PolkitSubmit);
    surface::surface(
        Column::new()
            .spacing(14)
            .push(
                Row::new()
                    .spacing(12)
                    .align_y(Vertical::Center)
                    .push(text::icon(Icon::Lock, IconStyle::Outlined, 28.0))
                    .push(
                        Column::new()
                            .push(text::text(message, TypeStyle::BODY_L))
                            .push(
                                text::text(user, TypeStyle::BODY_S).class(Tone::OnSurfaceVariant),
                            ),
                    ),
            )
            .push(field)
            .push(
                Row::new()
                    .spacing(10)
                    .push(Space::new().width(Length::Fill))
                    .push(
                        button::button("Cancel", None, Kind::Text, Size::M)
                            .on_press(Message::PolkitCancel),
                    )
                    .push(
                        button::button("Authenticate", None, Kind::Filled, Size::M)
                            .on_press(Message::PolkitSubmit),
                    ),
            ),
        Surface::LayerPanel(Material::Solid),
    )
    .padding(Padding::from([20.0, 22.0]))
    .width(420)
    .into()
}
