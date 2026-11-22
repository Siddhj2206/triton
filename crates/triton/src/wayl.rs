//! A second Wayland connection (iced owns the first) for the two protocols the
//! shell drives itself: ext-idle-notify for the dim/lock/suspend timers and
//! wlr-gamma-control for night light (roadmap §2). One worker thread owns the
//! connection, its event queue and every protocol object; callers reach it
//! only through commands on a channel — `start()` hands out the night-light
//! side, `idles()` the idle side.
//!
//! Every absence degrades: no display, no seat, no notifier or no gamma
//! manager each log a warning and narrow what works rather than failing.
//! Command senders wake the blocked dispatch with a `wl_display.sync` request:
//! the compositor's `done` reply makes the socket readable, which is what lets
//! a single thread multiplex the channel and protocol events without polling.

use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Seek, Write};
use std::os::unix::io::AsFd;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::thread;
use std::time::Duration;

use iced_futures::Subscription;
use iced_futures::futures::Stream;
use iced_futures::futures::channel::mpsc;
use wayland_client::protocol::wl_callback::{self, WlCallback};
use wayland_client::protocol::wl_display::WlDisplay;
use wayland_client::protocol::wl_output::{self, WlOutput};
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::protocol::wl_seat::{self, WlSeat};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle};
use wayland_protocols::ext::idle_notify::v1::client::{
    ext_idle_notification_v1, ext_idle_notifier_v1,
};
use wayland_protocols_wlr::gamma_control::v1::client::{
    zwlr_gamma_control_manager_v1, zwlr_gamma_control_v1,
};

use ext_idle_notification_v1::ExtIdleNotificationV1;
use ext_idle_notifier_v1::ExtIdleNotifierV1;
use zwlr_gamma_control_manager_v1::ZwlrGammaControlManagerV1;
use zwlr_gamma_control_v1::ZwlrGammaControlV1;

/// An idle notification from ext-idle-notify.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdleEvent {
    /// The seat went idle past the notification at this index of the timeouts
    /// the subscription was created with.
    Idle(usize),
    /// Activity resumed; the index of the notification that fired
    /// (`usize::MAX` is reserved for "any").
    Resumed(usize),
}

/// Night light: a clonable handle to every output's gamma control, held over
/// the command channel to the worker.
#[derive(Debug, Clone)]
pub struct Night {
    shared: &'static Shared,
}

impl Night {
    /// Set the color temperature on every output (Kelvin; ~4500 night,
    /// ~6500 day). One output's failure logs a warning and skips that output;
    /// the control objects themselves are owned by the worker thread.
    pub fn set_temperature(&self, kelvin: u16) {
        self.shared.send(Cmd::Temperature(kelvin));
    }

    /// Dim every output by `factor` (1.0 = full brightness): the idle dim
    /// stage. Composes with the temperature — both fold into the one table.
    pub fn set_dim(&self, factor: f32) {
        self.shared.send(Cmd::Dim(factor));
    }
}

/// Starts the worker thread's connection and returns the `Night` handle, or
/// `None` when there is no display or the compositor offers no
/// `zwlr_gamma_control_manager_v1` — the caller hides the night-light toggle.
/// Idle notifications share this one connection and work even when `None` is
/// returned. Connects lazily on first use, so `idles()` may run first.
pub fn start() -> Option<Night> {
    let shared = shared()?;
    shared.gamma.then_some(Night { shared })
}

/// A subscription registering one `ext_idle_notification_v1` per timeout on
/// the first seat and forwarding idled/resumed. New subscriptions re-register,
/// so the app re-subscribes when its timeouts change.
pub fn idles(timeouts: &'static [Duration]) -> Subscription<IdleEvent> {
    fn stream(timeouts: &'static [Duration]) -> impl Stream<Item = IdleEvent> {
        let (tx, rx) = mpsc::unbounded();
        if let Some(shared) = shared() {
            shared.send(Cmd::Watch {
                timeouts: timeouts.iter().map(|t| millis(*t)).collect(),
                tx,
            });
        } else {
            tracing::warn!("wayl: no wayland connection; idle timeouts disabled");
        }
        rx
    }
    Subscription::run_with(timeouts, |t| stream(t))
}

/// The worker's shared handle: the command channel plus what a caller thread
/// needs to poke the worker awake.
#[derive(Debug)]
struct Shared {
    tx: Sender<Cmd>,
    conn: Connection,
    display: WlDisplay,
    qh: QueueHandle<State>,
    /// The compositor advertised `zwlr_gamma_control_manager_v1`.
    gamma: bool,
}

impl Shared {
    fn send(&self, cmd: Cmd) {
        if self.tx.send(cmd).is_err() {
            tracing::warn!("wayl: command dropped; worker thread is gone");
            return;
        }
        // The callback's `done` reply makes `blocking_dispatch` return, so the
        // worker drains the command at once instead of at the next protocol
        // event.
        let _done = self.display.sync(&self.qh, ());
        if let Err(e) = self.conn.flush() {
            tracing::warn!(%e, "wayl: cannot wake the worker");
        }
    }
}

/// Connects lazily on first use so `idles()` and `start()` share the one
/// connection in either order.
fn shared() -> Option<&'static Shared> {
    static SHARED: OnceLock<Option<Shared>> = OnceLock::new();
    SHARED.get_or_init(connect).as_ref()
}

/// Commands the app threads hand the worker.
#[derive(Debug)]
enum Cmd {
    /// Apply this color temperature on every gamma-controlled output.
    Temperature(u16),
    /// Scale brightness by this factor on every output (the idle dim stage).
    Dim(f32),
    /// Register one idle notification per timeout; events go to `tx`.
    Watch {
        timeouts: Vec<u32>,
        tx: mpsc::UnboundedSender<IdleEvent>,
    },
}

/// udata of an `ext_idle_notification_v1`: which of its subscription's
/// timeouts it is, and where to send the event.
struct Note {
    index: usize,
    tx: mpsc::UnboundedSender<IdleEvent>,
}

/// A bound `wl_output` and its gamma control, when the manager exists.
struct Output {
    /// Its registry global name — how `global_remove` and the control's udata
    /// find it again.
    name: u32,
    /// The bound object, kept for a gamma manager that may arrive later.
    wl: WlOutput,
    control: Option<ZwlrGammaControlV1>,
    /// From the control's `gamma_size` event; 0 until it arrives.
    ramp: usize,
}

impl Drop for Output {
    fn drop(&mut self) {
        if let Some(control) = &self.control {
            control.destroy();
        }
    }
}

/// One subscription's idle notifications on the seat.
struct Watch {
    timeouts: Vec<u32>,
    notifications: Vec<ExtIdleNotificationV1>,
    tx: mpsc::UnboundedSender<IdleEvent>,
}

impl Watch {
    /// (Re)create the notification objects — initial registration, and a new
    /// seat replacing a removed one.
    fn register(&mut self, notifier: &ExtIdleNotifierV1, seat: &WlSeat, qh: &QueueHandle<State>) {
        for notification in self.notifications.drain(..) {
            notification.destroy();
        }
        for (index, &ms) in self.timeouts.iter().enumerate() {
            let note = Note {
                index,
                tx: self.tx.clone(),
            };
            self.notifications
                .push(notifier.get_idle_notification(ms, seat, qh, note));
        }
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        for notification in &self.notifications {
            notification.destroy();
        }
    }
}

/// The event queue's state: bound globals and the work they carry.
struct State {
    conn: Connection,
    rx: Receiver<Cmd>,
    seat: Option<WlSeat>,
    seat_name: Option<u32>,
    notifier: Option<ExtIdleNotifierV1>,
    notifier_name: Option<u32>,
    gamma: Option<ZwlrGammaControlManagerV1>,
    gamma_name: Option<u32>,
    /// Current color temperature — the gamma table folds it with `dim`.
    kelvin: u16,
    /// Brightness scale for the idle dim stage (1.0 = undimmed).
    dim: f32,
    outputs: Vec<Output>,
    watches: Vec<Watch>,
    /// Ramp files whose fds must stay open until the next `flush` — the
    /// backend stores raw fds in its outgoing buffer until then.
    pending: Vec<File>,
}

impl State {
    fn command(&mut self, cmd: Cmd, qh: &QueueHandle<Self>) {
        match cmd {
            Cmd::Temperature(kelvin) => {
                self.kelvin = kelvin;
                self.apply_gamma();
            }
            Cmd::Dim(factor) => {
                self.dim = factor.clamp(0.0, 1.0);
                self.apply_gamma();
            }
            Cmd::Watch { timeouts, tx } => {
                let mut watch = Watch {
                    timeouts,
                    notifications: Vec::new(),
                    tx,
                };
                if let (Some(notifier), Some(seat)) = (&self.notifier, &self.seat) {
                    watch.register(notifier, seat, qh);
                } else {
                    tracing::warn!("wayl: idle watch but no seat or notifier");
                }
                self.watches.push(watch);
            }
        }
    }

    /// Subscriptions that went away take their notifications with them.
    fn reap(&mut self) {
        self.watches.retain(|watch| !watch.tx.is_closed());
    }

    /// Write the table for the current kelvin × dim to every controlled output.
    fn apply_gamma(&mut self) {
        for output in &mut self.outputs {
            let Some(control) = &output.control else {
                continue;
            };
            if output.ramp == 0 {
                continue; // no gamma_size yet (or a failed control)
            }
            let table = gamma_table(self.kelvin, output.ramp, self.dim);
            let Some(file) = ramp_file(&table) else {
                tracing::warn!("wayl: cannot write a gamma table");
                continue;
            };
            control.set_gamma(file.as_fd());
            self.pending.push(file);
        }
    }

    fn output(&mut self, name: u32) -> Option<&mut Output> {
        self.outputs.iter_mut().find(|o| o.name == name)
    }

    /// A registry global appeared; every object we use exists at version 1, so
    /// binding the advertised version would only add events we ignore anyway.
    fn global(
        &mut self,
        name: u32,
        interface: &str,
        registry: &WlRegistry,
        qh: &QueueHandle<Self>,
    ) {
        if interface == WlSeat::interface().name {
            // Idle notifications ride the first seat only; niri has exactly one
            // and a second would need notifications per seat per watch.
            if self.seat.is_some() {
                return;
            }
            let seat: WlSeat = registry.bind(name, 1, qh, ());
            if let Some(notifier) = &self.notifier {
                for watch in &mut self.watches {
                    watch.register(notifier, &seat, qh);
                }
            }
            self.seat = Some(seat);
            self.seat_name = Some(name);
        } else if interface == WlOutput::interface().name {
            let output: WlOutput = registry.bind(name, 1, qh, ());
            let control = self
                .gamma
                .as_ref()
                .map(|manager| manager.get_gamma_control(&output, qh, name));
            self.outputs.push(Output {
                name,
                wl: output,
                control,
                ramp: 0,
            });
        } else if interface == ExtIdleNotifierV1::interface().name {
            self.notifier = Some(registry.bind(name, 1, qh, ()));
            self.notifier_name = Some(name);
        } else if interface == ZwlrGammaControlManagerV1::interface().name {
            let manager: ZwlrGammaControlManagerV1 = registry.bind(name, 1, qh, ());
            // The manager may arrive after the outputs did.
            for output in &mut self.outputs {
                let control = manager.get_gamma_control(&output.wl, qh, output.name);
                output.control = Some(control);
            }
            self.gamma = Some(manager);
            self.gamma_name = Some(name);
        }
    }

    /// Existing objects survive a global's removal; only new binds stop.
    fn global_remove(&mut self, name: u32) {
        if self.seat_name == Some(name) {
            tracing::warn!("wayl: the seat went away");
            self.seat = None;
            self.seat_name = None;
        } else if self.notifier_name == Some(name) {
            tracing::warn!("wayl: idle notifier went away");
            self.notifier = None;
            self.notifier_name = None;
        } else if self.gamma_name == Some(name) {
            tracing::warn!("wayl: gamma control manager went away");
            self.gamma = None;
            self.gamma_name = None;
        } else {
            self.outputs.retain(|output| output.name != name);
        }
    }
}

impl Dispatch<WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        registry: &WlRegistry,
        event: wl_registry::Event,
        (): &(),
        _: &Connection,
        qhandle: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } => {
                tracing::debug!(name, %interface, version, "wayl: global");
                state.global(name, &interface, registry, qhandle);
            }
            wl_registry::Event::GlobalRemove { name } => state.global_remove(name),
            _ => {}
        }
    }
}

/// Seats only identify which seat a notification watches; its capabilities are
/// unused.
impl Dispatch<WlSeat, ()> for State {
    fn event(
        _: &mut Self,
        _: &WlSeat,
        _: wl_seat::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

/// Outputs are only handles for the gamma manager; their events are unused.
impl Dispatch<WlOutput, ()> for State {
    fn event(
        _: &mut Self,
        _: &WlOutput,
        _: wl_output::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

/// `done` replies exist only to wake the worker for queued commands; see
/// `Shared::send`.
impl Dispatch<WlCallback, ()> for State {
    fn event(
        _: &mut Self,
        _: &WlCallback,
        _: wl_callback::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

/// The notifier and manager interfaces have no events.
impl Dispatch<ExtIdleNotifierV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &ExtIdleNotifierV1,
        _: ext_idle_notifier_v1::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrGammaControlManagerV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &ZwlrGammaControlManagerV1,
        _: zwlr_gamma_control_manager_v1::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ExtIdleNotificationV1, Note> for State {
    fn event(
        _: &mut Self,
        _: &ExtIdleNotificationV1,
        event: ext_idle_notification_v1::Event,
        note: &Note,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let event = match event {
            ext_idle_notification_v1::Event::Idled => IdleEvent::Idle(note.index),
            ext_idle_notification_v1::Event::Resumed => IdleEvent::Resumed(note.index),
            _ => return,
        };
        let _ = note.tx.unbounded_send(event);
    }
}

impl Dispatch<ZwlrGammaControlV1, u32> for State {
    fn event(
        state: &mut Self,
        control: &ZwlrGammaControlV1,
        event: zwlr_gamma_control_v1::Event,
        name: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_gamma_control_v1::Event::GammaSize { size } => {
                if let Some(output) = state.output(*name) {
                    let size = usize::try_from(size).unwrap_or(usize::MAX);
                    // A real ramp is a few hundred entries; anything absurdly
                    // large is not worth an allocation.
                    output.ramp = if size <= 65_536 {
                        size
                    } else {
                        tracing::warn!(name, size, "wayl: unreasonable gamma size");
                        0
                    };
                }
            }
            zwlr_gamma_control_v1::Event::Failed => {
                // The output can no longer be gamma-controlled; the protocol
                // asks us to destroy the object.
                tracing::warn!(name, "wayl: gamma control failed");
                control.destroy();
                if let Some(output) = state.output(*name) {
                    output.control = None;
                }
            }
            _ => {}
        }
    }
}

/// `Duration` to protocol milliseconds, saturating at `u32::MAX` (~49 days).
fn millis(timeout: Duration) -> u32 {
    u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX)
}

/// RGB multipliers for a blackbody color temperature, normalized so the
/// strongest channel is 1 — Tanner Helland's approximation, the one wlsunset
/// and gammastep also use.
fn whitepoint(kelvin: u16) -> [f64; 3] {
    let t = (f64::from(kelvin) / 100.0).clamp(10.0, 400.0);
    let r = if t <= 66.0 {
        1.0
    } else {
        1.292936 * (t - 60.0).powf(-0.1332047)
    };
    let g = if t <= 66.0 {
        0.3900816 * t.ln() - 0.6318414
    } else {
        1.1298909 * (t - 60.0).powf(-0.0755148)
    };
    let b = if t >= 66.0 {
        1.0
    } else if t <= 19.0 {
        0.0
    } else {
        0.5432068 * (t - 10.0).ln() - 1.196254
    };
    [r.clamp(0.0, 1.0), g.clamp(0.0, 1.0), b.clamp(0.0, 1.0)]
}

/// The gamma table `set_gamma` wants: `size` u16 entries per channel, red then
/// green then blue — a linear ramp scaled by the temperature's channel factor
/// and the idle dim factor.
fn gamma_table(kelvin: u16, size: usize, dim: f32) -> Vec<u16> {
    // chunks_mut panics on a zero chunk; the caller already skips ramp 0.
    if size == 0 {
        return Vec::new();
    }
    let mut table = vec![0; 3 * size];
    for (factor, ramp) in whitepoint(kelvin).into_iter().zip(table.chunks_mut(size)) {
        for (i, entry) in ramp.iter_mut().enumerate() {
            let level = if size > 1 {
                i as f64 / (size - 1) as f64
            } else {
                1.0
            };
            *entry = (level * factor * f64::from(dim) * 65535.0)
                .round()
                .min(65535.0) as u16;
        }
    }
    table
}

/// The table lives in an unlinked file under the runtime dir: the compositor
/// reads it through the fd, and the file must stay open until the request is
/// flushed (the backend keeps the raw fd in its outgoing buffer until then).
fn ramp_file(table: &[u16]) -> Option<File> {
    static SEQ: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map_or_else(std::env::temp_dir, PathBuf::from);
    for _ in 0..8 {
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let path = dir.join(format!("triton-gamma-{}-{seq}", std::process::id()));
        match OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                let mut bytes = Vec::with_capacity(table.len() * 2);
                for value in table {
                    bytes.extend_from_slice(&value.to_ne_bytes());
                }
                if let Err(e) = file.write_all(&bytes).and_then(|()| file.rewind()) {
                    tracing::warn!(%e, "wayl: cannot write a gamma table");
                    let _ = std::fs::remove_file(&path);
                    return None;
                }
                let _ = std::fs::remove_file(&path);
                return Some(file);
            }
            Err(e) if e.kind() == ErrorKind::AlreadyExists => {}
            Err(e) => {
                tracing::warn!(%e, path = %path.display(), "wayl: cannot create a gamma table");
                return None;
            }
        }
    }
    tracing::warn!("wayl: cannot create a gamma table");
    None
}

/// Connects, binds the globals the shell needs and spawns the worker. A
/// registry roundtrip here is what lets `start()` answer about the gamma
/// manager before the thread exists.
fn connect() -> Option<Shared> {
    let conn = match Connection::connect_to_env() {
        Ok(conn) => conn,
        Err(e) => {
            tracing::warn!(%e, "wayl: no wayland display");
            return None;
        }
    };
    let mut queue: EventQueue<State> = conn.new_event_queue();
    let qh = queue.handle();
    let display = conn.display();
    let _ = display.get_registry(&qh, ());
    let (tx, rx) = std::sync::mpsc::channel();
    let mut state = State {
        conn: conn.clone(),
        rx,
        seat: None,
        seat_name: None,
        notifier: None,
        notifier_name: None,
        gamma: None,
        gamma_name: None,
        kelvin: 6500,
        dim: 1.0,
        outputs: Vec::new(),
        watches: Vec::new(),
        pending: Vec::new(),
    };
    if let Err(e) = queue.roundtrip(&mut state) {
        tracing::warn!(%e, "wayl: registry roundtrip failed");
        return None;
    }
    if state.notifier.is_none() {
        tracing::warn!("wayl: no ext_idle_notifier_v1; idle timeouts disabled");
    }
    if state.seat.is_none() {
        tracing::warn!("wayl: no wl_seat; idle timeouts disabled");
    }
    let gamma = state.gamma.is_some();
    if !gamma {
        tracing::warn!("wayl: no zwlr_gamma_control_manager_v1; night light disabled");
    }
    if let Err(e) = thread::Builder::new()
        .name("wayl".into())
        .spawn(move || run(queue, state))
    {
        tracing::error!(%e, "wayl: cannot start the worker thread");
        return None;
    }
    Some(Shared {
        tx,
        conn,
        display,
        qh,
        gamma,
    })
}

/// The worker loop: drain queued commands, flush (ramp fds stay open until
/// this write lands), then block on the wayland socket. `Shared::send`'s sync
/// callback is what unblocks the dispatch when a command arrives.
fn run(mut queue: EventQueue<State>, mut state: State) {
    let qhandle = queue.handle();
    loop {
        while let Ok(cmd) = state.rx.try_recv() {
            state.command(cmd, &qhandle);
        }
        state.reap();
        if let Err(e) = state.conn.flush() {
            tracing::error!(%e, "wayl: connection lost");
            return;
        }
        state.pending.clear();
        if let Err(e) = queue.blocking_dispatch(&mut state) {
            tracing::error!(%e, "wayl: dispatch failed");
            return;
        }
    }
}
