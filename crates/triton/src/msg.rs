//! `triton msg …`: the CLI niri binds call (roadmap §3). One line over
//! `$XDG_RUNTIME_DIR/triton.sock`; the daemon streams each line in as a message.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::{fs, io, thread};

use iced_futures::Subscription;
use iced_futures::futures::Stream;
use iced_futures::futures::channel::mpsc;

fn socket() -> PathBuf {
    let base =
        std::env::var_os("XDG_RUNTIME_DIR").map_or_else(|| PathBuf::from("/tmp"), PathBuf::from);
    base.join("triton.sock")
}

/// Sends a command line to the running shell. `Err` means it is not listening.
pub fn send(command: &str) -> io::Result<()> {
    UnixStream::connect(socket())?.write_all(format!("{command}\n").as_bytes())
}

fn stream() -> impl Stream<Item = String> {
    let (tx, rx) = mpsc::unbounded();
    thread::Builder::new()
        .name("triton-msg".into())
        .spawn(move || {
            let path = socket();
            // A dead socket from a previous run.
            let _ = fs::remove_file(&path);
            let Ok(listener) = UnixListener::bind(&path) else {
                tracing::warn!(path = %path.display(), "msg: cannot bind the socket");
                return;
            };
            for conn in listener.incoming().flatten() {
                let mut line = String::new();
                if BufReader::new(conn).read_line(&mut line).is_ok()
                    && tx.unbounded_send(line.trim_end().to_owned()).is_err()
                {
                    return;
                }
            }
        })
        .map_err(|e| tracing::error!(%e, "msg: cannot start thread"))
        .ok();
    rx
}

/// Command lines as they arrive.
pub fn commands() -> Subscription<String> {
    Subscription::run(stream)
}
