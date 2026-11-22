//! `triton msg …`: the CLI niri binds call (roadmap §3). One line over
//! `$XDG_RUNTIME_DIR/triton.sock`; the daemon streams each line in as a message.

use std::io::{BufRead, BufReader, Read, Write};
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

/// Sends a command line to the running shell and prints any reply.
/// `Err` means it is not listening.
pub fn send(command: &str) -> io::Result<()> {
    let mut conn = UnixStream::connect(socket())?;
    conn.write_all(format!("{command}\n").as_bytes())?;
    conn.shutdown(std::net::Shutdown::Write)?;
    // The daemon closes after replying; the timeout guards a dead shell.
    conn.set_read_timeout(Some(std::time::Duration::from_secs(2)))?;
    let mut reply = String::new();
    if conn.read_to_string(&mut reply).is_ok() && !reply.is_empty() {
        print!("{reply}");
    }
    Ok(())
}

/// Replies the daemon owes pending connections, by request id.
fn pending() -> &'static std::sync::Mutex<std::collections::HashMap<u64, UnixStream>> {
    static PENDING: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<u64, UnixStream>>,
    > = std::sync::OnceLock::new();
    PENDING.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

/// Answer request `id` with `text` and close its connection. A second call for
/// the same id is a no-op — the command handler may reply early (e.g. `status`)
/// and the dispatcher always replies at the end.
pub fn reply(id: u64, text: &str) {
    let conn = pending()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(&id);
    if let Some(mut conn) = conn {
        let _ = conn.write_all(text.as_bytes());
    }
}

fn stream() -> impl Stream<Item = (u64, String)> {
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
            for (id, conn) in listener.incoming().flatten().enumerate() {
                let Ok(reader_conn) = conn.try_clone() else {
                    continue;
                };
                let mut line = String::new();
                if BufReader::new(reader_conn).read_line(&mut line).is_ok() {
                    pending()
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .insert(id as u64, conn);
                    if tx
                        .unbounded_send((id as u64, line.trim_end().to_owned()))
                        .is_err()
                    {
                        return;
                    }
                }
            }
        })
        .map_err(|e| tracing::error!(%e, "msg: cannot start thread"))
        .ok();
    rx
}

/// Command lines as they arrive: `(request id, command)`.
pub fn commands() -> Subscription<(u64, String)> {
    Subscription::run(stream)
}
