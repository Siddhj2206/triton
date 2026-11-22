//! Triton desktop shell daemon library.
//!
//! Single-process `ShellApp` host (`TritonShell`) with declarative
//! `(SurfaceRole, OutputName)` surface reconciliation, typed `ShellEvent`s,
//! `triton msg` Unix-socket IPC, and CLI parsing.
//!
//! Composes `triton-kit` components exclusively (ADR-0006, ADR-0008).

#![forbid(unsafe_code)]

mod fs_util;
mod ipc;
mod niri_ipc;
mod osd;
mod shell;
mod timer;
mod wallpaper;

pub mod bar;
pub mod cli;
pub mod headless;
pub mod niri_config;
pub mod state;
pub mod theme_pipeline;

pub use cli::{CliCommand, CliError, DaemonArgs, MsgArgs, parse_args};
pub use headless::HeadlessArgs;
pub use ipc::{
    BarCommand, IpcCommand, IpcResponse, IpcServer, OsdCommand, ThemeCommand, resolve_socket_path,
    send_command,
};
pub use niri_ipc::{NiriAction, NiriIpc, NiriIpcError};
pub use osd::{OsdPayload, TIMER_OSD_HIDE, TIMER_OSD_REAP};
pub use shell::{ShellMessage, SurfaceId, TIMER_WALLPAPER_SETTLE, TritonShell};
pub use state::{
    BackendMode, BarMode, FocusedWindow, OutputKey, ShellEvent, ShellStatusSnapshot, StatusSummary,
    SurfaceRole, WorkspaceInfo,
};
pub use theme_pipeline::{
    ThemeSyncConfig, apply_gsettings, extract_scheme, extract_source_color,
    gsettings_color_scheme_args, render_gtk3_css, render_gtk4_css, scheme_from_seed,
    sync_config_files, write_gtk_css,
};
pub use timer::TimerTag;
pub use wallpaper::{BRIGHT_BACKDROP_THRESHOLD, WallpaperImage, mean_relative_luminance};
