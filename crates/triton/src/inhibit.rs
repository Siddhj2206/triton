//! `org.freedesktop.ScreenSaver` on the session bus — the idle inhibitor
//! (roadmap §4: "dim, then lock, then suspend, respecting inhibitors").
//! Video players and browsers call `Inhibit`/`UnInhibit`; while any cookie is
//! held the app skips the dim/lock/suspend stages. One subscription keeps the
//! serving thread alive and reports the live inhibitor count to the app, like
//! `notify::server` does for notifications.

use std::collections::HashSet;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use iced_futures::Subscription;
use iced_futures::futures::Stream;
use iced_futures::futures::channel::mpsc;
use zbus::{Connection, interface};

const NAME: &str = "org.freedesktop.ScreenSaver";
const PATH: &str = "/org/freedesktop/ScreenSaver";

/// The served half: a set of live inhibit cookies. `Inhibit`/`UnInhibit` push
/// the new count onto the stream so the app always sees the current total.
struct ScreenSaver {
    next: AtomicU32,
    held: Arc<Mutex<HashSet<u32>>>,
    counts: mpsc::UnboundedSender<u32>,
}

impl ScreenSaver {
    fn count(&self) -> u32 {
        let n = self.held.lock().map_or(0, |held| held.len()) as u32;
        let _ = self.counts.unbounded_send(n);
        n
    }
}

#[interface(name = "org.freedesktop.ScreenSaver")]
impl ScreenSaver {
    /// `Inhibit(application_name, reason)` -> cookie.
    fn inhibit(&self, application_name: &str, reason_for_inhibit: &str) -> u32 {
        let cookie = self.next.fetch_add(1, Ordering::Relaxed);
        tracing::info!(cookie, %application_name, %reason_for_inhibit, "idle inhibited");
        if let Ok(mut held) = self.held.lock() {
            held.insert(cookie);
        }
        self.count();
        cookie
    }

    /// `UnInhibit(cookie)` — the spec spells it without the second capital.
    fn un_inhibit(&self, cookie: u32) {
        if let Ok(mut held) = self.held.lock() {
            held.remove(&cookie);
        }
        self.count();
    }

    /// `GetActive` — whether the screensaver would be blanked right now.
    /// Triton has no separate "active" state; inhibitors block the stages, so
    /// this reports whether any stage was allowed to run (always false).
    #[allow(clippy::unused_self, reason = "interface methods need a receiver")]
    fn get_active(&self) -> bool {
        false
    }

    /// `GetSessionIdleTime` — seconds since last activity. The compositor
    /// owns idle timing; we cannot know it here.
    #[allow(clippy::unused_self, reason = "interface methods need a receiver")]
    fn get_session_idle_time(&self) -> u32 {
        0
    }

    /// `SimulateUserActivity` — wakes the session; nothing to poke on our end.
    #[allow(clippy::unused_self, reason = "interface methods need a receiver")]
    fn simulate_user_activity(&self) {}
}

/// The serving thread: session bus, object, name, then hold the count stream
/// open (it ends when the app drops the subscription).
fn serve(counts: mpsc::UnboundedSender<u32>) {
    zbus::block_on(async {
        let conn = match Connection::session().await {
            Ok(conn) => conn,
            Err(e) => {
                tracing::warn!(%e, "inhibit: no session bus");
                return;
            }
        };
        let iface = ScreenSaver {
            next: AtomicU32::new(1),
            held: Arc::new(Mutex::new(HashSet::new())),
            counts,
        };
        if let Err(e) = conn.object_server().at(PATH, iface).await {
            tracing::warn!(%e, "inhibit: cannot serve /org/freedesktop/ScreenSaver");
            return;
        }
        if let Err(e) = conn.request_name(NAME).await {
            tracing::warn!(%e, "inhibit: org.freedesktop.ScreenSaver is taken");
            return;
        }
        // Keep the task alive; the interface's `counts` sender drives events.
        std::future::pending::<()>().await;
    });
}

fn stream() -> impl Stream<Item = u32> {
    let (tx, rx) = mpsc::unbounded();
    thread::Builder::new()
        .name("inhibit".into())
        .spawn(move || serve(tx))
        .map_err(|e| tracing::error!(%e, "inhibit: cannot start thread"))
        .ok();
    rx
}

/// The live inhibitor count, reported on every Inhibit/UnInhibit — one
/// subscription for the whole service.
pub fn server() -> Subscription<u32> {
    Subscription::run_with((), |()| stream())
}
