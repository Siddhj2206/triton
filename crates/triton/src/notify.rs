//! The `org.freedesktop.Notifications` server — Triton *is* the notification
//! daemon (roadmap §2). One subscription, one thread: it connects to the
//! session bus, serves `/org/freedesktop/Notifications` and claims the
//! well-known name. `Notify` calls become [`Event`]s on the subscription
//! stream; the app owns the [`Store`] and decides when a toast expires, so
//! the D-Bus side stays a thin translator and the store tests need no bus.
//! [`closed`] and [`action_invoked`] push the two spec signals back through a
//! channel to that same connection.
//!
//! Skipped for now: `image-data` (raw pixels — the UI has no image slot) and
//! the sound hints; seeing them is debug-logged once.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Once, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use iced_futures::Subscription;
use iced_futures::futures::channel::mpsc;
use iced_futures::futures::{Stream, StreamExt};
use zbus::object_server::SignalEmitter;
use zbus::zvariant::Variant as Value;
use zbus::{Connection, interface};

const NAME: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";
/// `expire_timeout <= 0` is "the server decides" — 6 s on the board's
/// Standard card.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(6);
/// "Never expires" as a `Duration`: `Instant::checked_add` overflows on it,
/// and [`Notification::alive_at`] reads the overflow as still alive.
const NEVER: Duration = Duration::MAX;

/// `hints["urgency"]` — a byte: 0 low, 1 normal, 2 critical.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Urgency {
    Low,
    Normal,
    Critical,
}

/// One row of the store — what a `Notify` call carried, plus arrival time.
#[derive(Debug, Clone)]
pub struct Notification {
    pub id: u32,
    pub app_name: String,
    /// `app_icon`, else the `image-path`/`image_path` hint.
    #[expect(dead_code, reason = "icon images land with the card icon support")]
    pub icon: Option<String>,
    pub summary: String,
    pub body: String,
    /// `(action key, label)` pairs from the flat alternating actions array.
    #[expect(dead_code, reason = "action buttons land with the card actions")]
    pub actions: Vec<(String, String)>,
    pub urgency: Urgency,
    /// Toast lifetime. `expire_timeout <= 0` becomes `DEFAULT_TIMEOUT`;
    /// critical notifications get `Duration::MAX` — they stay until dismissed.
    pub timeout: Duration,
    /// When the `Notify` call arrived; `toasts` and the "12 min" labels use it.
    pub arrived: Instant,
    /// Toast hover-pause: `timeout` has been rewritten to the lifetime that
    /// remained when the pointer entered; `alive_at` stays true until
    /// [`Store::resume_toast`] re-arms it from a fresh `arrived`.
    pub paused: bool,
}

impl Notification {
    fn alive_at(&self, now: Instant) -> bool {
        // Overflow means Duration::MAX — "never expires".
        self.paused
            || self
                .arrived
                .checked_add(self.timeout)
                .is_none_or(|end| end > now)
    }
}

/// Notifications the center renders, keyed by id. `BTreeMap` keeps arrival
/// order for free: ids only ever increase.
#[derive(Debug, Default)]
pub struct Store {
    items: BTreeMap<u32, Notification>,
}

impl Store {
    /// Insert, or replace when `n.id` is already known (`replaces_id`).
    pub fn upsert(&mut self, n: Notification) {
        self.items.insert(n.id, n);
    }

    /// Remove `id`; whether it existed.
    pub fn close(&mut self, id: u32) -> bool {
        self.items.remove(&id).is_some()
    }

    /// The center's per-group Clear: drop everything from `app`.
    pub fn clear_app(&mut self, app: &str) {
        self.items.retain(|_, n| n.app_name != app);
    }

    #[allow(clippy::missing_const_for_fn)]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    #[allow(clippy::missing_const_for_fn)]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Groups by `app_name`, the group holding the newest notification first,
    /// items inside a group newest first.
    pub fn groups(&self) -> Vec<Group> {
        let mut groups: Vec<Group> = Vec::new();
        for n in self.items.values().rev() {
            match groups.iter_mut().find(|g| g.app == n.app_name) {
                Some(group) => group.items.push(n.clone()),
                None => groups.push(Group {
                    app: n.app_name.clone(),
                    items: vec![n.clone()],
                }),
            }
        }
        groups
    }

    /// Toast-worthy right now: alive at `now`, and either not DND or critical.
    /// Newest first, like [`Store::groups`].
    pub fn toasts(&self, now: Instant, dnd: bool) -> Vec<&Notification> {
        self.items
            .values()
            .rev()
            .filter(|n| n.alive_at(now) && (!dnd || n.urgency == Urgency::Critical))
            .collect()
    }

    /// Hover-pause a toast: freeze the lifetime it had left at `now` into
    /// `timeout` and mark it paused, so the expiry pass leaves it up.
    pub fn pause_toast(&mut self, id: u32, now: Instant) {
        let Some(n) = self.items.get_mut(&id) else {
            return;
        };
        if n.paused {
            return;
        }
        if let Some(remaining) = n
            .arrived
            .checked_add(n.timeout)
            .map(|end| end.saturating_duration_since(now))
        {
            n.timeout = remaining;
            n.paused = true;
        }
    }

    /// The pointer left: re-arm the frozen lifetime from a fresh `arrived`.
    /// Returns the remaining lifetime so the caller can re-schedule expiry.
    pub fn resume_toast(&mut self, id: u32, now: Instant) -> Option<Duration> {
        let n = self.items.get_mut(&id)?;
        if !n.paused {
            return None;
        }
        n.paused = false;
        n.arrived = now;
        Some(n.timeout)
    }

    /// The lifetime a toast has left at `now` — for re-arming expiry after a
    /// pause that outlived the original timer.
    pub fn remaining(&self, id: u32, now: Instant) -> Option<Duration> {
        let n = self.items.get(&id)?;
        n.arrived
            .checked_add(n.timeout)
            .map(|end| end.saturating_duration_since(now))
    }
}

/// The center's per-app stack.
#[derive(Debug, Clone)]
pub struct Group {
    pub app: String,
    /// Newest first.
    pub items: Vec<Notification>,
}

/// What the server forwarded; the app applies it to its [`Store`].
#[derive(Debug, Clone)]
pub enum Event {
    /// A notification was stored or replaced (`replaces_id` reuses the id).
    Arrived(Notification),
    /// `CloseNotification` was called, or a toast expired — the app decides.
    Closed(u32),
}

/// The served half: translates `Notify` calls into [`Event`]s. Holds no store
/// — the app owns state — so all it needs is the id counter and the stream.
struct Notifications {
    next_id: AtomicU32,
    events: mpsc::UnboundedSender<Event>,
}

#[interface(name = "org.freedesktop.Notifications")]
impl Notifications {
    /// `Notify`: the one that does the work. `replaces_id == 0` allocates.
    #[allow(
        clippy::too_many_arguments,
        clippy::needless_pass_by_value,
        reason = "the Notify signature is fixed by the D-Bus spec"
    )]
    fn notify(
        &self,
        app_name: &str,
        replaces_id: u32,
        app_icon: &str,
        summary: &str,
        body: &str,
        actions: Vec<String>,
        hints: HashMap<String, Value<'_>>,
        expire_timeout: i32,
    ) -> u32 {
        log_ignored_hints(&hints);
        let urgency = urgency(&hints);
        let timeout = if urgency == Urgency::Critical {
            NEVER
        } else if expire_timeout > 0 {
            Duration::from_millis(expire_timeout as u64)
        } else {
            DEFAULT_TIMEOUT
        };
        let id = if replaces_id == 0 {
            self.next_id.fetch_add(1, Ordering::Relaxed)
        } else {
            replaces_id
        };
        let _ = self.events.unbounded_send(Event::Arrived(Notification {
            id,
            app_name: app_name.to_owned(),
            icon: icon(app_icon, &hints),
            summary: summary.to_owned(),
            body: body.to_owned(),
            actions: pair_actions(&actions),
            urgency,
            timeout,
            arrived: Instant::now(),
            paused: false,
        }));
        id
    }

    /// `CloseNotification`: forward the request; the app emits
    /// `NotificationClosed(id, 3)` once it actually dropped the entry.
    fn close_notification(&self, id: u32) {
        let _ = self.events.unbounded_send(Event::Closed(id));
    }

    /// `GetCapabilities`.
    #[allow(clippy::unused_self, reason = "interface methods need a receiver")]
    fn get_capabilities(&self) -> Vec<String> {
        ["actions", "body", "persistence"].map(String::from).into()
    }

    /// `GetServerInformation` -> (name, vendor, version, `spec_version`).
    #[allow(clippy::unused_self, reason = "interface methods need a receiver")]
    fn get_server_information(&self) -> (String, String, String, String) {
        ("triton".into(), "Triton".into(), "0.1".into(), "1.2".into())
    }

    /// `NotificationClosed(id, reason)`: 1 expired, 2 dismissed, 3 closed by call.
    #[zbus(signal)]
    async fn notification_closed(
        signal_emitter: &SignalEmitter<'_>,
        id: u32,
        reason: u32,
    ) -> zbus::Result<()>;

    /// `ActionInvoked(id, action_key)`.
    #[zbus(signal)]
    async fn action_invoked(
        signal_emitter: &SignalEmitter<'_>,
        id: u32,
        action_key: &str,
    ) -> zbus::Result<()>;
}

/// `urgency` is a byte per spec; a few clients send it wider — clamp those
/// to the byte range, anything missing or stranger is Normal.
fn urgency(hints: &HashMap<String, Value<'_>>) -> Urgency {
    let level: u8 = match hints.get("urgency") {
        Some(Value::U8(level)) => *level,
        Some(Value::I16(level)) => (*level).try_into().unwrap_or(1),
        Some(Value::U16(level)) => (*level).try_into().unwrap_or(1),
        Some(Value::I32(level)) => (*level).try_into().unwrap_or(1),
        Some(Value::U32(level)) => (*level).try_into().unwrap_or(1),
        Some(Value::I64(level)) => (*level).try_into().unwrap_or(1),
        Some(Value::U64(level)) => (*level).try_into().unwrap_or(1),
        _ => 1,
    };
    match level {
        0 => Urgency::Low,
        2 => Urgency::Critical,
        _ => Urgency::Normal,
    }
}

/// `app_icon` first; when empty, the `image-path` hint (some clients write
/// `image_path`). Raw `image-data` is not read — the UI has no image slot.
fn icon(app_icon: &str, hints: &HashMap<String, Value<'_>>) -> Option<String> {
    if !app_icon.is_empty() {
        return Some(app_icon.to_owned());
    }
    match hints.get("image-path").or_else(|| hints.get("image_path")) {
        Some(Value::Str(path)) if !path.as_str().is_empty() => Some(path.as_str().to_owned()),
        _ => None,
    }
}

/// The actions array is a flat `key, label, key, label` list; an odd trailing
/// string is dropped.
fn pair_actions(actions: &[String]) -> Vec<(String, String)> {
    actions
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| (pair[0].clone(), pair[1].clone()))
        .collect()
}

/// Hints we do not implement yet: raw pixels and sounds. Debug-logged once —
/// chat apps put `image-data` on every message.
fn log_ignored_hints(hints: &HashMap<String, Value<'_>>) {
    const IGNORED: [&str; 5] = [
        "image-data",
        "image_data",
        "sound-file",
        "sound-name",
        "suppress-sound",
    ];
    static ONCE: Once = Once::new();
    if !hints.keys().any(|k| IGNORED.contains(&k.as_str())) {
        return;
    }
    ONCE.call_once(|| tracing::debug!("notify: ignoring image-data and sound hints"));
}

/// Work for the serving connection: which spec signal to emit. The UI thread
/// only ever sends; the server thread awaits and emits.
enum Emit {
    Closed(u32, u32),
    Action(u32, String),
}

/// Installed by the serving thread once it runs; `closed` and
/// `action_invoked` quietly no-op before that or after it dies — a missing
/// bus is a warn-and-done, never a panic.
static EMIT: OnceLock<mpsc::UnboundedSender<Emit>> = OnceLock::new();

/// Emit `NotificationClosed(id, reason)` to clients — 1 expired, 2 dismissed,
/// 3 closed by `CloseNotification`. Called from the UI thread.
pub fn closed(id: u32, reason: u32) {
    if let Some(tx) = EMIT.get() {
        let _ = tx.unbounded_send(Emit::Closed(id, reason));
    }
}

/// Emit `ActionInvoked(id, key)` — the app fires this for the click/default
/// action.
pub fn action_invoked(id: u32, key: &str) {
    if let Some(tx) = EMIT.get() {
        let _ = tx.unbounded_send(Emit::Action(id, key.to_owned()));
    }
}

/// The serving thread: session bus, object, name, then the emit loop.
/// Anything missing is a warn and an early return — no retry, no panic.
fn serve(events: mpsc::UnboundedSender<Event>) {
    zbus::block_on(async {
        let conn = match Connection::session().await {
            Ok(conn) => conn,
            Err(e) => {
                tracing::warn!(%e, "notify: no session bus");
                return;
            }
        };
        let iface = Notifications {
            next_id: AtomicU32::new(1),
            events,
        };
        // The object before the name, so no `Notify` call lands unhandled.
        if let Err(e) = conn.object_server().at(PATH, iface).await {
            tracing::warn!(%e, "notify: cannot serve /org/freedesktop/Notifications");
            return;
        }
        if let Err(e) = conn.request_name(NAME).await {
            // Most likely another daemon (mako, dunst, a second triton).
            tracing::warn!(%e, "notify: org.freedesktop.Notifications is taken");
            return;
        }
        let signals = match conn
            .object_server()
            .interface::<_, Notifications>(PATH)
            .await
        {
            Ok(signals) => signals,
            Err(e) => {
                tracing::warn!(%e, "notify: lost the served object");
                return;
            }
        };
        let (tx, mut rx) = mpsc::unbounded();
        if EMIT.set(tx).is_err() {
            tracing::warn!("notify: a server is already running; this one cannot emit signals");
        }
        while let Some(emit) = rx.next().await {
            let sent = match emit {
                Emit::Closed(id, reason) => signals.notification_closed(id, reason).await,
                Emit::Action(id, key) => signals.action_invoked(id, &key).await,
            };
            if let Err(e) = sent {
                tracing::debug!(%e, "notify: signal emission failed");
            }
        }
    });
}

fn stream() -> impl Stream<Item = Event> {
    let (tx, rx) = mpsc::unbounded();
    thread::Builder::new()
        .name("notify".into())
        .spawn(move || serve(tx))
        .map_err(|e| tracing::error!(%e, "notify: cannot start thread"))
        .ok();
    rx
}

/// One subscription for the whole server: a thread that connects to the
/// session bus, claims `org.freedesktop.Notifications`, serves
/// `/org/freedesktop/Notifications` and forwards what it stores. A missing
/// bus logs a warn and ends — never panics, never retries.
pub fn server() -> Subscription<Event> {
    Subscription::run_with((), |()| stream())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(
        id: u32,
        app: &str,
        urgency: Urgency,
        arrived: Instant,
        timeout: Duration,
    ) -> Notification {
        Notification {
            id,
            app_name: app.to_owned(),
            icon: None,
            summary: format!("summary {id}"),
            body: String::new(),
            actions: Vec::new(),
            urgency,
            timeout,
            arrived,
            paused: false,
        }
    }

    fn ids(toasts: &[&Notification]) -> Vec<u32> {
        toasts.iter().map(|n| n.id).collect()
    }

    #[test]
    fn upsert_inserts_then_replaces() {
        let now = Instant::now();
        let mut store = Store::default();
        store.upsert(note(7, "chat", Urgency::Normal, now, DEFAULT_TIMEOUT));
        store.upsert(note(7, "chat", Urgency::Critical, now, NEVER));
        assert_eq!(store.len(), 1);
        assert_eq!(store.groups()[0].items[0].urgency, Urgency::Critical);
    }

    #[test]
    fn close_reports_whether_it_existed() {
        let now = Instant::now();
        let mut store = Store::default();
        assert!(!store.close(4));
        store.upsert(note(4, "chat", Urgency::Normal, now, DEFAULT_TIMEOUT));
        assert!(store.close(4));
        assert_eq!(store.len(), 0);
    }

    #[test]
    fn clear_app_drops_only_its_group() {
        let now = Instant::now();
        let mut store = Store::default();
        store.upsert(note(1, "chat", Urgency::Normal, now, DEFAULT_TIMEOUT));
        store.upsert(note(2, "cal", Urgency::Normal, now, DEFAULT_TIMEOUT));
        store.upsert(note(3, "chat", Urgency::Normal, now, DEFAULT_TIMEOUT));
        store.clear_app("chat");
        let groups = store.groups();
        assert_eq!(store.len(), 1);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].app, "cal");
    }

    #[test]
    fn groups_are_newest_group_then_newest_item() {
        let now = Instant::now();
        let mut store = Store::default();
        store.upsert(note(1, "chat", Urgency::Normal, now, DEFAULT_TIMEOUT));
        store.upsert(note(2, "cal", Urgency::Normal, now, DEFAULT_TIMEOUT));
        store.upsert(note(3, "chat", Urgency::Normal, now, DEFAULT_TIMEOUT));
        let groups = store.groups();
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].app, "chat");
        assert_eq!(
            groups[0].items.iter().map(|n| n.id).collect::<Vec<_>>(),
            [3, 1]
        );
        assert_eq!(groups[1].app, "cal");
    }

    #[test]
    fn toasts_drop_expired_and_dnd_hides_non_critical() {
        let now = Instant::now();
        let mut store = Store::default();
        store.upsert(note(
            1,
            "chat",
            Urgency::Normal,
            now,
            Duration::from_secs(6),
        ));
        // Arrived 10 s ago with a 6 s timeout: expired.
        store.upsert(note(
            2,
            "chat",
            Urgency::Normal,
            now.checked_sub(Duration::from_secs(10)).unwrap(),
            Duration::from_secs(6),
        ));
        // Critical with `NEVER` outlives everything and ignores DND.
        store.upsert(note(
            3,
            "cal",
            Urgency::Critical,
            now.checked_sub(Duration::from_secs(600)).unwrap(),
            NEVER,
        ));
        assert_eq!(ids(&store.toasts(now, false)), [3, 1]);
        assert_eq!(ids(&store.toasts(now, true)), [3]);
    }
}
