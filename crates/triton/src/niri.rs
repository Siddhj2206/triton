//! niri over IPC: a thread follows the event stream and hands the app immutable
//! snapshots; actions go out on their own short-lived connections.
//!
//! niri sends no output events, so outputs are re-queried whenever the workspaces
//! change (a hotplugged output gets workspaces, an unplugged one loses them).
//!
//! niri starts the shell, so a closed stream means niri is going away: the thread
//! logs it and ends rather than retrying.

use std::collections::HashMap;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use iced_futures::Subscription;
use iced_futures::futures::Stream;
use iced_futures::futures::channel::mpsc;
use niri_ipc::socket::Socket;
use niri_ipc::state::{EventStreamState, EventStreamStatePart};
use niri_ipc::{Action, Event, Reply, Request, Response, Window, Workspace, WorkspaceReferenceArg};

/// What the shell knows about niri at one moment.
#[derive(Debug, Default)]
pub struct Snapshot {
    /// Enabled outputs, by connector name, sorted.
    pub outputs: Vec<String>,
    /// Workspaces in output, then index order.
    pub workspaces: Vec<Workspace>,
    pub windows: HashMap<u64, Window>,
}

impl Snapshot {
    pub fn workspaces_on<'a>(&'a self, output: &'a str) -> impl Iterator<Item = &'a Workspace> {
        self.workspaces
            .iter()
            .filter(move |w| w.output.as_deref() == Some(output))
    }

    /// The window the active workspace on `output` shows focused.
    pub fn active_window(&self, output: &str) -> Option<&Window> {
        let ws = self.workspaces_on(output).find(|w| w.is_active)?;
        self.windows.get(&ws.active_window_id?)
    }

    /// The output holding the globally focused workspace, if niri reports one.
    pub fn focus_output(&self) -> Option<String> {
        self.workspaces
            .iter()
            .find(|w| w.is_focused)
            .and_then(|w| w.output.clone())
    }
}

fn outputs(socket: &mut Socket) -> Vec<String> {
    match socket.send(Request::Outputs) {
        Ok(Ok(Response::Outputs(map))) => {
            let mut names: Vec<_> = map
                .into_values()
                .filter(|o| o.logical.is_some())
                .map(|o| o.name)
                .collect();
            names.sort();
            names
        }
        other => {
            tracing::warn!(?other, "niri: outputs query failed");
            Vec::new()
        }
    }
}

fn snapshot(state: &EventStreamState, outputs: Vec<String>) -> Arc<Snapshot> {
    let mut workspaces: Vec<_> = state.workspaces.workspaces.values().cloned().collect();
    workspaces.sort_by(|a, b| (&a.output, a.idx).cmp(&(&b.output, b.idx)));
    Arc::new(Snapshot {
        outputs,
        workspaces,
        windows: state.windows.windows.clone(),
    })
}

/// Follows the event stream until it ends or the app stops listening.
fn follow(tx: &mpsc::UnboundedSender<Arc<Snapshot>>) {
    let (mut query, events) = match (Socket::connect(), Socket::connect()) {
        (Ok(q), Ok(e)) => (q, e),
        (Err(e), _) | (_, Err(e)) => {
            tracing::error!(%e, "niri: cannot connect");
            return;
        }
    };
    let mut events = events;
    match events.send(Request::EventStream) {
        Ok(Ok(Response::Handled)) => {}
        other => {
            tracing::error!(?other, "niri: event stream refused");
            return;
        }
    }
    let mut next = events.read_events();
    let mut state = EventStreamState::default();
    let mut outs = outputs(&mut query);
    loop {
        let event = match next() {
            Ok(e) => e,
            Err(e) => {
                tracing::error!(%e, "niri: event stream ended");
                return;
            }
        };
        let relevant = matches!(
            event,
            Event::WorkspacesChanged { .. }
                | Event::WorkspaceActivated { .. }
                | Event::WorkspaceActiveWindowChanged { .. }
                | Event::WindowsChanged { .. }
                | Event::WindowOpenedOrChanged { .. }
                | Event::WindowClosed { .. }
                | Event::WindowFocusChanged { .. }
        );
        if matches!(event, Event::WorkspacesChanged { .. }) {
            outs = outputs(&mut query);
        }
        state.apply(event);
        if relevant && tx.unbounded_send(snapshot(&state, outs.clone())).is_err() {
            return;
        }
    }
}

fn stream() -> impl Stream<Item = Arc<Snapshot>> {
    let (tx, rx) = mpsc::unbounded();
    thread::Builder::new()
        .name("niri-events".into())
        .spawn(move || follow(&tx))
        .map_err(|e| tracing::error!(%e, "niri: cannot start thread"))
        .ok();
    rx
}

/// Snapshots of niri, a new one after each change the shell shows.
pub fn snapshots() -> Subscription<Arc<Snapshot>> {
    Subscription::run(stream)
}

/// Asks niri to do something, without waiting for the reply on the UI thread.
pub fn act(action: Action) {
    thread::spawn(move || {
        let reply: std::io::Result<Reply> =
            Socket::connect().and_then(|mut s| s.send(Request::Action(action)));
        match reply {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => tracing::warn!(%e, "niri refused an action"),
            Err(e) => tracing::warn!(%e, "niri: cannot send action"),
        }
    });
}

pub fn focus_workspace(id: u64) {
    act(Action::FocusWorkspace {
        reference: WorkspaceReferenceArg::Id(id),
    });
}

/// Focus a window (the launcher's window results).
pub fn focus_window(id: u64) {
    act(Action::FocusWindow { id });
}

/// The "Log out" power action.
pub fn quit() {
    act(Action::Quit {
        skip_confirmation: true,
    });
}

/// A message at every minute boundary, for the clock.
pub fn minutes() -> Subscription<()> {
    fn stream() -> impl Stream<Item = ()> {
        let (tx, rx) = mpsc::unbounded();
        thread::Builder::new()
            .name("clock".into())
            .spawn(move || {
                loop {
                    let now = chrono::Local::now();
                    let into = u64::from(now.timestamp_subsec_millis())
                        + 1000 * u64::from(chrono::Timelike::second(&now));
                    #[expect(
                        clippy::disallowed_methods,
                        reason = "a wait for a known deadline (the next minute), not polling"
                    )]
                    thread::sleep(Duration::from_millis(60_000 - into.min(59_999)));
                    if tx.unbounded_send(()).is_err() {
                        break;
                    }
                }
            })
            .map_err(|e| tracing::error!(%e, "clock: cannot start thread"))
            .ok();
        rx
    }
    Subscription::run(stream)
}
