//! Real Unix-socket round-trip integration tests for `triton msg` IPC.

use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use triton::{
    BackendMode, BarCommand, BarMode, IpcCommand, IpcResponse, IpcServer, OsdCommand, OsdPayload,
    ShellEvent, ShellMessage, ShellStatusSnapshot, ThemeCommand, TritonShell, resolve_socket_path,
    send_command,
};
use triton_design::color::Variant;
use triton_engine::{AppCommand, HeadlessSession, event_channel};

static NEXT_DIR_ID: AtomicU64 = AtomicU64::new(1);

struct TempRuntimeDir {
    path: PathBuf,
}

impl TempRuntimeDir {
    fn new(label: &str) -> Self {
        let id = NEXT_DIR_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("triton-{label}-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&path).expect("create temp runtime dir");
        Self { path }
    }

    fn socket_path(&self) -> PathBuf {
        self.path.join("triton.sock")
    }
}

impl Drop for TempRuntimeDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn dispatch_one(
    socket_path: &Path,
    command: &IpcCommand,
    shell: &mut TritonShell,
    session: &mut HeadlessSession<ShellMessage>,
    rx: &triton_engine::EventReceiver<ShellEvent>,
) -> (IpcResponse, Vec<AppCommand<triton::SurfaceId>>) {
    std::thread::scope(|scope| {
        let client = scope.spawn(|| send_command(socket_path, command));
        let deadline = Instant::now() + Duration::from_secs(3);
        let cmds = loop {
            if let Some(event) = rx.try_recv() {
                break shell.handle_event(event, session.now());
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for ShellEvent from IPC server"
            );
            std::thread::yield_now();
        };
        let response = client
            .join()
            .expect("client thread did not panic")
            .expect("IPC command round-trip succeeded");
        (response, cmds)
    })
}

#[test]
fn round_trips_status_bar_and_osd_commands_over_unix_socket() {
    let runtime = TempRuntimeDir::new("ipc-roundtrip-bar-osd");
    let socket_path = runtime.socket_path();

    let (tx, rx) = event_channel::<ShellEvent>();
    let server = IpcServer::bind(socket_path.clone(), tx).expect("bind IPC server");
    assert_eq!(server.path(), socket_path.as_path());

    let mut shell = TritonShell::new(BackendMode::Fake);
    let mut session = HeadlessSession::<ShellMessage>::new((1920.0, 1080.0), 1.0);

    // 1. status
    let (resp, cmds) = dispatch_one(
        &socket_path,
        &IpcCommand::Status,
        &mut shell,
        &mut session,
        &rx,
    );
    assert_eq!(cmds.len(), 0);
    let IpcResponse::Ok {
        data: Some(data), ..
    } = resp
    else {
        panic!("expected Ok with status data, got {resp:?}");
    };
    let snapshot: ShellStatusSnapshot =
        serde_json::from_value(data).expect("deserialize ShellStatusSnapshot");
    assert_eq!(snapshot.backend, "fake");
    assert_eq!(snapshot.bar_mode, BarMode::Autohide);
    assert_eq!(snapshot.variant, "dark");
    assert!(!snapshot.osd_active);
    assert_eq!(snapshot.outputs, vec!["eDP-1".to_owned()]);

    // 2. bar pinned -> bar toggle (hidden)
    let (resp, cmds) = dispatch_one(
        &socket_path,
        &IpcCommand::Bar {
            action: BarCommand::Pinned,
        },
        &mut shell,
        &mut session,
        &rx,
    );
    assert!(matches!(resp, IpcResponse::Ok { .. }));
    assert!(cmds.contains(&AppCommand::RedrawAll));
    assert_eq!(shell.state().bar_mode, BarMode::Pinned);

    let (resp, _) = dispatch_one(
        &socket_path,
        &IpcCommand::Bar {
            action: BarCommand::Toggle,
        },
        &mut shell,
        &mut session,
        &rx,
    );
    assert!(matches!(resp, IpcResponse::Ok { .. }));
    assert_eq!(shell.state().bar_mode, BarMode::Hidden);

    // 3. osd show -> osd hide
    let (resp, cmds) = dispatch_one(
        &socket_path,
        &IpcCommand::Osd {
            action: OsdCommand::Show {
                payload: OsdPayload::Brightness { level: 0.5 },
                output: None,
            },
        },
        &mut shell,
        &mut session,
        &rx,
    );
    assert!(matches!(resp, IpcResponse::Ok { .. }));
    assert!(shell.state().osd.is_some());
    assert!(
        cmds.iter()
            .any(|c| matches!(c, AppCommand::ScheduleTimer { .. }))
    );

    let (resp, _) = dispatch_one(
        &socket_path,
        &IpcCommand::Osd {
            action: OsdCommand::Hide,
        },
        &mut shell,
        &mut session,
        &rx,
    );
    assert!(matches!(resp, IpcResponse::Ok { .. }));
    assert!(shell.state().osd.is_none());

    drop(server);
    assert!(!socket_path.exists(), "socket file cleaned up on drop");
}

#[test]
fn round_trips_wallpaper_theme_and_quit_commands_over_unix_socket() {
    let runtime = TempRuntimeDir::new("ipc-roundtrip-wall-theme");
    let socket_path = runtime.socket_path();

    let (tx, rx) = event_channel::<ShellEvent>();
    let server = IpcServer::bind(socket_path.clone(), tx).expect("bind IPC server");

    let mut shell = TritonShell::new(BackendMode::Fake);
    let mut session = HeadlessSession::<ShellMessage>::new((1920.0, 1080.0), 1.0);

    // 4. wallpaper
    let wall_path = PathBuf::from("/tmp/example-wallpaper.png");
    let (resp, _) = dispatch_one(
        &socket_path,
        &IpcCommand::Wallpaper {
            path: wall_path.clone(),
            output: Some("eDP-1".to_owned()),
        },
        &mut shell,
        &mut session,
        &rx,
    );
    assert!(matches!(resp, IpcResponse::Ok { .. }));
    assert_eq!(shell.state().wallpaper.path_for("eDP-1"), Some(&wall_path));

    // 5. theme light + reduce_motion
    let (resp, _) = dispatch_one(
        &socket_path,
        &IpcCommand::Theme {
            action: ThemeCommand::Light,
        },
        &mut shell,
        &mut session,
        &rx,
    );
    assert!(matches!(resp, IpcResponse::Ok { .. }));
    assert_eq!(shell.state().theme.variant, Variant::Light);

    let (resp, cmds) = dispatch_one(
        &socket_path,
        &IpcCommand::Theme {
            action: ThemeCommand::ReduceMotion { enabled: true },
        },
        &mut shell,
        &mut session,
        &rx,
    );
    assert!(matches!(resp, IpcResponse::Ok { .. }));
    assert!(shell.state().reduce_motion);
    assert!(cmds.contains(&AppCommand::SetReduceMotion(true)));

    // 6. quit
    let (resp, cmds) = dispatch_one(
        &socket_path,
        &IpcCommand::Quit,
        &mut shell,
        &mut session,
        &rx,
    );
    assert!(matches!(resp, IpcResponse::Ok { .. }));
    assert!(cmds.contains(&AppCommand::Quit));

    drop(server);
    assert!(!socket_path.exists(), "socket file cleaned up on drop");
}

#[test]
fn stale_socket_file_is_replaced_cleanly_on_bind() {
    let runtime = TempRuntimeDir::new("ipc-stale");
    let socket_path = runtime.socket_path();

    // Create a stale socket file by binding and dropping a listener without unlinking.
    {
        let _stale = UnixListener::bind(&socket_path).expect("create stale socket");
    }
    assert!(socket_path.exists());

    let (tx, rx) = event_channel::<ShellEvent>();
    let _server = IpcServer::bind(socket_path.clone(), tx).expect("recover from stale socket");

    let mut shell = TritonShell::new(BackendMode::Fake);
    let mut session = HeadlessSession::<ShellMessage>::new((1920.0, 1080.0), 1.0);
    let (resp, _) = dispatch_one(
        &socket_path,
        &IpcCommand::Status,
        &mut shell,
        &mut session,
        &rx,
    );
    assert!(matches!(resp, IpcResponse::Ok { .. }));
}

#[test]
fn resolve_socket_path_prefers_explicit_override() {
    let explicit = PathBuf::from("/tmp/custom-triton-override.sock");
    let resolved = resolve_socket_path(Some(&explicit)).expect("override always resolves");
    assert_eq!(resolved, explicit);
}
