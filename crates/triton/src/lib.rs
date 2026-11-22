//! Triton desktop shell daemon library.
//!
//! Single-process `ShellApp` host (`TritonShell`) with declarative
//! `(SurfaceRole, OutputName)` surface reconciliation, typed `ShellEvent`s,
//! `triton msg` Unix-socket IPC, and CLI parsing.
//!
//! Composes `triton-kit` components exclusively (ADR-0006, ADR-0008).

#![forbid(unsafe_code)]

pub mod cli;
pub mod ipc;
pub mod niri_config;
pub mod shell;
pub mod state;
pub mod wallpaper;

pub use cli::{CliCommand, CliError, DaemonArgs, MsgArgs, parse_args};
pub use ipc::{
    BarCommand, IpcCommand, IpcError, IpcResponse, IpcServer, OsdCommand, ThemeCommand,
    resolve_socket_path, send_command,
};
pub use niri_config::{
    INCLUDE_LINE, NiriConfigError, OWNED_DIR, install, niri_config_dir, render_binds,
    render_colors, render_index, render_layer_rules, write_colors, write_layer_rules,
    write_owned_files,
};
pub use shell::{ShellMessage, SurfaceId, TIMER_OSD_HIDE, TIMER_WALLPAPER_SETTLE, TritonShell};
pub use state::{
    BackendMode, BarMode, FocusedWindow, OsdPayload, OsdState, OutputKey, ShellEvent, ShellState,
    ShellStatusSnapshot, StatusSummary, SurfaceRole, SurfaceSummary, WallpaperState, WorkspaceInfo,
};
pub use wallpaper::{
    BRIGHT_BACKDROP_THRESHOLD, CROSSFADE_DURATION, CROSSFADE_DURATION_REDUCED, Crossfade,
    WallpaperError, WallpaperImage, WallpaperSet, crossfade_duration, mean_relative_luminance,
    spawn_decode, view_background,
};
