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
pub mod shell;
pub mod state;

pub use cli::{CliCommand, CliError, DaemonArgs, MsgArgs, parse_args};
pub use ipc::{
    BarCommand, IpcCommand, IpcError, IpcResponse, IpcServer, OsdCommand, ThemeCommand,
    resolve_socket_path, send_command,
};
pub use shell::{ShellMessage, SurfaceId, TIMER_OSD_HIDE, TritonShell};
pub use state::{
    BackendMode, BarMode, FocusedWindow, OsdPayload, OsdState, OutputKey, ShellEvent, ShellState,
    ShellStatusSnapshot, StatusSummary, SurfaceRole, SurfaceSummary, WallpaperState, WorkspaceInfo,
};
