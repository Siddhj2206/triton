//! The launcher's search engine (`Launcher` board): one field over apps,
//! open windows and shell commands, `>`/`=`/`/` scope prefixes, and detached
//! spawning of desktop entries.
//!
//! The app owns the field and the rendering; this module owns everything that
//! can be wrong: scanning the XDG application dirs into an [`Index`], turning a
//! query string into [`Results`], and turning a hit back into a spawned process
//! through [`launch_app`]. Math is a side channel: [`math`] reads the same raw
//! query and returns the card's two lines when the input is evaluable (the app
//! can also wrap it in a [`HitKind::Math`] hit). File search has no backend
//! yet; [`Scope::Files`] is already recognized so the UI can show its empty
//! state.

// The app wires the rest of this API in Phase 1; until then most of it is dead
// code and the expectation is fulfilled. The wiring change removes this.
#![expect(dead_code, reason = "wired by the launcher UI in Phase 1")]

mod index;
mod math;
mod search;
mod spawn;

pub use index::{App, AppAction, Exec, Index};
pub use math::math;
pub use search::{CommandRef, Group, Hit, HitKind, Results, Scope, WindowRef, scope, search};
pub use spawn::launch_app;

/// Whether `name` resolves to an executable: absolute paths must exist, bare
/// names are looked up in `PATH`. Used for `TryExec` and terminal lookup.
pub(crate) fn in_path(name: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;

    fn executable(path: &std::path::Path) -> bool {
        path.metadata()
            .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    }

    if name.contains('/') {
        return executable(std::path::Path::new(name));
    }
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| executable(&dir.join(name))))
}
