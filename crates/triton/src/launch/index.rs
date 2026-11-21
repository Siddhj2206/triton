//! The app index: `.desktop` files under the XDG application dirs, parsed once
//! and kept so a query is a memory-only walk. `Hidden` is honored as "the user
//! deleted this", so it also masks the same desktop id in lower-precedence
//! dirs; `NoDisplay` just drops the entry. Missing dirs, unreadable files and
//! non-`Application` types are skipped, never fatal — a launcher that dies on
//! a bad desktop file is worse than one that misses an app.

use std::collections::HashSet;
use std::path::PathBuf;

use freedesktop_desktop_entry::{DesktopEntry, Iter, get_languages_from_env};
use tracing::debug;

use super::in_path;

/// Parsed desktop entries, deduplicated by desktop id and sorted by name.
#[derive(Debug, Clone, Default)]
pub struct Index {
    /// The launchable apps, in name order. `HitKind::App::app` indexes this.
    pub apps: Vec<App>,
}

/// One launchable app.
#[derive(Debug, Clone)]
pub struct App {
    /// Desktop file id (`org.gnome.Console.desktop`).
    pub id: String,
    /// Localized `Name`.
    pub name: String,
    /// `GenericName`, falling back to `Comment`; the result row's second line.
    pub generic: Option<String>,
    /// The `Exec` line, ready to spawn.
    pub exec: Exec,
    /// `Icon`: a themed name or an absolute path.
    pub icon: Option<String>,
    /// Localized `Keywords`, extra haystack for matching.
    pub keywords: Vec<String>,
    /// `[Desktop Action …]` groups, in the file's `Actions` order.
    pub actions: Vec<AppAction>,
    /// `Terminal=true`: launch inside a terminal emulator.
    pub terminal: bool,
}

/// An `Exec` line with field codes stripped and quoting honored: element 0 is
/// the program, the rest its arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exec(pub Vec<String>);

/// A `[Desktop Action …]` group (`new-window`, `open-private`, …).
#[derive(Debug, Clone)]
pub struct AppAction {
    /// The action's id as listed in `Actions`.
    pub id: String,
    /// Localized action `Name` ("New window").
    pub name: String,
    /// The action's own `Exec` line.
    pub exec: Exec,
}

impl Index {
    /// Scan the real XDG dirs: `$XDG_DATA_HOME/applications`, every
    /// `$XDG_DATA_DIRS` entry's `applications/`, then the flatpak exports
    /// (`~/.local/share/flatpak` and `/var/lib/flatpak`).
    pub fn scan() -> Self {
        Self::scan_in(&app_dirs())
    }

    /// Scan an explicit dir list, in precedence order; [`Iter`] walks each dir
    /// recursively and skips files it cannot parse. Split out from [`scan`] so
    /// tests point at fixture dirs instead of env vars.
    ///
    /// [`scan`]: Self::scan
    pub fn scan_in(dirs: &[PathBuf]) -> Self {
        let locales = get_languages_from_env();
        let mut seen = HashSet::new();
        let mut apps = Vec::new();
        for entry in Iter::new(dirs.iter().cloned()).entries(Some(locales.as_slice())) {
            // Earlier dirs win. `seen` is marked before filtering so a
            // `Hidden`/`NoDisplay` file still masks lower-precedence ids.
            if !seen.insert(entry.id().to_owned()) {
                continue;
            }
            if let Some(app) = App::from_entry(&entry, &locales) {
                apps.push(app);
            } else {
                debug!(id = %entry.id(), "skipped desktop entry");
            }
        }
        apps.sort_by_key(|a| a.name.to_lowercase());
        Self { apps }
    }
}

impl App {
    /// The fields the launcher needs from one parsed entry; `None` for entries
    /// that must not appear (`Hidden`, `NoDisplay`, no usable `Exec`, failed
    /// `TryExec`). `OnlyShowIn` is deliberately ignored: triton is not a
    /// registered desktop name, so honoring it would hide half the apps.
    fn from_entry(entry: &DesktopEntry, locales: &[String]) -> Option<Self> {
        if entry.hidden() || entry.no_display() {
            return None;
        }
        if entry
            .desktop_entry("Type")
            .is_some_and(|t| t != "Application")
        {
            return None;
        }
        let exec = Exec(entry.parse_exec().ok()?);
        if exec.0.is_empty() {
            return None;
        }
        if let Some(try_exec) = entry.try_exec()
            && !in_path(try_exec)
        {
            return None;
        }
        let actions = entry
            .actions()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|id| {
                Some(AppAction {
                    id: id.to_owned(),
                    name: entry.action_name(id, locales)?.into_owned(),
                    exec: Exec(entry.parse_exec_action(id).ok()?),
                })
            })
            .collect();
        Some(Self {
            id: entry.id().to_owned(),
            name: entry.name(locales)?.into_owned(),
            generic: entry
                .generic_name(locales)
                .or_else(|| entry.comment(locales))
                .map(std::borrow::Cow::into_owned),
            exec,
            icon: entry.icon().map(str::to_owned),
            keywords: entry
                .keywords(locales)
                .unwrap_or_default()
                .iter()
                .map(ToString::to_string)
                .collect(),
            actions,
            terminal: entry.terminal(),
        })
    }
}

/// The applications dirs in precedence order: data home, every data dir, then
/// the two flatpak export roots (always last, so a native app wins a tie).
fn app_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| home_dir().map(|h| h.join(".local/share")));
    if let Some(home) = data_home {
        dirs.push(home.join("applications"));
    }
    for dir in std::env::var("XDG_DATA_DIRS")
        .unwrap_or_else(|_| "/usr/local/share:/usr/share".to_owned())
        .split(':')
    {
        if !dir.is_empty() {
            dirs.push(PathBuf::from(dir).join("applications"));
        }
    }
    if let Some(home) = home_dir() {
        dirs.push(home.join(".local/share/flatpak/exports/share/applications"));
    }
    dirs.push(PathBuf::from("/var/lib/flatpak/exports/share/applications"));
    dirs
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(dir: &std::path::Path, name: &str, body: &str) {
        fs::write(dir.join(name), body).unwrap();
    }

    #[test]
    fn scan_reads_apps_and_skips_bad_ones() {
        let dir = tempfile::tempdir().unwrap();
        let apps = dir.path().join("applications");
        fs::create_dir(&apps).unwrap();
        write(
            &apps,
            "good.desktop",
            "[Desktop Entry]\nType=Application\nName=Console\n\
             GenericName=Terminal emulator\nExec=kgx\nIcon=terminal\n",
        );
        write(
            &apps,
            "hidden.desktop",
            "[Desktop Entry]\nType=Application\nName=Hidden\nExec=x\nHidden=true\n",
        );
        write(
            &apps,
            "nodisplay.desktop",
            "[Desktop Entry]\nType=Application\nName=NoDisplay\nExec=x\nNoDisplay=true\n",
        );
        write(&apps, "broken.desktop", "not a desktop file");
        write(
            &apps,
            "noexec.desktop",
            "[Desktop Entry]\nType=Application\nName=NoExec\n",
        );
        write(
            &apps,
            "link.desktop",
            "[Desktop Entry]\nType=Link\nName=Link\nExec=x\n",
        );

        let index = Index::scan_in(std::slice::from_ref(&apps));
        assert_eq!(index.apps.len(), 1);
        let app = &index.apps[0];
        assert_eq!(app.name, "Console");
        assert_eq!(app.generic.as_deref(), Some("Terminal emulator"));
        assert_eq!(app.icon.as_deref(), Some("terminal"));
        assert_eq!(app.exec, Exec(vec!["kgx".to_owned()]));
    }

    #[test]
    fn earlier_dir_shadows_and_hidden_masks() {
        let user = tempfile::tempdir().unwrap();
        let system = tempfile::tempdir().unwrap();
        let body = |name: &str| format!("[Desktop Entry]\nType=Application\nName={name}\nExec=x\n");
        let a = user.path().join("applications");
        let b = system.path().join("applications");
        fs::create_dir(&a).unwrap();
        fs::create_dir(&b).unwrap();
        write(&a, "app.desktop", &body("User App"));
        write(&b, "app.desktop", &body("System App"));
        write(
            &a,
            "masked.desktop",
            "[Desktop Entry]\nType=Application\nName=Masked\nExec=x\nHidden=true\n",
        );
        write(&b, "masked.desktop", &body("Still There"));

        let index = Index::scan_in(&[a, b]);
        assert_eq!(index.apps.len(), 1);
        // Both files claim "app.desktop"/"masked.desktop" ids; the user's
        // visible file wins and the hidden file deletes the id entirely.
        assert_eq!(index.apps[0].name, "User App");
    }

    #[test]
    fn exec_strips_field_codes_and_keeps_quoting() {
        let dir = tempfile::tempdir().unwrap();
        let apps = dir.path().join("applications");
        fs::create_dir(&apps).unwrap();
        write(
            &apps,
            "exec.desktop",
            "[Desktop Entry]\nType=Application\nName=E\n\
             Exec=run --name \"two words\" %f %u\n",
        );
        let index = Index::scan_in(&[apps]);
        let exec = &index.apps[0].exec.0;
        // Quoting kept "two words" as one arg; the field codes went away.
        assert_eq!(exec[..], ["run", "--name", "two words"]);
    }

    #[test]
    fn actions_are_parsed() {
        let dir = tempfile::tempdir().unwrap();
        let apps = dir.path().join("applications");
        fs::create_dir(&apps).unwrap();
        write(
            &apps,
            "act.desktop",
            "[Desktop Entry]\nType=Application\nName=Browser\nExec=browser %U\n\
             Actions=new-window;\n\n\
             [Desktop Action new-window]\nName=New window\nExec=browser --new-window\n",
        );
        let index = Index::scan_in(&[apps]);
        let app = &index.apps[0];
        assert_eq!(app.actions.len(), 1);
        assert_eq!(app.actions[0].name, "New window");
        assert_eq!(app.actions[0].exec.0[0], "browser");
    }

    #[test]
    fn try_exec_missing_binary_drops_entry() {
        let dir = tempfile::tempdir().unwrap();
        let apps = dir.path().join("applications");
        fs::create_dir(&apps).unwrap();
        write(
            &apps,
            "gone.desktop",
            "[Desktop Entry]\nType=Application\nName=Gone\n\
             Exec=x\nTryExec=definitely-not-a-real-binary-triton\n",
        );
        let index = Index::scan_in(&[apps]);
        assert!(index.apps.is_empty());
    }
}
