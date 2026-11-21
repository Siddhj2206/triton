//! System services over D-Bus: UPower's display device and NetworkManager's
//! connectivity feed the bar's status group. Each service gets a thread that
//! `block_on`s one connection, re-reading its object when `PropertiesChanged`
//! arrives; a merge thread hands the app an immutable [`Statuses`] per change.
//! A missing service just never reports — a desktop has no battery icon.
//!
//! Unless `--real-services` is passed the fake backend runs instead: a dev
//! session must never touch real power or network state, and the preview
//! container has no system bus at all. The fake cycles the bar's states so a
//! screenshot or a dev run shows every tone.

use std::sync::Arc;
use std::thread;
use std::time::Duration;

use iced_futures::Subscription;
use iced_futures::futures::channel::mpsc;
use iced_futures::futures::{Stream, StreamExt};
use triton_design::Icon;
use zbus::proxy;
use zbus::{Connection, fdo};

/// What the status group knows about the machine at one moment.
#[derive(Debug, Clone, Default)]
pub struct Statuses {
    /// `None` until UPower reports a battery.
    pub battery: Option<Battery>,
    /// `None` until NetworkManager answers — unknown is not "offline".
    pub connectivity: Option<Connectivity>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Battery {
    pub percent: u8,
    pub charging: bool,
    /// Low enough to ask for attention: UPower's warning level, or 10 % left
    /// while discharging for devices that never raise one.
    pub low: bool,
}

impl Battery {
    pub fn icon(self) -> Icon {
        if self.low {
            return Icon::BatteryAlert;
        }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Connectivity {
    Online,
    /// None, portal or limited: anything short of full connectivity.
    Offline,
}

/// A change one service reported, merged into [`Statuses`] below.
enum Update {
    Battery(Option<Battery>),
    Connectivity(Option<Connectivity>),
}

#[proxy(
    interface = "org.freedesktop.UPower.Device",
    default_service = "org.freedesktop.UPower",
    default_path = "/org/freedesktop/UPower/devices/DisplayDevice"
)]
trait UpowerDevice {
    #[zbus(property)]
    fn percentage(&self) -> zbus::Result<f64>;
    #[zbus(property)]
    fn state(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn warning_level(&self) -> zbus::Result<u32>;
    /// UPower's `Type`: 1 is line power — the display device sits on it when
    /// the machine has no battery at all.
    #[zbus(property, name = "Type")]
    fn kind(&self) -> zbus::Result<u32>;
}

#[proxy(
    interface = "org.freedesktop.NetworkManager",
    default_service = "org.freedesktop.NetworkManager",
    default_path = "/org/freedesktop/NetworkManager"
)]
trait NetworkManager {
    /// `Connectivity`: 0 unknown, 1 none, 2 portal, 3 limited, 4 full.
    #[zbus(property)]
    fn connectivity(&self) -> zbus::Result<u32>;
}

async fn battery(dev: &UpowerDeviceProxy<'_>) -> Option<Battery> {
    // The display device reports kind 2 (battery) only while one exists.
    if dev.kind().await.ok() != Some(2) {
        return None;
    }
    let percent = dev.percentage().await.ok()?.round() as u8;
    // UpDeviceState: 1 charging, 4 fully charged, the rest discharging-ish.
    let charging = matches!(dev.state().await.unwrap_or(0), 1 | 4);
    // UpDeviceLevel: 3 low, 4 critical, 5 action.
    let warn = dev.warning_level().await.unwrap_or(0) >= 3;
    Some(Battery {
        percent,
        charging,
        low: warn || (!charging && percent <= 10),
    })
}

fn connectivity(level: u32) -> Connectivity {
    if level == 4 {
        Connectivity::Online
    } else {
        Connectivity::Offline
    }
}

fn watch_upower(updates: &std::sync::mpsc::SyncSender<Update>) {
    zbus::block_on(async {
        let conn = match Connection::system().await {
            Ok(conn) => conn,
            Err(e) => {
                tracing::warn!(%e, "upower: no system bus");
                return;
            }
        };
        let dev = match UpowerDeviceProxy::new(&conn).await {
            Ok(dev) => dev,
            Err(e) => {
                tracing::warn!(%e, "upower: not running");
                return;
            }
        };
        let props = match fdo::PropertiesProxy::builder(&conn)
            .destination("org.freedesktop.UPower")
            .path("/org/freedesktop/UPower/devices/DisplayDevice")
            .build()
            .await
        {
            Ok(props) => props,
            Err(e) => {
                tracing::warn!(%e, "upower: cannot watch the display device");
                return;
            }
        };
        let mut changes = match props.receive_properties_changed().await {
            Ok(changes) => changes,
            Err(e) => {
                tracing::warn!(%e, "upower: cannot subscribe to changes");
                return;
            }
        };
        loop {
            let _ = updates.send(Update::Battery(battery(&dev).await));
            if changes.next().await.is_none() {
                return;
            }
        }
    });
}

fn watch_networkmanager(updates: &std::sync::mpsc::SyncSender<Update>) {
    zbus::block_on(async {
        let conn = match Connection::system().await {
            Ok(conn) => conn,
            Err(e) => {
                tracing::warn!(%e, "networkmanager: no system bus");
                return;
            }
        };
        let nm = match NetworkManagerProxy::new(&conn).await {
            Ok(nm) => nm,
            Err(e) => {
                tracing::warn!(%e, "networkmanager: not running");
                return;
            }
        };
        let mut changes = nm.receive_connectivity_changed().await;
        loop {
            let level = match nm.connectivity().await {
                Ok(level) => level,
                Err(e) => {
                    tracing::warn!(%e, "networkmanager: connectivity read failed");
                    return;
                }
            };
            let _ = updates.send(Update::Connectivity(Some(connectivity(level))));
            if changes.next().await.is_none() {
                return;
            }
        }
    });
}

/// The real backend: one thread per service, merged into snapshots.
fn follow(tx: &mpsc::UnboundedSender<Arc<Statuses>>) {
    let (utx, urx) = std::sync::mpsc::sync_channel::<Update>(32);
    let spawn = |name: &'static str, watch: fn(&std::sync::mpsc::SyncSender<Update>)| {
        let utx = utx.clone();
        thread::Builder::new()
            .name(name.into())
            .spawn(move || watch(&utx))
            .map_err(|e| tracing::error!(%e, "{name}: cannot start thread"))
            .ok();
    };
    spawn("upower", watch_upower);
    spawn("networkmanager", watch_networkmanager);
    let mut statuses = Statuses::default();
    while let Ok(update) = urx.recv() {
        match update {
            Update::Battery(b) => statuses.battery = b,
            Update::Connectivity(c) => statuses.connectivity = c,
        }
        if tx.unbounded_send(Arc::new(statuses.clone())).is_err() {
            return;
        }
    }
}

/// The preview and dev backend: canned statuses on a slow cycle through every
/// tone the status group can take, so states are checked without a real bus.
fn follow_fake(tx: &mpsc::UnboundedSender<Arc<Statuses>>) {
    const STATES: [Statuses; 3] = [
        Statuses {
            battery: Some(Battery {
                percent: 72,
                charging: false,
                low: false,
            }),
            connectivity: Some(Connectivity::Online),
        },
        Statuses {
            battery: Some(Battery {
                percent: 11,
                charging: false,
                low: true,
            }),
            connectivity: Some(Connectivity::Online),
        },
        Statuses {
            battery: Some(Battery {
                percent: 88,
                charging: true,
                low: false,
            }),
            connectivity: Some(Connectivity::Offline),
        },
    ];
    let mut i = 0;
    loop {
        if tx.unbounded_send(Arc::new(STATES[i].clone())).is_err() {
            return;
        }
        i = (i + 1) % STATES.len();
        #[expect(
            clippy::disallowed_methods,
            reason = "a fixed demo cadence, not polling"
        )]
        thread::sleep(Duration::from_secs(8));
    }
}

fn stream(real: bool) -> impl Stream<Item = Arc<Statuses>> {
    let (tx, rx) = mpsc::unbounded();
    thread::Builder::new()
        .name("services".into())
        .spawn(move || {
            if real {
                follow(&tx);
            } else {
                follow_fake(&tx);
            }
        })
        .map_err(|e| tracing::error!(%e, "services: cannot start thread"))
        .ok();
    rx
}

/// A new [`Statuses`] after each change the status group shows. `real` is the
/// `--real-services` opt-in; without it the fake above runs.
pub fn statuses(real: bool) -> Subscription<Arc<Statuses>> {
    Subscription::run_with(real, |&real| stream(real))
}
