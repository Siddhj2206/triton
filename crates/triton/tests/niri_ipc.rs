//! Integration tests for the live niri IPC client (`--real-services`) driven
//! against a mock niri Unix socket in a temp directory. These tests never touch
//! the host `$NIRI_SOCKET` or a running niri session.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use triton::{
    BackendMode, NiriAction, NiriIpc, NiriIpcError, OutputKey, ShellEvent, ShellMessage, SurfaceId,
    SurfaceRole, TritonShell,
};
use triton_engine::{EventReceiver, ShellApp, event_channel};

static NEXT_DIR_ID: AtomicU64 = AtomicU64::new(1);

struct TempRuntimeDir {
    path: PathBuf,
}

impl TempRuntimeDir {
    fn new(label: &str) -> Self {
        let id = NEXT_DIR_ID.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("triton-niri-{label}-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&path).expect("create temp runtime dir");
        Self { path }
    }

    fn socket_path(&self) -> PathBuf {
        self.path.join("niri.sock")
    }
}

impl Drop for TempRuntimeDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Scripted behaviour for the mock niri socket server.
#[derive(Clone)]
struct MockConfig {
    /// JSON map returned as `Response::Outputs` (empty map when `None`).
    outputs: Option<String>,
    /// When set, `EventStream` requests get an `{"Err": ...}` reply.
    event_stream_error: Option<String>,
    /// Event lines streamed after a successful `EventStream` reply.
    events: Vec<String>,
    /// Close the event-stream connection right after streaming `events`.
    close_after_events: bool,
}

impl MockConfig {
    fn events(events: Vec<String>) -> Self {
        Self {
            outputs: Some(
                r#"{"eDP-1":{"name":"eDP-1","logical":{"x":0,"y":0,"width":1920,"height":1080}},"HDMI-A-1":{"name":"HDMI-A-1","logical":{"x":1920,"y":0,"width":2560,"height":1440}}}"#
                    .to_owned(),
            ),
            event_stream_error: None,
            events,
            close_after_events: false,
        }
    }
}

/// Mock niri IPC server recording every request line it receives.
struct MockNiri {
    dir: TempRuntimeDir,
    requests: mpsc::Receiver<String>,
    shutdown: Arc<AtomicBool>,
    listener: Option<JoinHandle<()>>,
    workers: Arc<Mutex<Vec<JoinHandle<()>>>>,
}

impl MockNiri {
    fn start(label: &str, config: MockConfig) -> Self {
        let dir = TempRuntimeDir::new(label);
        let listener = UnixListener::bind(dir.socket_path()).expect("bind mock niri socket");
        let (req_tx, req_rx) = mpsc::channel::<String>();
        let shutdown = Arc::new(AtomicBool::new(false));
        let workers = Arc::new(Mutex::new(Vec::<JoinHandle<()>>::new()));

        let worker_shutdown = Arc::clone(&shutdown);
        let worker_pool = Arc::clone(&workers);
        let handle = thread::Builder::new()
            .name(format!("mock-niri-{label}"))
            .spawn(move || {
                for incoming in listener.incoming() {
                    if worker_shutdown.load(Ordering::Acquire) {
                        break;
                    }
                    let Ok(stream) = incoming else { break };
                    let cfg = config.clone();
                    let reqs = req_tx.clone();
                    let join = thread::Builder::new()
                        .name("mock-niri-conn".to_owned())
                        .spawn(move || serve_conn(stream, &cfg, &reqs))
                        .expect("spawn mock conn worker");
                    worker_pool.lock().expect("worker pool").push(join);
                }
            })
            .expect("spawn mock niri listener");

        Self {
            dir,
            requests: req_rx,
            shutdown,
            listener: Some(handle),
            workers,
        }
    }

    fn socket_path(&self) -> PathBuf {
        self.dir.socket_path()
    }

    /// Next request line observed by the mock server (any connection).
    fn next_request(&self) -> String {
        self.requests
            .recv_timeout(Duration::from_secs(5))
            .expect("timed out waiting for a request on the mock niri socket")
    }
}

impl Drop for MockNiri {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        // Unblock the listener's accept() without polling.
        let _ = UnixStream::connect(self.socket_path());
        if let Some(handle) = self.listener.take() {
            let _ = handle.join();
        }
        if let Ok(mut workers) = self.workers.lock() {
            for handle in workers.drain(..) {
                let _ = handle.join();
            }
        }
    }
}

fn serve_conn(stream: UnixStream, cfg: &MockConfig, reqs: &mpsc::Sender<String>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let mut reader = BufReader::new(stream.try_clone().expect("clone conn stream"));
    let mut writer = stream;
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let request = line.trim().to_owned();
        let _ = reqs.send(request.clone());
        match request.as_str() {
            "\"Outputs\"" => {
                let map = cfg.outputs.clone().unwrap_or_else(|| "{}".to_owned());
                writeln!(writer, "{{\"Ok\":{{\"Outputs\":{map}}}}}").expect("write outputs reply");
            }
            "\"EventStream\"" => {
                if let Some(msg) = &cfg.event_stream_error {
                    let msg = serde_json::to_string(msg).expect("encode error string");
                    writeln!(writer, "{{\"Err\":{msg}}}").expect("write error reply");
                } else {
                    writer
                        .write_all(b"{\"Ok\":\"Handled\"}\n")
                        .expect("write event-stream reply");
                    for event in &cfg.events {
                        writeln!(writer, "{event}").expect("write event line");
                    }
                    writer.flush().expect("flush events");
                    if cfg.close_after_events {
                        return;
                    }
                }
            }
            _ => {
                writer
                    .write_all(b"{\"Ok\":\"Handled\"}\n")
                    .expect("write action reply");
            }
        }
    }
}

/// Drain shell events until `count` events matching `pred` arrive or the
/// deadline hits. Returns everything observed, in arrival order.
fn collect_until(
    rx: &EventReceiver<ShellEvent>,
    count: usize,
    within: Duration,
    pred: impl Fn(&ShellEvent) -> bool,
) -> Vec<ShellEvent> {
    let deadline = Instant::now() + within;
    let mut out = Vec::new();
    while out.iter().filter(|e| pred(e)).count() < count && Instant::now() < deadline {
        out.extend(rx.drain());
        thread::yield_now();
    }
    out
}

fn by_id(list: &[triton::WorkspaceInfo], id: u64) -> &triton::WorkspaceInfo {
    list.iter().find(|w| w.id == id).expect("workspace by id")
}

fn fixture_events() -> Vec<String> {
    std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/niri-events.jsonl"
    ))
    .expect("read niri-events.jsonl fixture")
    .lines()
    .map(str::to_owned)
    .collect()
}

#[test]
fn streams_workspaces_focused_window_and_focused_output() {
    let mock = MockNiri::start("basic-stream", MockConfig::events(fixture_events()));
    let (tx, rx) = event_channel::<ShellEvent>();
    let _ipc = NiriIpc::connect_to(mock.socket_path(), tx).expect("connect to mock niri");

    // Event stream delivers workspaces, focused window, then a focus change
    // that moves focus to the fullscreen-sized window on HDMI-A-1.
    let events = collect_until(&rx, 2, Duration::from_secs(5), |e| {
        matches!(e, ShellEvent::FocusedWindowChanged(Some(_)))
    });

    let workspaces = events
        .iter()
        .find_map(|e| match e {
            ShellEvent::WorkspacesUpdated(ws) => Some(ws.clone()),
            _ => None,
        })
        .expect("expected WorkspacesUpdated");
    assert_eq!(workspaces.len(), 3);
    let ws1 = workspaces.iter().find(|w| w.id == 1).expect("ws 1");
    assert!(ws1.is_active && ws1.is_focused);
    assert_eq!(ws1.output, "eDP-1");
    let ws2 = workspaces.iter().find(|w| w.id == 2).expect("ws 2");
    assert_eq!(ws2.name.as_deref(), Some("web"));
    assert!(ws2.is_urgent);
    let ws3 = workspaces.iter().find(|w| w.id == 3).expect("ws 3");
    assert_eq!(ws3.output, "HDMI-A-1");
    assert!(ws3.is_active && !ws3.is_focused);

    let focused: Vec<Option<triton::FocusedWindow>> = events
        .iter()
        .filter_map(|e| match e {
            ShellEvent::FocusedWindowChanged(w) => Some(w.clone()),
            _ => None,
        })
        .collect();
    let editor = focused
        .first()
        .and_then(Clone::clone)
        .expect("editor focus");
    assert_eq!(editor.title, "Editor");
    assert_eq!(editor.app_id.as_deref(), Some("dev.editor"));
    assert_eq!(editor.output.as_deref(), Some("eDP-1"));
    // tile_size 956x1029 does not cover the 1920x1080 output.
    assert!(!editor.is_fullscreen);
    let video = focused.last().and_then(Clone::clone).expect("video focus");
    assert_eq!(video.id, 20);
    assert_eq!(video.title, "Video");
    assert_eq!(video.output.as_deref(), Some("HDMI-A-1"));
    // tile_size covers the full 2560x1440 output → fullscreen.
    assert!(video.is_fullscreen);

    let outputs: Vec<Option<String>> = events
        .iter()
        .filter_map(|e| match e {
            ShellEvent::FocusedOutputChanged(o) => Some(o.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        outputs.first().and_then(Clone::clone).as_deref(),
        Some("eDP-1")
    );
    assert_eq!(
        outputs.last().and_then(Clone::clone).as_deref(),
        Some("HDMI-A-1")
    );

    assert_eq!(mock.next_request(), "\"Outputs\"");
    assert_eq!(mock.next_request(), "\"EventStream\"");
}

#[test]
fn workspace_activation_updates_active_flags_and_focused_output() {
    let events = vec![
        r#"{"WorkspacesChanged":{"workspaces":[{"id":1,"idx":1,"name":null,"output":"eDP-1","is_urgent":false,"is_active":true,"is_focused":true,"active_window_id":null},{"id":2,"idx":2,"name":null,"output":"eDP-1","is_urgent":false,"is_active":false,"is_focused":false,"active_window_id":null},{"id":3,"idx":1,"name":null,"output":"HDMI-A-1","is_urgent":false,"is_active":true,"is_focused":false,"active_window_id":null}]}}"#.to_owned(),
        r#"{"WorkspaceActivated":{"id":2,"focused":true}}"#.to_owned(),
        r#"{"WorkspaceActivated":{"id":3,"focused":true}}"#.to_owned(),
    ];
    let mock = MockNiri::start("ws-activation", MockConfig::events(events));
    let (tx, rx) = event_channel::<ShellEvent>();
    let _ipc = NiriIpc::connect_to(mock.socket_path(), tx).expect("connect to mock niri");

    let events = collect_until(&rx, 3, Duration::from_secs(5), |e| {
        matches!(e, ShellEvent::WorkspacesUpdated(_))
    });
    let updates: Vec<Vec<triton::WorkspaceInfo>> = events
        .iter()
        .filter_map(|e| match e {
            ShellEvent::WorkspacesUpdated(ws) => Some(ws.clone()),
            _ => None,
        })
        .collect();
    // Second activation: workspace 2 becomes active+focused on eDP-1, ws 1 deactivates.
    let after_first = &updates[1];
    assert!(by_id(after_first, 2).is_active && by_id(after_first, 2).is_focused);
    assert!(!by_id(after_first, 1).is_active && !by_id(after_first, 1).is_focused);
    // Third activation: workspace 3 on HDMI-A-1 takes focus; ws 2 stays active
    // on eDP-1 but loses focus.
    let after_second = &updates[2];
    assert!(by_id(after_second, 3).is_active && by_id(after_second, 3).is_focused);
    assert!(by_id(after_second, 2).is_active && !by_id(after_second, 2).is_focused);

    let outputs: Vec<Option<String>> = events
        .iter()
        .filter_map(|e| match e {
            ShellEvent::FocusedOutputChanged(o) => Some(o.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        outputs.last().and_then(Clone::clone).as_deref(),
        Some("HDMI-A-1")
    );
}

#[test]
fn fullscreen_state_tracks_window_layout_covering_the_output() {
    let events = vec![
        r#"{"WorkspacesChanged":{"workspaces":[{"id":1,"idx":1,"name":null,"output":"eDP-1","is_urgent":false,"is_active":true,"is_focused":true,"active_window_id":10}]}}"#.to_owned(),
        r#"{"WindowsChanged":{"windows":[{"id":10,"title":"Game","app_id":"game","pid":1,"workspace_id":1,"is_focused":true,"is_floating":false,"is_urgent":false,"layout":{"pos_in_scrolling_layout":[1,1],"tile_size":[956.0,1029.0],"window_size":[956,1029],"tile_pos_in_workspace_view":null,"window_offset_in_tile":[0.0,0.0]},"focus_timestamp":null}]}}"#.to_owned(),
        r#"{"WindowLayoutsChanged":{"changes":[[10,{"pos_in_scrolling_layout":[1,1],"tile_size":[1920.0,1080.0],"window_size":[1920,1080],"tile_pos_in_workspace_view":null,"window_offset_in_tile":[0.0,0.0]}]]}}"#.to_owned(),
        r#"{"WindowLayoutsChanged":{"changes":[[10,{"pos_in_scrolling_layout":[1,1],"tile_size":[956.0,1029.0],"window_size":[956,1029],"tile_pos_in_workspace_view":null,"window_offset_in_tile":[0.0,0.0]}]]}}"#.to_owned(),
    ];
    let mock = MockNiri::start("fullscreen", MockConfig::events(events));
    let (tx, rx) = event_channel::<ShellEvent>();
    let _ipc = NiriIpc::connect_to(mock.socket_path(), tx).expect("connect to mock niri");

    let events = collect_until(&rx, 3, Duration::from_secs(5), |e| {
        matches!(e, ShellEvent::FocusedWindowChanged(Some(_)))
    });
    let focused: Vec<triton::FocusedWindow> = events
        .iter()
        .filter_map(|e| match e {
            ShellEvent::FocusedWindowChanged(Some(w)) => Some(w.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        focused.len(),
        3,
        "each layout flip re-emits the focused window"
    );
    assert!(!focused[0].is_fullscreen);
    assert!(
        focused[1].is_fullscreen,
        "tile covering output == fullscreen"
    );
    assert!(!focused[2].is_fullscreen);
}

#[test]
fn workspace_urgency_and_active_window_updates_are_forwarded() {
    let events = vec![
        r#"{"WorkspacesChanged":{"workspaces":[{"id":1,"idx":1,"name":null,"output":"eDP-1","is_urgent":false,"is_active":true,"is_focused":true,"active_window_id":null}]}}"#.to_owned(),
        r#"{"WorkspaceUrgencyChanged":{"id":1,"urgent":true}}"#.to_owned(),
        r#"{"WorkspaceActiveWindowChanged":{"workspace_id":1,"active_window_id":42}}"#.to_owned(),
    ];
    let mock = MockNiri::start("ws-urgency", MockConfig::events(events));
    let (tx, rx) = event_channel::<ShellEvent>();
    let _ipc = NiriIpc::connect_to(mock.socket_path(), tx).expect("connect to mock niri");

    let events = collect_until(&rx, 3, Duration::from_secs(5), |e| {
        matches!(e, ShellEvent::WorkspacesUpdated(_))
    });
    let updates: Vec<Vec<triton::WorkspaceInfo>> = events
        .iter()
        .filter_map(|e| match e {
            ShellEvent::WorkspacesUpdated(ws) => Some(ws.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(updates.len(), 3);
    assert!(!updates[0][0].is_urgent);
    assert!(updates[1][0].is_urgent);
    assert_eq!(updates[2][0].active_window_id, Some(42));
}

#[test]
fn action_sink_sends_niri_action_requests_on_a_separate_connection() {
    let mock = MockNiri::start("actions", MockConfig::events(Vec::new()));
    let (tx, rx) = event_channel::<ShellEvent>();
    let ipc = NiriIpc::connect_to(mock.socket_path(), tx).expect("connect to mock niri");
    assert_eq!(mock.next_request(), "\"Outputs\"");
    assert_eq!(mock.next_request(), "\"EventStream\"");

    let sink = ipc.action_sink();
    sink.send(NiriAction::FocusWorkspace(7));
    sink.send(NiriAction::FocusWorkspaceDown);
    sink.send(NiriAction::FocusWorkspaceUp);
    sink.send(NiriAction::FocusWindow(99));

    assert_eq!(
        mock.next_request(),
        r#"{"Action":{"FocusWorkspace":{"reference":{"Id":7}}}}"#
    );
    assert_eq!(
        mock.next_request(),
        r#"{"Action":{"FocusWorkspaceDown":{}}}"#
    );
    assert_eq!(mock.next_request(), r#"{"Action":{"FocusWorkspaceUp":{}}}"#);
    assert_eq!(
        mock.next_request(),
        r#"{"Action":{"FocusWindow":{"id":99}}}"#
    );
    drop(rx);
}

#[test]
fn workspace_pill_click_dispatches_niri_focus_action() {
    let mock = MockNiri::start("pill-click", MockConfig::events(Vec::new()));
    let (tx, _rx) = event_channel::<ShellEvent>();
    let ipc = NiriIpc::connect_to(mock.socket_path(), tx).expect("connect to mock niri");

    let mut shell = TritonShell::new(BackendMode::RealServices);
    shell.set_niri_action_sink(ipc.action_sink());
    let bar = SurfaceId {
        role: SurfaceRole::Bar,
        output: OutputKey(1),
    };
    let cmds = shell.update(bar, ShellMessage::FocusWorkspace(9), Instant::now());
    assert!(cmds.contains(&triton_engine::AppCommand::RedrawAll));

    assert_eq!(
        mock.next_request(),
        "\"Outputs\"",
        "event-stream handshake happens first"
    );
    assert_eq!(mock.next_request(), "\"EventStream\"");
    assert_eq!(
        mock.next_request(),
        r#"{"Action":{"FocusWorkspace":{"reference":{"Id":9}}}}"#
    );
}

#[test]
fn missing_socket_and_rejected_event_stream_report_errors_cleanly() {
    let dir = TempRuntimeDir::new("missing-sock");
    let (tx, _rx) = event_channel::<ShellEvent>();
    let err = NiriIpc::connect_to(dir.path.join("does-not-exist.sock"), tx)
        .expect_err("missing socket path must fail");
    assert!(matches!(err, NiriIpcError::Io(_)));

    let mock = MockNiri::start(
        "rejected",
        MockConfig {
            event_stream_error: Some("niri refused".to_owned()),
            ..MockConfig::events(Vec::new())
        },
    );
    let (tx, _rx) = event_channel::<ShellEvent>();
    let err =
        NiriIpc::connect_to(mock.socket_path(), tx).expect_err("rejected EventStream must fail");
    assert!(matches!(err, NiriIpcError::RequestRejected(_)));
}

#[test]
fn unknown_event_lines_are_skipped_and_the_stream_continues() {
    let events = vec![
        r#"{"SomeFutureEvent":{"whatever":true}}"#.to_owned(),
        "this is not json".to_owned(),
        r#"{"WorkspacesChanged":{"workspaces":[{"id":1,"idx":1,"name":null,"output":"eDP-1","is_urgent":false,"is_active":true,"is_focused":true,"active_window_id":null}]}}"#.to_owned(),
    ];
    let mock = MockNiri::start("unknown-events", MockConfig::events(events));
    let (tx, rx) = event_channel::<ShellEvent>();
    let _ipc = NiriIpc::connect_to(mock.socket_path(), tx).expect("connect to mock niri");

    let events = collect_until(&rx, 1, Duration::from_secs(5), |e| {
        matches!(e, ShellEvent::WorkspacesUpdated(_))
    });
    assert!(
        events
            .iter()
            .any(|e| matches!(e, ShellEvent::WorkspacesUpdated(_))),
        "known events still arrive after unrecognized lines"
    );
}

#[test]
fn event_stream_disconnect_drops_cleanly() {
    let events = vec![r#"{"WorkspacesChanged":{"workspaces":[]}}"#.to_owned()];
    let mock = MockNiri::start(
        "disconnect",
        MockConfig {
            close_after_events: true,
            ..MockConfig::events(events)
        },
    );
    let (tx, rx) = event_channel::<ShellEvent>();
    let ipc = NiriIpc::connect_to(mock.socket_path(), tx).expect("connect to mock niri");
    let _ = collect_until(&rx, 1, Duration::from_secs(5), |e| {
        matches!(e, ShellEvent::WorkspacesUpdated(_))
    });
    // Dropping must join the event + action threads without hanging.
    drop(ipc);
}

#[test]
fn fake_dev_mode_has_no_niri_action_sink() {
    let shell = TritonShell::new(BackendMode::Fake);
    assert!(shell.niri_action_sink().is_none());
    let bar = SurfaceId {
        role: SurfaceRole::Bar,
        output: OutputKey(1),
    };
    let mut shell = shell;
    // Dispatching a workspace focus without a sink must not panic or block.
    let cmds = shell.update(bar, ShellMessage::FocusWorkspace(1), Instant::now());
    assert!(cmds.contains(&triton_engine::AppCommand::RedrawAll));
}
