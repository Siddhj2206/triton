//! Structured JSON IPC protocol and Unix domain socket server/client for `triton msg`.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{debug, warn};
use triton_engine::EventSender;

use crate::state::{OsdPayload, ShellEvent};

/// Environment variable overriding the default `$XDG_RUNTIME_DIR/triton.sock` path.
pub const SOCKET_ENV_VAR: &str = "TRITON_SOCKET";

/// Default socket filename inside `$XDG_RUNTIME_DIR`.
pub const DEFAULT_SOCKET_NAME: &str = "triton.sock";

/// Maximum request payload size accepted over the Unix socket (64 KiB).
const MAX_REQUEST_BYTES: u64 = 64 * 1024;

/// I/O timeout for individual socket reads and writes.
const SOCKET_IO_TIMEOUT: Duration = Duration::from_secs(5);

/// Maximum time the IPC thread waits for the main `ShellApp` loop to reply.
const REPLY_TIMEOUT: Duration = Duration::from_secs(5);

/// Errors returned by the IPC socket server or client.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum IpcError {
    #[error("neither --socket, {SOCKET_ENV_VAR}, nor XDG_RUNTIME_DIR is set")]
    NoRuntimeDir,
    #[error("a triton daemon is already listening on {0}")]
    AlreadyRunning(PathBuf),
    #[error("IPC I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("IPC JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("daemon did not respond before timeout")]
    Timeout,
    #[error("daemon event channel is closed")]
    ChannelClosed,
}

/// Bar control subcommands for `triton msg bar`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum BarCommand {
    Autohide,
    Pinned,
    Hidden,
    Toggle,
}

/// OSD control subcommands for `triton msg osd`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "kebab-case")]
#[non_exhaustive]
pub enum OsdCommand {
    Show {
        payload: OsdPayload,
        #[serde(default)]
        output: Option<String>,
    },
    Hide,
}

/// Theme control subcommands for `triton msg theme`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "kebab-case")]
#[non_exhaustive]
pub enum ThemeCommand {
    Dark,
    Light,
    ToggleVariant,
    ReduceMotion { enabled: bool },
}

/// Structured command sent from `triton msg` to the running daemon.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "kebab-case")]
#[non_exhaustive]
pub enum IpcCommand {
    Status,
    Bar {
        action: BarCommand,
    },
    Osd {
        action: OsdCommand,
    },
    Wallpaper {
        path: PathBuf,
        #[serde(default)]
        output: Option<String>,
    },
    Theme {
        action: ThemeCommand,
    },
    Quit,
}

/// Structured response returned by the daemon over the Unix socket.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum IpcResponse {
    Ok {
        #[serde(skip_serializing_if = "Option::is_none")]
        message: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        data: Option<serde_json::Value>,
    },
    Error {
        message: String,
    },
}

impl IpcResponse {
    /// Construct a successful response with an optional human-readable message.
    pub fn ok(message: impl Into<String>) -> Self {
        Self::Ok {
            message: Some(message.into()),
            data: None,
        }
    }

    /// Construct a successful response carrying structured JSON data.
    pub fn ok_data(data: serde_json::Value) -> Self {
        Self::Ok {
            message: None,
            data: Some(data),
        }
    }

    /// Construct an error response.
    pub fn error(message: impl Into<String>) -> Self {
        Self::Error {
            message: message.into(),
        }
    }
}

/// Resolve the Unix socket path from an explicit override, `TRITON_SOCKET`,
/// or `$XDG_RUNTIME_DIR/triton.sock`.
pub fn resolve_socket_path(override_path: Option<&Path>) -> Result<PathBuf, IpcError> {
    if let Some(path) = override_path {
        return Ok(path.to_path_buf());
    }
    if let Ok(env_socket) = std::env::var(SOCKET_ENV_VAR)
        && !env_socket.is_empty()
    {
        return Ok(PathBuf::from(env_socket));
    }
    if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR")
        && !runtime_dir.is_empty()
    {
        return Ok(PathBuf::from(runtime_dir).join(DEFAULT_SOCKET_NAME));
    }
    Err(IpcError::NoRuntimeDir)
}

/// Background Unix-socket server accepting `triton msg` connections and
/// forwarding them into `TritonShell` via [`EventSender`].
#[derive(Debug)]
pub struct IpcServer {
    path: PathBuf,
    shutdown: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl IpcServer {
    /// Bind a Unix domain socket at `path` and start serving requests on a
    /// dedicated background thread.
    pub fn bind(path: PathBuf, sender: EventSender<ShellEvent>) -> Result<Self, IpcError> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }

        if path.exists() {
            if UnixStream::connect(&path).is_ok() {
                return Err(IpcError::AlreadyRunning(path));
            }
            // Stale socket file left by an earlier crash; safe to unlink.
            std::fs::remove_file(&path)?;
        }

        let listener = UnixListener::bind(&path)?;
        let shutdown = Arc::new(AtomicBool::new(false));
        let worker_shutdown = Arc::clone(&shutdown);
        let worker = thread::Builder::new()
            .name("triton-ipc".to_owned())
            .spawn(move || serve_loop(&listener, &sender, &worker_shutdown))?;

        Ok(Self {
            path,
            shutdown,
            worker: Some(worker),
        })
    }

    /// Path of the bound Unix socket.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for IpcServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        // Unblock the listener's blocking `accept()` immediately without polling.
        let _ = UnixStream::connect(&self.path);
        if let Some(handle) = self.worker.take() {
            let _ = handle.join();
        }
        let _ = std::fs::remove_file(&self.path);
    }
}

fn serve_loop(listener: &UnixListener, sender: &EventSender<ShellEvent>, shutdown: &AtomicBool) {
    for incoming in listener.incoming() {
        if shutdown.load(Ordering::Acquire) {
            break;
        }
        match incoming {
            Ok(stream) => {
                if let Err(err) = handle_connection(stream, sender) {
                    debug!("triton IPC connection error: {err}");
                }
            }
            Err(err) => {
                if shutdown.load(Ordering::Acquire) {
                    break;
                }
                warn!("triton IPC accept error: {err}");
            }
        }
    }
}

fn handle_connection(
    mut stream: UnixStream,
    sender: &EventSender<ShellEvent>,
) -> Result<(), IpcError> {
    stream.set_read_timeout(Some(SOCKET_IO_TIMEOUT))?;
    stream.set_write_timeout(Some(SOCKET_IO_TIMEOUT))?;

    let mut line = String::new();
    {
        let mut reader = BufReader::new((&stream).take(MAX_REQUEST_BYTES));
        reader.read_line(&mut line)?;
    }

    if line.trim().is_empty() {
        return Ok(());
    }

    let response = match serde_json::from_str::<IpcCommand>(&line) {
        Ok(command) => {
            let (reply_tx, reply_rx) = mpsc::channel();
            if sender
                .send(ShellEvent::Ipc {
                    command,
                    reply: reply_tx,
                })
                .is_err()
            {
                IpcResponse::error("daemon event loop is closed")
            } else {
                match reply_rx.recv_timeout(REPLY_TIMEOUT) {
                    Ok(resp) => resp,
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        IpcResponse::error("daemon timed out handling command")
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        IpcResponse::error("daemon disconnected before replying")
                    }
                }
            }
        }
        Err(err) => IpcResponse::error(format!("invalid IPC request: {err}")),
    };

    let mut encoded = serde_json::to_string(&response)?;
    encoded.push('\n');
    stream.write_all(encoded.as_bytes())?;
    stream.flush()?;
    Ok(())
}

/// Send a single [`IpcCommand`] to the daemon listening at `socket_path`.
pub fn send_command(socket_path: &Path, command: &IpcCommand) -> Result<IpcResponse, IpcError> {
    let mut stream = UnixStream::connect(socket_path)?;
    stream.set_read_timeout(Some(SOCKET_IO_TIMEOUT))?;
    stream.set_write_timeout(Some(SOCKET_IO_TIMEOUT))?;

    let mut encoded = serde_json::to_string(command)?;
    encoded.push('\n');
    stream.write_all(encoded.as_bytes())?;
    stream.flush()?;

    let mut line = String::new();
    let mut reader = BufReader::new(stream);
    reader.read_line(&mut line)?;
    if line.trim().is_empty() {
        return Err(IpcError::ChannelClosed);
    }
    let response = serde_json::from_str::<IpcResponse>(&line)?;
    Ok(response)
}
