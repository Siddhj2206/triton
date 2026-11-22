//! Live niri IPC client for `--real-services`: JSON event stream → typed
//! [`ShellEvent`]s on a dedicated background thread, plus a non-blocking
//! action-dispatch sink for compositor actions (e.g. workspace-pill clicks).
//!
//! The `niri-ipc` crate is GPL-3.0, so the small wire-protocol subset triton
//! needs is implemented here directly with `serde` (see `wire` below). Shapes
//! were verified against niri 26.04's `niri_ipc` sources.
//!
//! Protocol: newline-delimited JSON over `$NIRI_SOCKET` (a Unix socket). Each
//! request line gets one reply line (`{"Ok": <Response>}` / `{"Err": "..."}`).
//! After a successful `EventStream` request the socket switches to streaming
//! mode and niri sends one [`wire::Event`] per line, starting with a full
//! snapshot (`WorkspacesChanged`, `WindowsChanged`, ...). Actions are sent on
//! short-lived separate connections so they never interleave with the stream.
//!
//! Note: niri's IPC does not expose a window `is_fullscreen` flag or a
//! focused-output-changed event (checked niri-ipc 26.4.0 and `main`). Fullscreen
//! is approximated by "the focused window's tile covers its output's whole
//! logical size" (fullscreen tiles ignore the work-area struts, so a real
//! fullscreen window always covers the output; a maximized column only does
//! when nothing reserves an exclusive zone). Focused output is derived from
//! the focused workspace / focused window's workspace.

use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, SyncSender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use thiserror::Error;
use tracing::{debug, error, trace, warn};
use triton_engine::EventSender;

use crate::state::{FocusedWindow, ShellEvent, WorkspaceInfo};

/// Environment variable holding the niri IPC socket path.
pub const NIRI_SOCKET_ENV: &str = "NIRI_SOCKET";

/// Backlog bound for queued compositor actions (`SyncSender` never blocks the
/// shell; a saturated queue drops actions with a warning instead).
const ACTION_QUEUE_CAP: usize = 64;

/// Upper bound on a single niri IPC round-trip so a wedged compositor can
/// never hang startup, action dispatch, or daemon shutdown.
const IO_TIMEOUT: Duration = Duration::from_secs(5);

/// Maximum accepted size of a single niri reply/event line (4 MiB is far above
/// any realistic `WindowsChanged` snapshot).
const MAX_LINE_BYTES: u64 = 4 * 1024 * 1024;

/// Output name → logical size in logical pixels, used for the fullscreen
/// approximation (see [`tile_covers_output`]).
type OutputSizes = HashMap<String, (f64, f64)>;

/// Errors returned when establishing or using the niri IPC connection.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum NiriIpcError {
    /// `$NIRI_SOCKET` is unset or empty — not running inside niri.
    #[error("{NIRI_SOCKET_ENV} is not set; is triton running inside a niri session?")]
    MissingSocketEnv,
    /// Socket or stream I/O failure.
    #[error("niri IPC I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// niri returned `{"Err": ...}` for a request.
    #[error("niri IPC request was rejected: {0}")]
    RequestRejected(String),
    /// Malformed or unexpected wire data.
    #[error("niri IPC protocol error: {0}")]
    Protocol(String),
}

/// Compositor actions triton dispatches back to niri, e.g. when the user
/// clicks a workspace pill in the Bar. Dispatch is non-blocking: actions are
/// queued for the dedicated niri action worker.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum NiriAction {
    /// Focus a workspace by its stable niri workspace id.
    FocusWorkspace(u64),
    /// Focus the workspace below the currently active one.
    FocusWorkspaceDown,
    /// Focus the workspace above the currently active one.
    FocusWorkspaceUp,
    /// Focus the previously focused workspace.
    FocusWorkspacePrevious,
    /// Focus a window by its niri window id.
    FocusWindow(u64),
}

impl NiriAction {
    fn to_wire(&self) -> wire::Action {
        match self {
            Self::FocusWorkspace(id) => wire::Action::FocusWorkspace {
                reference: wire::WorkspaceRef::Id(*id),
            },
            Self::FocusWorkspaceDown => wire::Action::FocusWorkspaceDown {},
            Self::FocusWorkspaceUp => wire::Action::FocusWorkspaceUp {},
            Self::FocusWorkspacePrevious => wire::Action::FocusWorkspacePrevious {},
            Self::FocusWindow(id) => wire::Action::FocusWindow { id: *id },
        }
    }
}

/// Internal queue payload for the action worker (the `Stop` sentinel stays
/// private so the public `NiriAction` vocabulary mirrors niri only).
#[derive(Debug)]
enum ActionMsg {
    Run(NiriAction),
    Stop,
}

/// Cloneable, non-blocking handle used by `TritonShell` to dispatch
/// [`NiriAction`]s off the UI thread.
#[derive(Debug, Clone)]
pub struct NiriActionSender {
    tx: SyncSender<ActionMsg>,
}

impl NiriActionSender {
    /// Queue `action` for dispatch to niri. Never blocks the caller; a full or
    /// dead queue drops the action with a structured log line.
    pub fn send(&self, action: NiriAction) {
        if let Err(mpsc::TrySendError::Full(ActionMsg::Run(action))) =
            self.tx.try_send(ActionMsg::Run(action))
        {
            warn!(action = ?action, "niri action queue is full; dropping action");
        }
        // Full(Stop) and Disconnected need no log: the worker is shutting down.
    }
}

/// Live niri IPC connection: a background `triton-niri-events` thread feeding
/// [`ShellEvent`]s into the shell, and a `triton-niri-actions` worker draining
/// queued [`NiriAction`]s. Dropping shuts both threads down cleanly.
#[derive(Debug)]
pub struct NiriIpc {
    socket_path: PathBuf,
    action_sink: NiriActionSender,
    shutdown: Arc<AtomicBool>,
    /// Clone of the event-stream connection used to unblock `read_line` on Drop.
    event_socket: UnixStream,
    event_thread: Option<JoinHandle<()>>,
    action_thread: Option<JoinHandle<()>>,
}

impl NiriIpc {
    /// Connect to the niri IPC socket named by `$NIRI_SOCKET`.
    ///
    /// Only call under `BackendMode::RealServices`; default fake dev mode must
    /// never touch the host's niri socket.
    pub fn connect(events: EventSender<ShellEvent>) -> Result<Self, NiriIpcError> {
        let path = std::env::var_os(NIRI_SOCKET_ENV)
            .filter(|p| !p.is_empty())
            .ok_or(NiriIpcError::MissingSocketEnv)?;
        Self::connect_to(PathBuf::from(path), events)
    }

    /// Connect to a niri IPC socket at `socket_path`, perform the
    /// `Outputs` + `EventStream` handshake, and start the background threads.
    pub fn connect_to(
        socket_path: impl Into<PathBuf>,
        events: EventSender<ShellEvent>,
    ) -> Result<Self, NiriIpcError> {
        let socket_path = socket_path.into();
        let (stream, outputs) = handshake(&socket_path)?;
        let event_socket = stream.get_ref().try_clone().map_err(NiriIpcError::Io)?;

        let shutdown = Arc::new(AtomicBool::new(false));
        let (action_tx, action_rx) = mpsc::sync_channel::<ActionMsg>(ACTION_QUEUE_CAP);

        let event_shutdown = Arc::clone(&shutdown);
        let event_path = socket_path.clone();
        let event_thread = thread::Builder::new()
            .name("triton-niri-events".to_owned())
            .spawn(move || {
                event_loop(stream, &event_path, outputs, &events, &event_shutdown);
            })
            .map_err(NiriIpcError::Io)?;

        let action_path = socket_path.clone();
        let action_shutdown = Arc::clone(&shutdown);
        let action_thread = thread::Builder::new()
            .name("triton-niri-actions".to_owned())
            .spawn(move || action_loop(&action_path, &action_rx, &action_shutdown))
            .map_err(NiriIpcError::Io)?;

        Ok(Self {
            socket_path,
            action_sink: NiriActionSender { tx: action_tx },
            shutdown,
            event_socket,
            event_thread: Some(event_thread),
            action_thread: Some(action_thread),
        })
    }

    /// Path of the connected niri IPC socket.
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Cloneable non-blocking handle for dispatching [`NiriAction`]s. Inject
    /// into `TritonShell` via `set_niri_action_sink`.
    pub fn action_sink(&self) -> NiriActionSender {
        self.action_sink.clone()
    }
}

impl Drop for NiriIpc {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        // Unblock the event thread's `read_line` without polling.
        let _ = self.event_socket.shutdown(std::net::Shutdown::Both);
        // Wake the action worker so it observes the shutdown flag.
        let _ = self.action_sink.tx.try_send(ActionMsg::Stop);
        if let Some(handle) = self.event_thread.take() {
            let _ = handle.join();
        }
        if let Some(handle) = self.action_thread.take() {
            let _ = handle.join();
        }
        debug!(socket = %self.socket_path.display(), "niri IPC connection shut down");
    }
}

/// Perform the startup handshake on a fresh connection: fetch the output
/// inventory (for fullscreen inference), then switch the socket into
/// `EventStream` mode.
fn handshake(socket_path: &Path) -> Result<(BufReader<UnixStream>, OutputSizes), NiriIpcError> {
    let stream = connect_stream(socket_path)?;
    let mut stream = BufReader::new(stream);

    let mut outputs = HashMap::new();
    match send_request(&mut stream, &wire::Request::Outputs)? {
        Ok(value) => {
            if let Some(map) = value.get("Outputs")
                && let Err(err) = parse_outputs(map, &mut outputs)
            {
                warn!(error = %err, "could not parse niri outputs; fullscreen detection disabled");
            }
        }
        Err(msg) => {
            warn!(error = %msg, "niri rejected the Outputs request; fullscreen detection disabled");
        }
    }

    match send_request(&mut stream, &wire::Request::EventStream)? {
        Ok(_) => Ok((stream, outputs)),
        Err(msg) => Err(NiriIpcError::RequestRejected(format!("EventStream: {msg}"))),
    }
}

/// Connect with bounded I/O timeouts so a wedged compositor cannot hang the
/// daemon or its shutdown.
fn connect_stream(socket_path: &Path) -> Result<UnixStream, std::io::Error> {
    let stream = UnixStream::connect(socket_path)?;
    stream.set_read_timeout(Some(IO_TIMEOUT))?;
    stream.set_write_timeout(Some(IO_TIMEOUT))?;
    Ok(stream)
}

/// niri reply envelope: `{"Ok": <Response>}` or `{"Err": "..."}`.
type WireReply = Result<serde_json::Value, String>;

/// Write one request line and read back its single-line reply.
fn send_request(
    stream: &mut BufReader<UnixStream>,
    request: &wire::Request,
) -> Result<WireReply, NiriIpcError> {
    let mut line =
        serde_json::to_string(request).map_err(|e| NiriIpcError::Protocol(e.to_string()))?;
    line.push('\n');
    stream.get_mut().write_all(line.as_bytes())?;
    stream.get_mut().flush()?;

    line.clear();
    let read = (&mut *stream).take(MAX_LINE_BYTES).read_line(&mut line)?;
    if read == 0 {
        return Err(NiriIpcError::Protocol(
            "connection closed before reply".to_owned(),
        ));
    }
    serde_json::from_str(&line).map_err(|e| NiriIpcError::Protocol(e.to_string()))
}

fn parse_outputs(map: &serde_json::Value, outputs: &mut OutputSizes) -> Result<(), NiriIpcError> {
    let parsed: HashMap<String, wire::Output> =
        serde_json::from_value(map.clone()).map_err(|e| NiriIpcError::Protocol(e.to_string()))?;
    for (name, output) in parsed {
        if let Some(logical) = output.logical {
            outputs.insert(name, (f64::from(logical.width), f64::from(logical.height)));
        }
    }
    Ok(())
}

/// Refresh the output logical-size inventory over a short-lived connection.
fn query_outputs(socket_path: &Path) -> Result<OutputSizes, NiriIpcError> {
    let stream = connect_stream(socket_path)?;
    let mut stream = BufReader::new(stream);
    let mut outputs = HashMap::new();
    if let Ok(value) = send_request(&mut stream, &wire::Request::Outputs)?
        && let Some(map) = value.get("Outputs")
    {
        parse_outputs(map, &mut outputs)?;
    }
    Ok(outputs)
}

/// Blocking read loop translating niri [`wire::Event`]s into [`ShellEvent`]s.
/// Runs on the dedicated `triton-niri-events` thread; exits on stream EOF,
/// read error, or shutdown.
fn event_loop(
    mut stream: BufReader<UnixStream>,
    socket_path: &Path,
    outputs: OutputSizes,
    events: &EventSender<ShellEvent>,
    shutdown: &AtomicBool,
) {
    let mut state = StreamState::new(socket_path, outputs);
    let mut line = String::new();
    loop {
        line.clear();
        let read = (&mut stream).take(MAX_LINE_BYTES).read_line(&mut line);
        match read {
            Ok(0) => {
                debug!("niri event stream closed by compositor");
                break;
            }
            Ok(_) => match serde_json::from_str::<wire::Event>(&line) {
                Ok(event) => {
                    if !state.apply(event, events) {
                        break;
                    }
                }
                Err(err) => {
                    debug!(error = %err, line = %line.trim(), "skipping unrecognized niri event");
                }
            },
            Err(err) => {
                if shutdown.load(Ordering::Acquire) {
                    break;
                }
                error!(error = %err, "niri event stream read failed; stopping niri IPC thread");
                break;
            }
        }
    }
}

/// Blocking worker draining queued [`NiriAction`]s onto short-lived niri
/// connections so the UI thread never waits on the compositor.
fn action_loop(socket_path: &Path, rx: &mpsc::Receiver<ActionMsg>, shutdown: &AtomicBool) {
    loop {
        if shutdown.load(Ordering::Acquire) {
            break;
        }
        match rx.recv() {
            Ok(ActionMsg::Run(action)) => {
                if let Err(err) = dispatch_action(socket_path, &action) {
                    warn!(action = ?action, error = %err, "niri action dispatch failed");
                }
            }
            Ok(ActionMsg::Stop) | Err(_) => break,
        }
    }
}

fn dispatch_action(socket_path: &Path, action: &NiriAction) -> Result<(), NiriIpcError> {
    let stream = connect_stream(socket_path)?;
    let mut stream = BufReader::new(stream);
    let reply = send_request(&mut stream, &wire::Request::Action(action.to_wire()))?;
    reply.map_err(NiriIpcError::RequestRejected)?;
    debug!(action = ?action, "dispatched niri action");
    Ok(())
}

/// Per-connection niri state translated into `TritonShell`'s domain events,
/// with latest-wins dedupe so the shell only wakes for real changes.
struct StreamState {
    socket_path: PathBuf,
    workspaces: BTreeMap<u64, wire::Workspace>,
    windows: BTreeMap<u64, wire::Window>,
    /// Output name → logical size, for fullscreen inference.
    outputs: OutputSizes,
    last_workspaces: Vec<WorkspaceInfo>,
    last_focused: Option<FocusedWindow>,
    last_focused_output: Option<String>,
}

impl StreamState {
    fn new(socket_path: &Path, outputs: OutputSizes) -> Self {
        Self {
            socket_path: socket_path.to_path_buf(),
            workspaces: BTreeMap::new(),
            windows: BTreeMap::new(),
            outputs,
            last_workspaces: Vec::new(),
            last_focused: None,
            last_focused_output: None,
        }
    }

    /// Apply one niri event, emitting deduplicated `ShellEvent`s. Returns
    /// `false` when the shell's event channel is gone (daemon exiting).
    fn apply(&mut self, event: wire::Event, events: &EventSender<ShellEvent>) -> bool {
        match event {
            wire::Event::WorkspacesChanged { workspaces } => {
                self.workspaces = workspaces.into_iter().map(|w| (w.id, w)).collect();
                self.refresh_outputs_if_needed();
                self.emit_focused_output(events)
                    && self.emit_workspaces(events)
                    && self.emit_focused(events)
            }
            wire::Event::WorkspaceActivated { id, focused } => {
                let output = self.workspaces.get(&id).and_then(|w| w.output.clone());
                for ws in self.workspaces.values_mut() {
                    if ws.output == output {
                        ws.is_active = ws.id == id;
                    }
                    if focused {
                        ws.is_focused = ws.id == id;
                    }
                }
                if output.is_none() {
                    debug!(id, "WorkspaceActivated for unknown workspace");
                }
                self.emit_focused_output(events) && self.emit_workspaces(events)
            }
            wire::Event::WorkspaceUrgencyChanged { id, urgent } => {
                if let Some(ws) = self.workspaces.get_mut(&id) {
                    ws.is_urgent = urgent;
                }
                self.emit_workspaces(events)
            }
            wire::Event::WorkspaceActiveWindowChanged {
                workspace_id,
                active_window_id,
            } => {
                if let Some(ws) = self.workspaces.get_mut(&workspace_id) {
                    ws.active_window_id = active_window_id;
                }
                self.emit_workspaces(events)
            }
            wire::Event::WindowsChanged { windows } => {
                self.windows = windows.into_iter().map(|w| (w.id, w)).collect();
                self.refresh_outputs_if_needed();
                self.emit_focused(events) && self.emit_focused_output(events)
            }
            wire::Event::WindowOpenedOrChanged { window } => {
                if window.is_focused {
                    for w in self.windows.values_mut() {
                        w.is_focused = false;
                    }
                }
                self.windows.insert(window.id, window);
                self.refresh_outputs_if_needed();
                self.emit_focused(events) && self.emit_focused_output(events)
            }
            wire::Event::WindowClosed { id } => {
                self.windows.remove(&id);
                self.emit_focused(events)
            }
            wire::Event::WindowFocusChanged { id } => {
                for w in self.windows.values_mut() {
                    w.is_focused = Some(w.id) == id;
                }
                self.emit_focused(events) && self.emit_focused_output(events)
            }
            wire::Event::WindowUrgencyChanged { id, urgent } => {
                if let Some(w) = self.windows.get_mut(&id) {
                    w.is_urgent = urgent;
                }
                true
            }
            wire::Event::WindowLayoutsChanged { changes } => {
                for (id, layout) in changes {
                    if let Some(w) = self.windows.get_mut(&id) {
                        w.layout = layout;
                    }
                }
                // Dedupe keeps this quiet; only a real fullscreen flip emits.
                self.emit_focused(events)
            }
            wire::Event::ConfigLoaded { failed } => {
                if failed {
                    warn!("niri configuration reload failed");
                }
                true
            }
            wire::Event::KeyboardLayoutSwitched { idx } => {
                trace!(idx, "niri keyboard layout switched");
                true
            }
            wire::Event::KeyboardLayoutsChanged { .. }
            | wire::Event::OverviewOpenedOrClosed { .. }
            | wire::Event::WindowFocusTimestampChanged { .. }
            | wire::Event::ScreenshotCaptured { .. }
            | wire::Event::CastsChanged { .. }
            | wire::Event::CastStartedOrChanged { .. }
            | wire::Event::CastStopped { .. } => {
                trace!("ignoring niri event");
                true
            }
        }
    }

    /// Lazily refresh the output inventory when a workspace references an
    /// output we have no logical size for (monitor hotplug).
    fn refresh_outputs_if_needed(&mut self) {
        let missing = self
            .workspaces
            .values()
            .filter_map(|w| w.output.as_deref())
            .any(|o| !self.outputs.contains_key(o));
        if !missing {
            return;
        }
        match query_outputs(&self.socket_path) {
            Ok(fresh) => {
                if !fresh.is_empty() {
                    self.outputs = fresh;
                }
            }
            Err(err) => debug!(error = %err, "niri outputs refresh failed"),
        }
    }

    fn emit_workspaces(&mut self, events: &EventSender<ShellEvent>) -> bool {
        let mut list: Vec<WorkspaceInfo> = self
            .workspaces
            .values()
            .map(|ws| WorkspaceInfo {
                id: ws.id,
                idx: ws.idx,
                name: ws.name.clone(),
                output: ws.output.clone().unwrap_or_default(),
                is_active: ws.is_active,
                is_focused: ws.is_focused,
                is_urgent: ws.is_urgent,
                active_window_id: ws.active_window_id,
            })
            .collect();
        list.sort_by(|a, b| {
            a.output
                .cmp(&b.output)
                .then(a.idx.cmp(&b.idx))
                .then(a.id.cmp(&b.id))
        });
        if list == self.last_workspaces {
            return true;
        }
        self.last_workspaces.clone_from(&list);
        Self::push(events, ShellEvent::WorkspacesUpdated(list))
    }

    fn emit_focused(&mut self, events: &EventSender<ShellEvent>) -> bool {
        let focused = self.derived_focused_window();
        if focused == self.last_focused {
            return true;
        }
        self.last_focused.clone_from(&focused);
        Self::push(events, ShellEvent::FocusedWindowChanged(focused))
    }

    fn emit_focused_output(&mut self, events: &EventSender<ShellEvent>) -> bool {
        // Sticky: keep the last known focused output when nothing is focused
        // (e.g. a layer-shell surface holds keyboard focus).
        let Some(derived) = self.derived_focused_output() else {
            return true;
        };
        if self.last_focused_output.as_deref() == Some(derived.as_str()) {
            return true;
        }
        self.last_focused_output = Some(derived.clone());
        Self::push(events, ShellEvent::FocusedOutputChanged(Some(derived)))
    }

    fn derived_focused_output(&self) -> Option<String> {
        if let Some(ws) = self.workspaces.values().find(|w| w.is_focused) {
            return ws.output.clone();
        }
        let win = self.windows.values().find(|w| w.is_focused)?;
        let ws = win.workspace_id.and_then(|id| self.workspaces.get(&id))?;
        ws.output.clone()
    }

    fn derived_focused_window(&self) -> Option<FocusedWindow> {
        let win = self.windows.values().find(|w| w.is_focused)?;
        let output = win
            .workspace_id
            .and_then(|id| self.workspaces.get(&id))
            .and_then(|ws| ws.output.clone());
        let is_fullscreen = output
            .as_deref()
            .and_then(|name| self.outputs.get(name))
            .is_some_and(|(w, h)| tile_covers_output(&win.layout, *w, *h));
        Some(FocusedWindow {
            id: win.id,
            title: win.title.clone().unwrap_or_default(),
            app_id: win.app_id.clone(),
            is_fullscreen,
            output,
        })
    }

    fn push(events: &EventSender<ShellEvent>, event: ShellEvent) -> bool {
        match events.send(event) {
            Ok(()) => true,
            Err(err) => {
                debug!(error = %err, "shell event channel closed; stopping niri IPC thread");
                false
            }
        }
    }
}

/// Fullscreen approximation: niri IPC exposes no fullscreen flag, so we treat a
/// focused window as fullscreen when its tile covers the whole logical output
/// (fullscreen tiles ignore work-area struts). A maximized single column also
/// qualifies when no exclusive zones are reserved — visually equivalent.
fn tile_covers_output(layout: &wire::WindowLayout, out_w: f64, out_h: f64) -> bool {
    const EPSILON: f64 = 1.0;
    layout.tile_size.0 + EPSILON >= out_w && layout.tile_size.1 + EPSILON >= out_h
}

/// niri IPC wire-protocol subset (niri-ipc is GPL-3.0, so the few types we
/// need are declared here; shapes match niri 26.04's serde serialization).
mod wire {
    use serde::{Deserialize, Serialize};

    /// Request sent to niri, one JSON line per request.
    #[derive(Debug, Serialize)]
    pub enum Request {
        Outputs,
        EventStream,
        Action(Action),
    }

    /// Compositor action subset triton dispatches.
    #[derive(Debug, Serialize)]
    #[expect(
        clippy::enum_variant_names,
        reason = "variant spellings must match niri_ipc::Action exactly"
    )]
    pub enum Action {
        FocusWorkspaceDown {},
        FocusWorkspaceUp {},
        FocusWorkspace { reference: WorkspaceRef },
        FocusWorkspacePrevious {},
        FocusWindow { id: u64 },
    }

    /// `niri_ipc::WorkspaceReferenceArg` (only `Id` is needed today).
    #[derive(Debug, Serialize)]
    pub enum WorkspaceRef {
        Id(u64),
    }

    /// `niri_ipc::Output` (fields triton does not need are ignored).
    #[derive(Debug, Deserialize)]
    pub struct Output {
        #[serde(default)]
        pub logical: Option<LogicalOutput>,
    }

    /// `niri_ipc::LogicalOutput` (geometry fields only).
    #[derive(Debug, Deserialize)]
    pub struct LogicalOutput {
        pub width: u32,
        pub height: u32,
    }

    /// `niri_ipc::Workspace`.
    #[derive(Debug, Clone, Deserialize)]
    pub struct Workspace {
        pub id: u64,
        pub idx: u8,
        #[serde(default)]
        pub name: Option<String>,
        #[serde(default)]
        pub output: Option<String>,
        #[serde(default)]
        pub is_urgent: bool,
        #[serde(default)]
        pub is_active: bool,
        #[serde(default)]
        pub is_focused: bool,
        #[serde(default)]
        pub active_window_id: Option<u64>,
    }

    /// `niri_ipc::Window` (`pid` and `focus_timestamp` ignored).
    #[derive(Debug, Clone, Deserialize)]
    pub struct Window {
        pub id: u64,
        #[serde(default)]
        pub title: Option<String>,
        #[serde(default)]
        pub app_id: Option<String>,
        #[serde(default)]
        pub workspace_id: Option<u64>,
        #[serde(default)]
        pub is_focused: bool,
        #[serde(default)]
        pub is_urgent: bool,
        pub layout: WindowLayout,
    }

    /// `niri_ipc::WindowLayout` — triton only needs `tile_size` for the
    /// fullscreen approximation; niri's other layout fields are skipped by
    /// serde during deserialization.
    #[derive(Debug, Clone, Copy, Deserialize)]
    pub struct WindowLayout {
        pub tile_size: (f64, f64),
    }

    /// `niri_ipc::Event` — every variant niri 26.04 emits, so known events
    /// never hit the unknown-line debug path. Variants triton ignores carry
    /// `serde_json::Value` payloads.
    #[derive(Debug, Deserialize)]
    pub enum Event {
        WorkspacesChanged {
            workspaces: Vec<Workspace>,
        },
        WorkspaceUrgencyChanged {
            id: u64,
            urgent: bool,
        },
        WorkspaceActivated {
            id: u64,
            focused: bool,
        },
        WorkspaceActiveWindowChanged {
            workspace_id: u64,
            active_window_id: Option<u64>,
        },
        WindowsChanged {
            windows: Vec<Window>,
        },
        WindowOpenedOrChanged {
            window: Window,
        },
        WindowClosed {
            id: u64,
        },
        WindowFocusChanged {
            id: Option<u64>,
        },
        WindowFocusTimestampChanged {
            #[expect(dead_code, reason = "not consumed by triton yet")]
            id: u64,
            #[expect(dead_code, reason = "not consumed by triton yet")]
            focus_timestamp: Option<serde_json::Value>,
        },
        WindowUrgencyChanged {
            id: u64,
            urgent: bool,
        },
        WindowLayoutsChanged {
            changes: Vec<(u64, WindowLayout)>,
        },
        KeyboardLayoutsChanged {
            // OSD wiring for keyboard layouts lands with the OSD ticket (#17).
            #[expect(dead_code, reason = "not consumed by triton yet")]
            keyboard_layouts: serde_json::Value,
        },
        KeyboardLayoutSwitched {
            idx: u8,
        },
        OverviewOpenedOrClosed {
            #[expect(dead_code, reason = "not consumed by triton yet")]
            is_open: bool,
        },
        ConfigLoaded {
            failed: bool,
        },
        ScreenshotCaptured {
            #[expect(dead_code, reason = "not consumed by triton yet")]
            path: Option<String>,
        },
        CastsChanged {
            #[expect(dead_code, reason = "not consumed by triton yet")]
            casts: Vec<serde_json::Value>,
        },
        CastStartedOrChanged {
            #[expect(dead_code, reason = "not consumed by triton yet")]
            cast: serde_json::Value,
        },
        CastStopped {
            #[expect(dead_code, reason = "not consumed by triton yet")]
            stream_id: u64,
        },
    }
}
