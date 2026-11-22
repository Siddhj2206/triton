//! `triton` binary entry point: daemon mode and `triton msg` CLI client.

#![forbid(unsafe_code)]

use std::io::IsTerminal;
use std::process::ExitCode;

use anyhow::{Context, Result};
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;
use triton::cli::USAGE;
use triton::{
    BackendMode, CliCommand, DaemonArgs, IpcResponse, IpcServer, MsgArgs, NiriIpc, ShellEvent,
    ThemeSyncConfig, TritonShell, parse_args, resolve_socket_path, send_command,
};
use triton::{niri_config, theme_pipeline};
use triton_engine::{event_channel, run_with_events};

fn main() -> ExitCode {
    match run_cli() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("triton: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn run_cli() -> Result<()> {
    let command = parse_args(std::env::args().skip(1))?;
    match command {
        CliCommand::Help => {
            print!("{USAGE}");
            Ok(())
        }
        CliCommand::Msg(args) => run_msg(&args),
        CliCommand::Daemon(args) => run_daemon(&args),
    }
}

fn run_msg(args: &MsgArgs) -> Result<()> {
    let socket_path =
        resolve_socket_path(args.socket.as_deref()).context("resolve triton IPC socket path")?;
    let response = send_command(&socket_path, &args.command)
        .with_context(|| format!("send command to {}", socket_path.display()))?;

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&response).context("format IPC response as JSON")?
        );
    } else {
        match &response {
            IpcResponse::Ok {
                data: Some(data), ..
            } => {
                println!(
                    "{}",
                    serde_json::to_string_pretty(data).context("format status data as JSON")?
                );
            }
            IpcResponse::Ok {
                message: Some(msg), ..
            } => {
                println!("{msg}");
            }
            IpcResponse::Ok { .. } => {
                println!("ok");
            }
            IpcResponse::Error { message } => {
                anyhow::bail!("{message}");
            }
        }
    }

    if let IpcResponse::Error { message } = response {
        anyhow::bail!("{message}");
    }
    Ok(())
}

fn run_daemon(args: &DaemonArgs) -> Result<()> {
    init_logging();
    install_panic_hook();

    let socket_path =
        resolve_socket_path(args.socket.as_deref()).context("resolve triton IPC socket path")?;
    let (event_tx, event_rx) = event_channel::<ShellEvent>();
    let _ipc_server = IpcServer::bind(socket_path.clone(), event_tx.clone())
        .with_context(|| format!("bind IPC socket at {}", socket_path.display()))?;

    info!(
        backend = args.backend.as_str(),
        socket = %socket_path.display(),
        "starting triton shell daemon"
    );

    let mut app = TritonShell::new(args.backend);
    // Lets `SetWallpaper` spawn off-thread decode workers that deliver
    // `WallpaperReady`/`WallpaperFailed` back into this same event loop.
    app.set_event_sender(event_tx.clone());

    // ADR-0004: the only write the host `config.kdl` ever gets, and only under
    // `--real-services`. The fake dev backend never touches niri config, GTK
    // CSS, or host `gsettings`.
    if args.backend == BackendMode::RealServices {
        install_niri_config(&app);
        configure_theme_sync(&mut app);
    }

    // Live niri IPC only under --real-services; default fake dev mode never
    // touches the host $NIRI_SOCKET.
    let _niri_ipc = if args.backend == BackendMode::RealServices {
        match NiriIpc::connect(event_tx) {
            Ok(ipc) => {
                info!(socket = %ipc.socket_path().display(), "connected to niri IPC");
                app.set_niri_action_sink(ipc.action_sink());
                Some(ipc)
            }
            Err(err) => {
                warn!(error = %err, "niri IPC unavailable; running without compositor integration");
                None
            }
        }
    } else {
        None
    };

    run_with_events(app, event_rx, TritonShell::handle_event)
        .context("triton Wayland event loop exited with error")?;

    Ok(())
}

/// ADR-0004 one-time owned-config install at `--real-services` startup.
///
/// A failure degrades blur/xray rules and niri-side colors; it never takes the
/// shell down — the daemon reports it and keeps running.
fn install_niri_config(app: &TritonShell) {
    let Some(config_dir) = niri_config::niri_config_dir() else {
        warn!("HOME/XDG_CONFIG_HOME unset; skipping owned niri config install");
        return;
    };
    let state = app.state();
    match niri_config::install(
        &config_dir,
        &state.theme.scheme,
        state.theme.variant,
        state.bar_mode,
    ) {
        Ok(()) => info!(dir = %config_dir.display(), "installed owned niri config"),
        Err(err) => {
            error!(dir = %config_dir.display(), error = %err, "owned niri config install failed");
        }
    }
}

/// Wire live GTK3/4 `gtk.css`, `triton/colors.kdl`, and `gsettings` syncing
/// under `--real-services`, and perform the initial startup sync.
fn configure_theme_sync(app: &mut TritonShell) {
    let Some(config_root) = theme_pipeline::xdg_config_home() else {
        warn!("HOME/XDG_CONFIG_HOME unset; skipping GTK/gsettings theme sync");
        return;
    };
    let state = app.state();
    if let Err(err) =
        theme_pipeline::write_gtk_css(&config_root, &state.theme.scheme, state.theme.variant)
    {
        error!(dir = %config_root.display(), error = %err, "initial GTK css write failed");
    }
    if let Err(err) = theme_pipeline::apply_gsettings(state.theme.variant, state.backend) {
        warn!(error = %err, "initial gsettings color-scheme update failed");
    }
    app.set_theme_sync(ThemeSyncConfig::real_services(config_root));
}

fn init_logging() {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("triton=info,warn"));
    let use_ansi = std::io::stderr().is_terminal();
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_ansi(use_ansi)
        .try_init();
}

fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        error!(panic = %info, "unhandled panic in triton");
        default_hook(info);
    }));
}
