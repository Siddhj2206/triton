//! Turning a hit back into a process. Apps are spawned through `setsid` so
//! they get their own session: a launcher that dies (or just closes the
//! panel) must never take the launched app with it. `Terminal=true` apps go
//! through a terminal emulator — `$TERMINAL` if set, else the GNOME trio the
//! canvas ships with.
//!
//! `Command::output`/`status` are banned workspace-wide; only `spawn` is used
//! and the returned child is dropped immediately — `setsid` already detached
//! the real process.

use std::ffi::OsStr;
use std::io;
use std::path::Path;
use std::process::{Command, Stdio};

use tracing::debug;

use super::{Index, in_path};

/// The terminal emulators tried when `$TERMINAL` is unset, in canvas order.
/// All three accept `--` before the command to run.
const TERMINALS: &[&str] = &["kgx", "ptyxis", "gnome-terminal"];

/// Spawn an app's `Exec` line (or one of its action's), detached. Errors are
/// plain `io::Error`s: the caller only needs to know it failed to toast it.
pub fn launch_app(index: &Index, app: usize, action: Option<usize>) -> io::Result<()> {
    let invalid = |what: &str| io::Error::new(io::ErrorKind::InvalidInput, what.to_owned());
    let app = index
        .apps
        .get(app)
        .ok_or_else(|| invalid("app index out of range"))?;
    let exec = match action {
        Some(a) => {
            &app.actions
                .get(a)
                .ok_or_else(|| invalid("action index out of range"))?
                .exec
        }
        None => &app.exec,
    };
    let (program, args) = exec
        .0
        .split_first()
        .ok_or_else(|| invalid("empty Exec line"))?;

    // `setsid <cmd>` puts the child in its own session, detaching it from us.
    let mut command = Command::new("setsid");
    if app.terminal {
        let terminal = terminal_argv()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no terminal emulator found"))?;
        command.args(terminal);
    }
    debug!(app = %app.id, terminal = app.terminal, "launching");
    command
        .arg(program)
        .args(args)
        .stdin(Stdio::null())
        .spawn()?;
    Ok(())
}

/// The argv prefix that runs a command inside a terminal: `$TERMINAL` first
/// (it may carry its own options, so it is split on whitespace), then the
/// known emulators. The known ones take `-- command`; for an unknown
/// `$TERMINAL` the POSIX-ish `-e` convention is the best guess.
fn terminal_argv() -> Option<Vec<String>> {
    if let Ok(term) = std::env::var("TERMINAL") {
        let mut argv: Vec<String> = term.split_whitespace().map(str::to_owned).collect();
        if let Some(program) = argv.first()
            && in_path(program)
        {
            let known = TERMINALS
                .iter()
                .any(|t| Path::new(program).file_name() == Some(OsStr::new(t)));
            argv.push(if known {
                "--".to_owned()
            } else {
                "-e".to_owned()
            });
            return Some(argv);
        }
    }
    TERMINALS
        .iter()
        .copied()
        .find(|t| in_path(t))
        .map(|t| vec![t.to_owned(), "--".to_owned()])
}
