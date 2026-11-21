//! System services, one subscription for everything the bar, quick settings
//! and the OSD read. Each service gets a thread that `block_on`s one
//! connection, re-reading its objects when signals arrive; a merge thread
//! hands the app an immutable [`State`] per change. A missing service just
//! never reports — its fields keep their defaults, and the UI hides the
//! control instead of erroring.
//!
//! Unless `--real-services` is passed the fake backend runs instead: a dev
//! session must never touch real power or network state, and the preview
//! container has no system bus at all. The fake's canned [`State`] answers
//! every command, so quick-settings toggles visibly work in preview.

use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{RecvTimeoutError, SyncSender};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::Duration;

use iced_futures::Subscription;
use iced_futures::futures::channel::mpsc;
use iced_futures::futures::{FutureExt, Stream, StreamExt};
use triton_design::Icon;
use zbus::{Connection, MatchRule, MessageStream, OwnedObjectPath, OwnedVariant, fdo, proxy};

/// Everything quick settings and the OSD read. `None`/`Unknown` fields mean
/// the service is absent — the UI hides the control, never errors.
#[derive(Debug, Clone, Default)]
pub struct State {
    /// Battery + connectivity, exactly what the status group shows.
    pub statuses: Statuses,
    pub net: Net,
    pub bt: Bluetooth,
    pub audio: Audio,
    /// Percent of the primary backlight; `None` when there is none to read.
    pub brightness: Option<u8>,
    pub profile: Profile,
}

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

#[derive(Debug, Clone, Default)]
pub struct Net {
    pub wifi_enabled: bool,
    /// SSID of the active wifi connection, if any.
    pub wifi: Option<String>,
    pub ethernet: bool,
    pub vpn: bool,
    /// Visible networks, strongest first, capped ~8.
    pub aps: Vec<AccessPoint>,
}

#[derive(Debug, Clone)]
pub struct AccessPoint {
    pub ssid: String,
    pub strength: u8,
    pub secured: bool,
    pub connected: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Bluetooth {
    pub powered: bool,
    pub devices: Vec<Device>,
}

#[derive(Debug, Clone)]
pub struct Device {
    pub name: String,
    pub address: String,
    pub connected: bool,
    pub paired: bool,
    pub battery: Option<u8>,
}

#[derive(Debug, Clone, Default)]
pub struct Audio {
    /// 0..=150; over-amplification is allowed, like gnome-control-center.
    pub volume: u8,
    pub muted: bool,
    /// Friendly output name, e.g. "Speakers".
    pub sink: Option<String>,
    pub mic_present: bool,
    pub mic_muted: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Profile {
    #[default]
    Balanced,
    Performance,
    PowerSaver,
    Unknown,
}

/// A change one service reported, merged into [`State`] below.
#[derive(Debug)]
enum Update {
    Battery(Option<Battery>),
    /// NM reports the whole tree at once; connectivity rides along.
    Net(Net, Option<Connectivity>),
    Bluetooth(Bluetooth),
    Audio(Audio),
    Brightness(Option<u8>),
    Profile(Profile),
}

/// The active backend: real services once `state(true)` starts its stream.
/// Commands default to the fake, so a call made before the stream exists
/// still lands somewhere harmless rather than touching the real session.
static REAL: AtomicBool = AtomicBool::new(false);

/// The merge channel, so commands can push a change no watcher can see —
/// brightness is read-once (see `watch_power`), so `set_brightness` reports
/// the value it just wrote instead of waiting for a signal that never comes.
static UPDATES: OnceLock<SyncSender<Update>> = OnceLock::new();

fn real() -> bool {
    REAL.load(Ordering::Relaxed)
}

/// Post a change into the merge. A no-op in fake mode or before the stream.
fn hint(update: Update) {
    if let Some(tx) = UPDATES.get() {
        let _ = tx.send(update);
    }
}

/// A short-lived worker for a command: fire-and-forget, failures logged,
/// and the watchers push the new state back.
fn run_command(name: &'static str, f: impl FnOnce() + Send + 'static) {
    thread::Builder::new()
        .name(name.into())
        .spawn(f)
        .map_err(|e| tracing::error!(%e, "{name}: cannot start thread"))
        .ok();
}

/// A managed-object property converted to its Rust type; a missing key or a
/// wrong shape is `None` — services are loose about what they fill in.
fn prop<T>(value: Option<&OwnedVariant>) -> Option<T>
where
    T: TryFrom<OwnedVariant>,
{
    value.and_then(|v| T::try_from(v.clone()).ok())
}

/// One `GetManagedObjects` call; `None` when the service is absent.
async fn managed(conn: &Connection, destination: &str, path: &str) -> Option<fdo::ManagedObjects> {
    let om = fdo::ObjectManagerProxy::builder(conn)
        .destination(destination)
        .path(path)
        .build()
        .await
        .map_err(|e| tracing::warn!(%e, "{destination}: cannot reach the object manager"))
        .ok()?;
    om.get_managed_objects()
        .await
        .map_err(|e| tracing::warn!(%e, "{destination}: GetManagedObjects failed"))
        .ok()
}

fn connectivity(level: u32) -> Connectivity {
    if level == 4 {
        Connectivity::Online
    } else {
        Connectivity::Offline
    }
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
    /// `WirelessEnabled`, read-write: the wifi toggle.
    #[zbus(property)]
    fn set_wireless_enabled(&self, enabled: bool) -> zbus::Result<()>;
}

#[proxy(
    interface = "org.freedesktop.NetworkManager.Device.Wireless",
    default_service = "org.freedesktop.NetworkManager"
)]
trait Wireless {
    /// `RequestScan(a{sv})`: options stay empty, the defaults are right.
    fn request_scan(&self, options: HashMap<String, OwnedVariant>) -> zbus::Result<()>;
}

#[proxy(interface = "org.bluez.Adapter1", default_service = "org.bluez")]
trait Adapter {
    /// `Powered`, read-write: the bluetooth toggle.
    #[zbus(property)]
    fn set_powered(&self, powered: bool) -> zbus::Result<()>;
}

#[proxy(interface = "org.bluez.Device1", default_service = "org.bluez")]
trait BtDevice {
    fn connect(&self) -> zbus::Result<()>;
    fn disconnect(&self) -> zbus::Result<()>;
}

#[proxy(
    interface = "net.hadess.PowerProfiles",
    default_service = "net.hadess.PowerProfiles",
    default_path = "/net/hadess/PowerProfiles"
)]
trait PowerProfiles {
    /// `ActiveProfile`: "performance", "balanced" or "power-saver".
    #[zbus(property)]
    fn active_profile(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn set_active_profile(&self, profile: &str) -> zbus::Result<()>;
}

#[proxy(
    interface = "org.freedesktop.login1.Session",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1/session/self"
)]
trait LoginSession {
    /// `SetBrightness(su)`: subsystem ("backlight"), device name, value.
    fn set_brightness(&self, subsystem: &str, name: &str, value: u32) -> zbus::Result<()>;
}

#[proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1"
)]
trait LoginManager {
    /// `interactive=false`: the power menu already confirmed.
    fn suspend(&self, interactive: bool) -> zbus::Result<()>;
    fn reboot(&self, interactive: bool) -> zbus::Result<()>;
    fn power_off(&self, interactive: bool) -> zbus::Result<()>;
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

fn watch_upower(updates: &SyncSender<Update>) {
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

/// Every NM object in one pass over the managed tree.
fn net_from(objects: &fdo::ManagedObjects) -> (Net, Option<Connectivity>) {
    const NM: &str = "org.freedesktop.NetworkManager";
    const DEVICE: &str = "org.freedesktop.NetworkManager.Device";
    const WIRELESS: &str = "org.freedesktop.NetworkManager.Device.Wireless";
    const AP: &str = "org.freedesktop.NetworkManager.AccessPoint";
    const ACTIVE: &str = "org.freedesktop.NetworkManager.Connection.Active";
    let mut net = Net::default();
    let mut conn = None;
    // The wifi devices' AP lists, and the AP the connection is on.
    let mut ap_paths = Vec::new();
    let mut active = None;
    for ifaces in objects.values() {
        if let Some(nm) = ifaces.get(NM) {
            net.wifi_enabled = prop::<bool>(nm.get("WirelessEnabled")).unwrap_or_default();
            conn = prop::<u32>(nm.get("Connectivity")).map(connectivity);
        }
        if let Some(dev) = ifaces.get(DEVICE) {
            // NMDeviceState 100: activated. DeviceType 1 is ethernet.
            if prop::<u32>(dev.get("DeviceType")) == Some(1)
                && prop::<u32>(dev.get("State")) == Some(100)
            {
                net.ethernet = true;
            }
        }
        if let Some(wifi) = ifaces.get(WIRELESS) {
            if let Some(ap) = prop::<OwnedObjectPath>(wifi.get("ActiveAccessPoint")) {
                // "/" is NM's "no active AP".
                if ap.as_str() != "/" {
                    active = Some(ap);
                }
            }
            if let Some(aps) = prop::<Vec<OwnedObjectPath>>(wifi.get("AccessPoints")) {
                ap_paths.extend(aps);
            }
        }
        if let Some(ac) = ifaces.get(ACTIVE) {
            net.vpn |= prop::<bool>(ac.get("Vpn")).unwrap_or_default()
                || matches!(
                    prop::<String>(ac.get("Type")).as_deref(),
                    Some("vpn" | "tun" | "wireguard")
                );
        }
    }
    net.wifi = active.as_ref().and_then(|path| ssid(objects, path));
    net.aps = finish_aps(aps(objects, &ap_paths, active.as_ref()));
    (net, conn)
}

/// An AP path's SSID; `None` for hidden networks (they carry no name).
fn ssid(objects: &fdo::ManagedObjects, path: &OwnedObjectPath) -> Option<String> {
    let ap = objects.get(path)?.get("org.freedesktop.NetworkManager.AccessPoint")?;
    let bytes: Vec<u8> = prop(ap.get("Ssid"))?;
    let ssid = String::from_utf8_lossy(&bytes).into_owned();
    (!ssid.is_empty()).then_some(ssid)
}

fn aps(
    objects: &fdo::ManagedObjects,
    paths: &[OwnedObjectPath],
    active: Option<&OwnedObjectPath>,
) -> Vec<AccessPoint> {
    paths
        .iter()
        .filter_map(|path| {
            let ap = objects
                .get(path)?
                .get("org.freedesktop.NetworkManager.AccessPoint")?;
            let ssid = String::from_utf8_lossy(&prop::<Vec<u8>>(ap.get("Ssid"))?).into_owned();
            Some(AccessPoint {
                ssid,
                strength: prop::<u8>(ap.get("Strength")).unwrap_or_default(),
                // Secured means any privacy flags at all.
                secured: prop::<u32>(ap.get("Flags")).unwrap_or_default() != 0
                    || prop::<u32>(ap.get("WpaFlags")).unwrap_or_default() != 0
                    || prop::<u32>(ap.get("RsnFlags")).unwrap_or_default() != 0,
                connected: active == Some(path),
            })
        })
        .collect()
}

/// Hidden and duplicate networks out, strongest first, capped for the panel.
/// A BSSIDs-deep duplicate keeps the strongest signal and the connected flag.
fn finish_aps(aps: Vec<AccessPoint>) -> Vec<AccessPoint> {
    let mut by_ssid: HashMap<String, AccessPoint> = HashMap::new();
    for ap in aps {
        if ap.ssid.is_empty() {
            continue;
        }
        by_ssid
            .entry(ap.ssid.clone())
            .and_modify(|keep| {
                keep.strength = keep.strength.max(ap.strength);
                keep.connected |= ap.connected;
                keep.secured |= ap.secured;
            })
            .or_insert(ap);
    }
    let mut aps: Vec<_> = by_ssid.into_values().collect();
    aps.sort_by(|a, b| b.strength.cmp(&a.strength));
    aps.truncate(8);
    aps
}

/// Paths of the wifi devices, for `RequestScan`.
fn wifi_paths(objects: &fdo::ManagedObjects) -> Vec<OwnedObjectPath> {
    objects
        .iter()
        .filter(|(_, ifaces)| ifaces.contains_key("org.freedesktop.NetworkManager.Device.Wireless"))
        .map(|(path, _)| path.clone())
        .collect()
}

fn watch_net(updates: &SyncSender<Update>) {
    zbus::block_on(async {
        let conn = match Connection::system().await {
            Ok(conn) => conn,
            Err(e) => {
                tracing::warn!(%e, "networkmanager: no system bus");
                return;
            }
        };
        // NM's object manager exposes the whole tree at /org/freedesktop.
        let om = match fdo::ObjectManagerProxy::builder(&conn)
            .destination("org.freedesktop.NetworkManager")
            .path("/org/freedesktop")
            .build()
            .await
        {
            Ok(om) => om,
            Err(e) => {
                tracing::warn!(%e, "networkmanager: cannot reach the object manager");
                return;
            }
        };
        // Any NM signal — devices and APs appear and vanish, properties
        // change — re-reads the whole tree. A scan emits one PropertiesChanged
        // per AP, so a burst is drained and answered once.
        let rule = match MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender("org.freedesktop.NetworkManager")
            .build()
        {
            Ok(rule) => rule,
            Err(e) => {
                tracing::warn!(%e, "networkmanager: cannot build a match rule");
                return;
            }
        };
        let mut changes = match MessageStream::for_match_rule(rule, &conn, None).await {
            Ok(changes) => changes,
            Err(e) => {
                tracing::warn!(%e, "networkmanager: cannot subscribe to signals");
                return;
            }
        };
        loop {
            let Some(objects) = managed_tree(&om).await else {
                return;
            };
            let (net, connectivity) = net_from(&objects);
            let _ = updates.send(Update::Net(net, connectivity));
            if changes.next().await.is_none() {
                return;
            }
            while changes.next().now_or_never().flatten().is_some() {}
        }
    });
}

/// `GetManagedObjects` on an already-built proxy.
async fn managed_tree(om: &fdo::ObjectManagerProxy<'_>) -> Option<fdo::ManagedObjects> {
    om.get_managed_objects()
        .await
        .map_err(|e| tracing::warn!(%e, "GetManagedObjects failed"))
        .ok()
}

/// Adapters and devices in one pass over BlueZ's managed tree.
fn bt_from(objects: &fdo::ManagedObjects) -> Bluetooth {
    const ADAPTER: &str = "org.bluez.Adapter1";
    const DEVICE: &str = "org.bluez.Device1";
    const BATTERY: &str = "org.bluez.Battery1";
    // The toggle powers every adapter, so the read side is "all on". No
    // adapters at all reads as off — the control is hidden anyway.
    let powered = {
        let adapters: Vec<bool> = objects
            .values()
            .filter_map(|ifaces| ifaces.get(ADAPTER))
            .filter_map(|a| prop::<bool>(a.get("Powered")))
            .collect();
        !adapters.is_empty() && adapters.iter().all(|&p| p)
    };
    let mut devices: Vec<Device> = objects
        .iter()
        .filter_map(|(_, ifaces)| {
            let dev = ifaces.get(DEVICE)?;
            let address = prop::<String>(dev.get("Address")).unwrap_or_default();
            Some(Device {
                name: prop::<String>(dev.get("Name"))
                    .or_else(|| prop::<String>(dev.get("Alias")))
                    .unwrap_or_else(|| address.clone()),
                address,
                connected: prop::<bool>(dev.get("Connected")).unwrap_or_default(),
                paired: prop::<bool>(dev.get("Paired")).unwrap_or_default(),
                // BlueZ puts the battery on the same object, as Battery1.
                battery: ifaces
                    .get(BATTERY)
                    .and_then(|b| prop::<u8>(b.get("Percentage")))
                    .or_else(|| prop::<u8>(dev.get("BatteryPercentage"))),
            })
        })
        .collect();
    devices.sort_by(|a, b| a.name.cmp(&b.name));
    Bluetooth { powered, devices }
}

/// A device's object path from its "AA:BB:…" address.
fn device_path(objects: &fdo::ManagedObjects, address: &str) -> Option<OwnedObjectPath> {
    objects.iter().find_map(|(path, ifaces)| {
        let dev = ifaces.get("org.bluez.Device1")?;
        let found: String = prop(dev.get("Address"))?;
        if found.eq_ignore_ascii_case(address) {
            Some(path.clone())
        } else {
            None
        }
    })
}

fn watch_bluez(updates: &SyncSender<Update>) {
    zbus::block_on(async {
        let conn = match Connection::system().await {
            Ok(conn) => conn,
            Err(e) => {
                tracing::warn!(%e, "bluez: no system bus");
                return;
            }
        };
        let om = match fdo::ObjectManagerProxy::builder(&conn)
            .destination("org.bluez")
            .path("/")
            .build()
            .await
        {
            Ok(om) => om,
            Err(e) => {
                tracing::warn!(%e, "bluez: cannot reach the object manager");
                return;
            }
        };
        // InterfacesAdded/Removed and PropertiesChanged all arrive on org.bluez.
        let rule = match MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender("org.bluez")
            .build()
        {
            Ok(rule) => rule,
            Err(e) => {
                tracing::warn!(%e, "bluez: cannot build a match rule");
                return;
            }
        };
        let mut changes = match MessageStream::for_match_rule(rule, &conn, None).await {
            Ok(changes) => changes,
            Err(e) => {
                tracing::warn!(%e, "bluez: cannot subscribe to signals");
                return;
            }
        };
        loop {
            let Some(objects) = managed_tree(&om).await else {
                return;
            };
            let _ = updates.send(Update::Bluetooth(bt_from(&objects)));
            if changes.next().await.is_none() {
                return;
            }
            while changes.next().now_or_never().flatten().is_some() {}
        }
    });
}

// ── Audio: wpctl/pactl, the transitional PipeWire backend ────────────────────
// PipeWire volume has no D-Bus API, and libpipewire FFI needs headers nx does
// not ship, so Phase 1 shells out: `wpctl` for get/set, `pactl subscribe` for
// change events. Spawn is the one process API clippy allows; every call below
// spawns, reads stdout through a pipe and reaps the child — never
// `Command::output`/`status`. A missing tool warns once and audio stays at
// its defaults — the UI hides the controls.
// TODO(pipewire): replace with the libpipewire registry API (or WirePlumber's
// mixer API) once the native dependency situation allows it.

/// Missing tools are worth one warning, not one per call.
fn missing(cmd: &str, e: &std::io::Error) {
    static WARNED: AtomicBool = AtomicBool::new(false);
    if !WARNED.swap(true, Ordering::Relaxed) {
        tracing::warn!(%e, "audio: {cmd} unavailable; volume controls stay hidden");
    }
}

/// Spawn a tool with piped stdout, collect it, reap the child. `None` means
/// "tool missing or call failed" — never fatal.
fn run(cmd: &str, args: &[&str]) -> Option<String> {
    let mut child = match Command::new(cmd)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(e) => {
            missing(cmd, &e);
            return None;
        }
    };
    let mut out = String::new();
    let read = child
        .stdout
        .take()
        .and_then(|mut stdout| stdout.read_to_string(&mut out).ok())
        .is_some();
    match (read, child.wait()) {
        (true, Ok(status)) if status.success() => Some(out),
        _ => None,
    }
}

/// "Volume: 0.62" or "Volume: 0.62 [MUTED]" → (percent, muted).
fn parse_volume(out: &str) -> Option<(u8, bool)> {
    let rest = out.trim().strip_prefix("Volume:")?.trim_start();
    let frac: f64 = rest.split_whitespace().next()?.parse().ok()?;
    Some(((frac * 100.0).round() as u8, rest.contains("[MUTED]")))
}

/// `node.description = "Speakers"`, with `node.nick` as the fallback. The
/// `wpctl inspect` table marks lines with a `*` and tabs.
fn description(out: &str) -> Option<String> {
    for key in ["node.description", "node.nick"] {
        for line in out.lines() {
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            if k.trim().trim_start_matches('*').trim() == key {
                let name = v.trim().trim_matches('"');
                if !name.is_empty() {
                    return Some(name.to_string());
                }
            }
        }
    }
    None
}

/// (percent, muted) of a `@DEFAULT_AUDIO_*@` node.
fn volume(target: &str) -> Option<(u8, bool)> {
    run("wpctl", &["get-volume", target]).and_then(|out| parse_volume(&out))
}

/// `wpctl inspect`'s friendliest name for the default sink.
fn sink_name() -> Option<String> {
    run("wpctl", &["inspect", "@DEFAULT_AUDIO_SINK@"]).and_then(|out| description(&out))
}

/// (present, muted) for the default source. A machine with no mic keeps
/// `mic_present = false` — the UI hides the control. Monitors of outputs are
/// not mics, so names ending `.monitor` don't count.
fn mic() -> (bool, bool) {
    let present = run("pactl", &["list", "short", "sources"])
        .map(|out| {
            out.lines().any(|line| {
                line.split_whitespace()
                    .nth(1)
                    .is_some_and(|name| !name.ends_with(".monitor"))
            })
        })
        .unwrap_or_default();
    let muted = present && volume("@DEFAULT_AUDIO_SOURCE@").is_some_and(|(_, m)| m);
    (present, muted)
}

fn read_audio() -> Option<Audio> {
    let (volume, muted) = volume("@DEFAULT_AUDIO_SINK@")?;
    let (mic_present, mic_muted) = mic();
    Some(Audio {
        volume,
        muted,
        sink: sink_name(),
        mic_present,
        mic_muted,
    })
}

fn watch_audio(updates: &SyncSender<Update>) {
    // A first read before subscribing, so the panel isn't empty until the
    // first event.
    if let Some(audio) = read_audio() {
        let _ = updates.send(Update::Audio(audio));
    }
    let mut child = match Command::new("pactl")
        .arg("subscribe")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(e) => {
            missing("pactl", &e);
            return;
        }
    };
    let Some(stdout) = child.stdout.take() else {
        return;
    };
    // Lines like "Event 'change' on sink #45": any change re-reads the state.
    for line in BufReader::new(stdout).lines() {
        match line {
            Ok(line)
                if line.contains("change") || line.contains("new") || line.contains("remove") =>
            {
                if let Some(audio) = read_audio() {
                    let _ = updates.send(Update::Audio(audio));
                }
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    let _ = child.wait();
}

// ── Brightness, power profile, power actions ─────────────────────────────────

/// The primary backlight: prefer the `raw` device (real panel control) over
/// `firmware`/`platform` entries, which on some drivers are shadows of the
/// raw one.
fn backlight() -> Option<Backlight> {
    let root = Path::new("/sys/class/backlight");
    let mut names: Vec<String> = fs::read_dir(root)
        .ok()?
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect();
    names.sort();
    let name = names
        .iter()
        .find(|name| read_str(&root.join(name).join("type")).as_deref() == Some("raw"))
        .or_else(|| names.first())?
        .clone();
    let dir = root.join(&name);
    let max: u32 = read_str(&dir.join("max_brightness"))?.parse().ok()?;
    (max > 0).then_some(Backlight { name, dir, max })
}

#[derive(Debug)]
struct Backlight {
    name: String,
    dir: PathBuf,
    max: u32,
}

fn read_str(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

fn backlight_percent() -> Option<u8> {
    let bl = backlight()?;
    let actual: u32 = read_str(&bl.dir.join("actual_brightness"))?.parse().ok()?;
    Some(((actual * 100 / bl.max).min(100)) as u8)
}

/// The logind session's `SetBrightness`, with a direct sysfs write as the
/// documented fallback (older logind, unusual seats).
fn brightness_set(percent: u8) {
    run_command("brightness", move || {
        let Some(bl) = backlight() else {
            tracing::warn!("brightness: no backlight device");
            return;
        };
        let value = (bl.max * u32::from(percent) / 100).max(1);
        let res: zbus::Result<()> = zbus::block_on(async {
            let conn = Connection::system().await?;
            LoginSessionProxy::new(&conn)
                .await?
                .set_brightness("backlight", &bl.name, value)
                .await
        });
        if let Err(e) = res {
            tracing::debug!(%e, "brightness: logind refused, writing sysfs");
            if let Err(e) = fs::write(bl.dir.join("brightness"), value.to_string()) {
                tracing::warn!(%e, "brightness: cannot set");
                return;
            }
        }
        // No watcher exists for backlight — report the value we just wrote.
        hint(Update::Brightness(Some(percent)));
    });
}

fn profile_name(profile: Profile) -> &'static str {
    match profile {
        Profile::Performance => "performance",
        // Unknown writes the default rather than nothing at all.
        Profile::Balanced | Profile::Unknown => "balanced",
        Profile::PowerSaver => "power-saver",
    }
}

fn profile_from(name: &str) -> Profile {
    match name {
        "performance" => Profile::Performance,
        "power-saver" => Profile::PowerSaver,
        "balanced" => Profile::Balanced,
        _ => Profile::Unknown,
    }
}

fn pp_set_profile(profile: Profile) {
    run_command("power-profiles", move || {
        zbus::block_on(async {
            let conn = match Connection::system().await {
                Ok(conn) => conn,
                Err(e) => {
                    tracing::warn!(%e, "power-profiles: no system bus");
                    return;
                }
            };
            let pp = match PowerProfilesProxy::new(&conn).await {
                Ok(pp) => pp,
                Err(e) => {
                    tracing::warn!(%e, "power-profiles: not running");
                    return;
                }
            };
            if let Err(e) = pp.set_active_profile(profile_name(profile)).await {
                tracing::warn!(%e, "power-profiles: set failed");
            }
        });
    });
}

fn watch_profile(updates: &SyncSender<Update>) {
    zbus::block_on(async {
        let conn = match Connection::system().await {
            Ok(conn) => conn,
            Err(e) => {
                tracing::warn!(%e, "power-profiles: no system bus");
                return;
            }
        };
        let pp = match PowerProfilesProxy::new(&conn).await {
            Ok(pp) => pp,
            Err(e) => {
                tracing::warn!(%e, "power-profiles: not running");
                return;
            }
        };
        let mut changes = pp.receive_active_profile_changed().await;
        loop {
            let name = match pp.active_profile().await {
                Ok(name) => name,
                Err(e) => {
                    tracing::warn!(%e, "power-profiles: ActiveProfile read failed");
                    return;
                }
            };
            let _ = updates.send(Update::Profile(profile_from(&name)));
            if changes.next().await.is_none() {
                return;
            }
        }
    });
}

/// Brightness is read once — nothing emits a signal for sysfs backlight, and
/// `set_brightness` pushes its own update through `hint`. The power profile
/// daemon is watched the usual way below.
fn watch_power(updates: &SyncSender<Update>) {
    if let Some(percent) = backlight_percent() {
        let _ = updates.send(Update::Brightness(Some(percent)));
    }
    watch_profile(updates);
}

#[derive(Debug, Clone, Copy)]
enum PowerAction {
    Suspend,
    Reboot,
    PowerOff,
}

/// `Suspend`/`Reboot`/`PowerOff` with `interactive=false`.
fn logind(action: PowerAction) {
    run_command("logind", move || {
        zbus::block_on(async {
            let conn = match Connection::system().await {
                Ok(conn) => conn,
                Err(e) => {
                    tracing::warn!(%e, "logind: no system bus");
                    return;
                }
            };
            let mgr = match LoginManagerProxy::new(&conn).await {
                Ok(mgr) => mgr,
                Err(e) => {
                    tracing::warn!(%e, "logind: manager unreachable");
                    return;
                }
            };
            let res = match action {
                PowerAction::Suspend => mgr.suspend(false).await,
                PowerAction::Reboot => mgr.reboot(false).await,
                PowerAction::PowerOff => mgr.power_off(false).await,
            };
            if let Err(e) = res {
                tracing::warn!(%e, ?action, "logind: power action failed");
            }
        });
    });
}

// ── Commands (real backend) ──────────────────────────────────────────────────

fn nm_set_wifi(enabled: bool) {
    run_command("nm-wifi", move || {
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
            if let Err(e) = nm.set_wireless_enabled(enabled).await {
                tracing::warn!(%e, "networkmanager: wifi toggle failed");
            }
        });
    });
}

/// `RequestScan` on every wifi device — the QS wifi page calls it on open.
fn nm_rescan() {
    run_command("nm-scan", || {
        zbus::block_on(async {
            let conn = match Connection::system().await {
                Ok(conn) => conn,
                Err(e) => {
                    tracing::warn!(%e, "networkmanager: no system bus");
                    return;
                }
            };
            let Some(objects) = managed(&conn, "org.freedesktop.NetworkManager", "/org/freedesktop")
                .await
            else {
                return;
            };
            for path in wifi_paths(&objects) {
                let dev = match WirelessProxy::builder(&conn).path(&path).build().await {
                    Ok(dev) => dev,
                    Err(e) => {
                        tracing::warn!(%e, "networkmanager: wifi device unreachable");
                        continue;
                    }
                };
                if let Err(e) = dev.request_scan(HashMap::new()).await {
                    tracing::warn!(%e, "networkmanager: scan request failed");
                }
            }
        });
    });
}

/// Every adapter follows the toggle, matching how desktops present bluetooth.
fn bt_set_power(on: bool) {
    run_command("bluez-power", move || {
        zbus::block_on(async {
            let conn = match Connection::system().await {
                Ok(conn) => conn,
                Err(e) => {
                    tracing::warn!(%e, "bluez: no system bus");
                    return;
                }
            };
            let Some(objects) = managed(&conn, "org.bluez", "/").await else {
                return;
            };
            for (path, ifaces) in &objects {
                if !ifaces.contains_key("org.bluez.Adapter1") {
                    continue;
                }
                let adapter = match AdapterProxy::builder(&conn).path(path).build().await {
                    Ok(adapter) => adapter,
                    Err(e) => {
                        tracing::warn!(%e, "bluez: adapter unreachable");
                        continue;
                    }
                };
                if let Err(e) = adapter.set_powered(on).await {
                    tracing::warn!(%e, "bluez: power toggle failed");
                }
            }
        });
    });
}

fn bt_device_call(address: &str, connect: bool) {
    let address = address.to_string();
    run_command("bluez-device", move || {
        zbus::block_on(async {
            let conn = match Connection::system().await {
                Ok(conn) => conn,
                Err(e) => {
                    tracing::warn!(%e, "bluez: no system bus");
                    return;
                }
            };
            let Some(objects) = managed(&conn, "org.bluez", "/").await else {
                return;
            };
            let Some(path) = device_path(&objects, &address) else {
                tracing::warn!(%address, "bluez: no such device");
                return;
            };
            let dev = match BtDeviceProxy::builder(&conn).path(&path).build().await {
                Ok(dev) => dev,
                Err(e) => {
                    tracing::warn!(%e, %address, "bluez: device unreachable");
                    return;
                }
            };
            let res = if connect {
                dev.connect().await
            } else {
                dev.disconnect().await
            };
            if let Err(e) = res {
                tracing::warn!(%e, %address, "bluez: device call failed");
            }
        });
    });
}

fn audio_set_volume(percent: f32) {
    run_command("audio-volume", move || {
        // -l 1.5: the UI's 0..=150 range, same as gnome-control-center.
        let frac = format!("{:.2}", percent.clamp(0.0, 150.0) / 100.0);
        let _ = run(
            "wpctl",
            &[
                "set-volume",
                "-l",
                "1.5",
                "@DEFAULT_AUDIO_SINK@",
                frac.as_str(),
            ],
        );
    });
}

fn audio_set_mute(muted: bool) {
    run_command("audio-mute", move || {
        let _ = run(
            "wpctl",
            &[
                "set-mute",
                "@DEFAULT_AUDIO_SINK@",
                if muted { "1" } else { "0" },
            ],
        );
    });
}

fn audio_set_mic_mute(muted: bool) {
    run_command("audio-mic-mute", move || {
        let _ = run(
            "wpctl",
            &[
                "set-mute",
                "@DEFAULT_AUDIO_SOURCE@",
                if muted { "1" } else { "0" },
            ],
        );
    });
}

// ── The fake backend ─────────────────────────────────────────────────────────
// Canned state behind a mutex; the command functions mutate it so toggles
// visibly work in preview, and a slow cadence moves the status pill through
// every tone so screenshots vary. Commands are rare user actions, so a plain
// mutex is enough — no lock is ever held across an await or a channel send.

static FAKE: OnceLock<Mutex<State>> = OnceLock::new();
/// Wakes the fake follower after a command mutates the state.
static WAKE: OnceLock<SyncSender<()>> = OnceLock::new();

fn fake() -> &'static Mutex<State> {
    FAKE.get_or_init(|| Mutex::new(canned()))
}

fn fake_state() -> State {
    fake().lock().map(|s| s.clone()).unwrap_or_default()
}

/// Mutate the fake snapshot and wake the follower.
fn fake_mutate(f: impl FnOnce(&mut State)) {
    if let Ok(mut state) = fake().lock() {
        f(&mut state);
    }
    if let Some(tx) = WAKE.get() {
        let _ = tx.send(());
    }
}

/// The preview's machine: a laptop mid-afternoon.
fn canned() -> State {
    State {
        statuses: Statuses {
            battery: Some(Battery {
                percent: 72,
                charging: false,
                low: false,
            }),
            connectivity: Some(Connectivity::Online),
        },
        net: Net {
            wifi_enabled: true,
            wifi: Some("Neptuno-5G".into()),
            ethernet: true,
            vpn: false,
            aps: vec![
                AccessPoint {
                    ssid: "Neptuno-5G".into(),
                    strength: 82,
                    secured: true,
                    connected: true,
                },
                AccessPoint {
                    ssid: "Neptuno".into(),
                    strength: 64,
                    secured: true,
                    connected: false,
                },
                AccessPoint {
                    ssid: "Café Libro".into(),
                    strength: 41,
                    secured: true,
                    connected: false,
                },
                AccessPoint {
                    ssid: "Vodafone-8F2A".into(),
                    strength: 28,
                    secured: true,
                    connected: false,
                },
            ],
        },
        bt: Bluetooth {
            powered: true,
            devices: vec![
                Device {
                    name: "WH-1000XM5".into(),
                    address: "28:6F:40:A1:B2:C3".into(),
                    connected: true,
                    paired: true,
                    battery: Some(58),
                },
                Device {
                    name: "MX Master 3S".into(),
                    address: "D8:3A:DD:0E:71:9F".into(),
                    connected: false,
                    paired: true,
                    battery: None,
                },
            ],
        },
        audio: Audio {
            volume: 62,
            muted: false,
            sink: Some("Family 17h HD Audio".into()),
            mic_present: true,
            mic_muted: false,
        },
        brightness: Some(70),
        profile: Profile::Balanced,
    }
}

fn fake_set_wifi(state: &mut State, enabled: bool) {
    state.net.wifi_enabled = enabled;
    // A radio that comes back reconnects to its usual network.
    state.net.wifi = enabled.then(|| "Neptuno-5G".to_string());
    for ap in &mut state.net.aps {
        ap.connected = enabled && ap.ssid == "Neptuno-5G";
    }
    state.statuses.connectivity = Some(if enabled {
        Connectivity::Online
    } else {
        Connectivity::Offline
    });
}

fn fake_bt_power(state: &mut State, on: bool) {
    state.bt.powered = on;
    if !on {
        for dev in &mut state.bt.devices {
            dev.connected = false;
        }
    }
}

fn fake_bt_connect(state: &mut State, address: &str) {
    if let Some(dev) = state.bt.devices.iter_mut().find(|d| d.address == address) {
        dev.connected = true;
        dev.paired = true;
    }
}

fn fake_bt_disconnect(state: &mut State, address: &str) {
    if let Some(dev) = state.bt.devices.iter_mut().find(|d| d.address == address) {
        dev.connected = false;
    }
}

fn fake_set_volume(state: &mut State, percent: f32) {
    state.audio.volume = percent.clamp(0.0, 150.0).round() as u8;
}

fn fake_set_brightness(state: &mut State, percent: f32) {
    state.brightness = Some(percent.clamp(1.0, 100.0).round() as u8);
}

/// The preview and dev backend: the canned state, nudged through the status
/// pill's tones on a slow demo cadence. Command mutations take priority —
/// the cycle only touches `statuses`, so toggles stay put.
fn follow_fake(tx: &mpsc::UnboundedSender<Arc<State>>) {
    let (wake_tx, wake_rx) = std::sync::mpsc::sync_channel::<()>(4);
    let _ = WAKE.set(wake_tx);
    // The same pill cycle the old statuses() fake ran: every tone shows up.
    const PILL: [Statuses; 4] = [
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
        // Charging alone already warms the pill.
        Statuses {
            battery: Some(Battery {
                percent: 88,
                charging: true,
                low: false,
            }),
            connectivity: Some(Connectivity::Online),
        },
        Statuses {
            battery: Some(Battery {
                percent: 88,
                charging: false,
                low: false,
            }),
            connectivity: Some(Connectivity::Offline),
        },
    ];
    let mut i = 0;
    loop {
        if tx.unbounded_send(Arc::new(fake_state())).is_err() {
            return;
        }
        match wake_rx.recv_timeout(Duration::from_secs(8)) {
            Err(RecvTimeoutError::Timeout) => {
                if let Ok(mut state) = fake().lock() {
                    state.statuses = PILL[i].clone();
                }
                i = (i + 1) % PILL.len();
            }
            // A command poked us: resend right away so the toggle feels live.
            Ok(()) | Err(RecvTimeoutError::Disconnected) => {}
        }
    }
}

/// The real backend: one thread per service, merged into snapshots.
fn follow(tx: &mpsc::UnboundedSender<Arc<State>>) {
    let (utx, urx) = std::sync::mpsc::sync_channel::<Update>(32);
    let _ = UPDATES.set(utx.clone());
    let spawn = |name: &'static str, watch: fn(&SyncSender<Update>)| {
        let utx = utx.clone();
        thread::Builder::new()
            .name(name.into())
            .spawn(move || watch(&utx))
            .map_err(|e| tracing::error!(%e, "{name}: cannot start thread"))
            .ok();
    };
    spawn("upower", watch_upower);
    spawn("networkmanager", watch_net);
    spawn("bluez", watch_bluez);
    spawn("audio", watch_audio);
    spawn("power", watch_power);
    let mut state = State::default();
    while let Ok(update) = urx.recv() {
        match update {
            Update::Battery(b) => state.statuses.battery = b,
            Update::Net(net, connectivity) => {
                state.net = net;
                state.statuses.connectivity = connectivity;
            }
            Update::Bluetooth(bt) => state.bt = bt,
            Update::Audio(audio) => state.audio = audio,
            Update::Brightness(b) => state.brightness = b,
            Update::Profile(p) => state.profile = p,
        }
        if tx.unbounded_send(Arc::new(state.clone())).is_err() {
            return;
        }
    }
}

fn stream(real: bool) -> impl Stream<Item = Arc<State>> {
    let (tx, rx) = mpsc::unbounded();
    thread::Builder::new()
        .name("services".into())
        .spawn(move || {
            // Which backend the command functions act on: set once, at stream
            // start; commands made before this run hit the fake.
            REAL.store(real, Ordering::Relaxed);
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

/// A new [`State`] after each change quick settings or the OSD shows: one
/// subscription, same pattern as before — worker threads → merge → `Arc`
/// snapshot. `real` is the `--real-services` opt-in; without it the fake
/// backend runs. Replaces `statuses()`, which only covered the status group.
pub fn state(real: bool) -> Subscription<Arc<State>> {
    Subscription::run_with(real, |&real| stream(real))
}

/// Turn the wifi radios on or off.
pub fn set_wifi(enabled: bool) {
    if real() {
        nm_set_wifi(enabled);
    } else {
        fake_mutate(|state| fake_set_wifi(state, enabled));
    }
}

/// Ask NM for a fresh AP scan — the QS wifi page calls this when it opens.
/// Fake mode has nothing to scan; the canned list stands.
pub fn wifi_rescan() {
    if real() {
        nm_rescan();
    }
}

/// Power the bluetooth adapters on or off.
pub fn bt_power(on: bool) {
    if real() {
        bt_set_power(on);
    } else {
        fake_mutate(|state| fake_bt_power(state, on));
    }
}

/// Connect a device by its "AA:BB:…" address.
pub fn bt_connect(address: &str) {
    if real() {
        bt_device_call(address, true);
    } else {
        let address = address.to_string();
        fake_mutate(move |state| fake_bt_connect(state, &address));
    }
}

/// Disconnect a device by address.
pub fn bt_disconnect(address: &str) {
    if real() {
        bt_device_call(address, false);
    } else {
        let address = address.to_string();
        fake_mutate(move |state| fake_bt_disconnect(state, &address));
    }
}

/// Output volume, clamped to the UI's 0..=150 range.
pub fn set_volume(percent: f32) {
    if real() {
        audio_set_volume(percent);
    } else {
        fake_mutate(|state| fake_set_volume(state, percent));
    }
}

/// Mute or unmute the output.
pub fn set_mute(muted: bool) {
    if real() {
        audio_set_mute(muted);
    } else {
        fake_mutate(|state| state.audio.muted = muted);
    }
}

/// Mute or unmute the mic.
pub fn set_mic_mute(muted: bool) {
    if real() {
        audio_set_mic_mute(muted);
    } else {
        fake_mutate(|state| state.audio.mic_muted = muted);
    }
}

/// Panel backlight, clamped to 1..=100 — off is the power menu's job.
pub fn set_brightness(percent: f32) {
    let percent = percent.clamp(1.0, 100.0);
    if real() {
        brightness_set(percent.round() as u8);
    } else {
        fake_mutate(|state| fake_set_brightness(state, percent));
    }
}

/// The system's power profile.
pub fn set_profile(profile: Profile) {
    if real() {
        pp_set_profile(profile);
    } else {
        fake_mutate(|state| state.profile = profile);
    }
}

/// Suspend the machine. A no-op in preview — dev machines stay awake.
pub fn suspend() {
    if real() {
        logind(PowerAction::Suspend);
    } else {
        tracing::info!("fake services: suspend ignored in preview");
    }
}

/// Reboot the machine. A no-op in preview.
pub fn reboot() {
    if real() {
        logind(PowerAction::Reboot);
    } else {
        tracing::info!("fake services: reboot ignored in preview");
    }
}

/// Power the machine off. A no-op in preview.
pub fn power_off() {
    if real() {
        logind(PowerAction::PowerOff);
    } else {
        tracing::info!("fake services: power off ignored in preview");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ap(ssid: &str, strength: u8) -> AccessPoint {
        AccessPoint {
            ssid: ssid.into(),
            strength,
            secured: true,
            connected: false,
        }
    }

    #[test]
    fn aps_deduped_sorted_capped() {
        let mut aps: Vec<_> = (0..12).map(|i| ap(&format!("net{i}"), 10 + i)).collect();
        // A weaker duplicate loses; the same SSID connected at lower strength
        // still shows connected.
        let mut dup = ap("net3", 99);
        aps.push(ap("net3", 20));
        aps.push(dup.clone());
        dup.connected = true;
        dup.strength = 30;
        aps.push(dup);
        aps.push(ap("", 95)); // hidden: no SSID, dropped
        let aps = finish_aps(aps);
        assert_eq!(aps.len(), 8);
        assert!(aps.iter().all(|a| !a.ssid.is_empty()));
        assert!(
            aps.windows(2)
                .all(|w| w[0].strength >= w[1].strength),
            "strongest first"
        );
        let net3 = aps.iter().find(|a| a.ssid == "net3").unwrap();
        assert_eq!(net3.strength, 99);
        assert!(net3.connected);
    }

    #[test]
    fn volume_lines() {
        assert_eq!(parse_volume("Volume: 0.62"), Some((62, false)));
        assert_eq!(parse_volume("Volume: 0.62 [MUTED]"), Some((62, true)));
        assert_eq!(parse_volume("Volume: 1.50"), Some((150, false)));
        assert_eq!(parse_volume("Volume: 0.00 [MUTED]"), Some((0, true)));
        assert_eq!(parse_volume("Volume: nonsense"), None);
        assert_eq!(parse_volume(""), None);
    }

    #[test]
    fn sink_description() {
        let out = "\tid 45, type PipeWire:Interface:Node\n * \tnode.description = \"Family 17h HD Audio Controller\"\n * \tnode.name = \"alsa_output.pci\"\n";
        assert_eq!(
            description(out).as_deref(),
            Some("Family 17h HD Audio Controller")
        );
        assert_eq!(description("nothing useful"), None);
    }

    #[test]
    fn fake_commands_update_state() {
        // REAL is false in tests: the public commands hit the fake store.
        set_volume(220.0);
        assert_eq!(fake_state().audio.volume, 150);
        set_brightness(0.0);
        assert_eq!(fake_state().brightness, Some(1));
        set_wifi(false);
        let s = fake_state();
        assert!(!s.net.wifi_enabled);
        assert_eq!(s.net.wifi, None);
        assert_eq!(s.statuses.connectivity, Some(Connectivity::Offline));
        bt_power(false);
        assert!(fake_state().bt.devices.iter().all(|d| !d.connected));
    }

    #[test]
    fn fake_device_connect() {
        let addr = fake_state().bt.devices[1].address.clone();
        bt_connect(&addr);
        let s = fake_state();
        assert!(s.bt.devices[1].connected);
        bt_disconnect(&addr);
        assert!(!fake_state().bt.devices[1].connected);
    }
}
