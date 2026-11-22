//! Query → grouped result rows. One field covers every scope (`Launcher`
//! board): bare text searches apps, windows and commands; `>` searches only
//! commands, `=` only math (evaluated by [`crate::launch::math`], so
//! [`search`] returns no groups for it) and `/` is reserved for files.
//!
//! Ranking is name-prefix > word-start > substring > subsequence, with a small
//! penalty for matching a secondary field (`GenericName`, keywords, app id)
//! instead of the title, and earlier + shorter haystacks winning ties.

use super::Index;

/// Which list a query belongs to, by its leading sigil. The payload is the
/// query with the sigil (and surrounding whitespace) stripped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope<'q> {
    /// Bare query: apps, windows and commands.
    All(&'q str),
    /// `>`: shell command palette entries only.
    Commands(&'q str),
    /// `=`: math only; [`search`] yields nothing, call
    /// [`math`](crate::launch::math) with the raw query.
    Math(&'q str),
    /// `/`: files only; no backend yet.
    Files(&'q str),
}

/// Classify a query by its leading sigil.
pub fn scope(query: &str) -> Scope<'_> {
    let q = query.trim_start();
    if let Some(rest) = q.strip_prefix('>') {
        Scope::Commands(rest.trim_start())
    } else if let Some(rest) = q.strip_prefix('=') {
        Scope::Math(rest.trim_start())
    } else if let Some(rest) = q.strip_prefix('/') {
        Scope::Files(rest.trim_start())
    } else {
        Scope::All(q)
    }
}

/// A result group, in board order: apps first, then windows, then commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    Apps,
    Windows,
    Commands,
}

/// One result row.
#[derive(Debug, Clone)]
pub struct Hit {
    /// The row's title (app name, window title, command title).
    pub title: String,
    /// The second line (generic name, app id, command detail).
    pub supporting: Option<String>,
    /// The right-aligned chip — the workspace name for windows.
    pub tag: Option<String>,
    /// How to activate this hit.
    pub kind: HitKind,
}

/// How a hit activates.
#[derive(Debug, Clone, PartialEq)]
pub enum HitKind {
    /// Launch `index.apps[app]` — or one of its desktop actions.
    App {
        /// Index into [`Index::apps`].
        app: usize,
        /// Index into [`App::actions`](super::App::actions), when the hit is an
        /// action row rather than the app itself. The UI also reads this off
        /// the first app hit to fill the top-hit card's action buttons.
        action: Option<usize>,
    },
    /// Focus an open window from niri's snapshot.
    Window {
        /// niri's window id.
        id: u64,
    },
    /// Run `commands[i]` — the index is into the slice passed to [`search`].
    Command(usize),
    /// An evaluated expression: the board's math card. [`search`] never
    /// produces this — [`math`](crate::launch::math) does the evaluating and
    /// the app wraps the `(expr, result)` pair itself, since the card replaces
    /// the whole result list rather than joining a group.
    Math {
        /// The normalized expression ("1,248 ÷ 4").
        expr: String,
        /// The computed result ("312").
        result: String,
    },
}

/// Ordered groups; empty groups are omitted, so the first hit of the first
/// group is the board's top hit.
#[derive(Debug, Clone, Default)]
pub struct Results {
    /// `(group, hits)` pairs in [`Group`] order.
    pub groups: Vec<(Group, Vec<Hit>)>,
}

/// A window from niri's snapshot, as the caller reports it.
#[derive(Debug, Clone)]
pub struct WindowRef {
    /// niri's window id.
    pub id: u64,
    /// The window title.
    pub title: String,
    /// `app_id`, matched as a secondary field and shown as the second line.
    pub app_id: Option<String>,
    /// Workspace name, shown as the row's tag chip.
    pub workspace: Option<String>,
}

/// A shell command palette entry the caller provides (Triton's own actions).
#[derive(Debug, Clone, Copy)]
pub struct CommandRef<'a> {
    /// The row's title ("Toggle dark style").
    pub title: &'a str,
    /// The second line ("Currently on · follows schedule").
    pub supporting: Option<&'a str>,
    /// The caller's own id for activation; not matched against.
    pub id: &'a str,
}

// Per-group caps from the board: ~6 apps, ~4 windows and commands.
const APP_CAP: usize = 6;
const WINDOW_CAP: usize = 4;
const COMMAND_CAP: usize = 4;

/// Fuzzy-search the index under the query's [`Scope`]. An empty `All` query
/// still returns the caps' worth of each group (apps arrive name-sorted from
/// [`Index::scan`]), so the panel has content before the first keystroke.
pub fn search(
    index: &Index,
    windows: &[WindowRef],
    commands: &[CommandRef<'_>],
    query: &str,
) -> Results {
    let mut groups = Vec::new();
    match scope(query) {
        Scope::All(q) => {
            push(&mut groups, Group::Apps, app_hits(index, q));
            push(&mut groups, Group::Windows, window_hits(windows, q));
            push(&mut groups, Group::Commands, command_hits(commands, q));
        }
        Scope::Commands(q) => push(&mut groups, Group::Commands, command_hits(commands, q)),
        // Math is evaluated by `launch::math`; files have no backend yet.
        Scope::Math(_) | Scope::Files(_) => {}
    }
    Results { groups }
}

fn push(groups: &mut Vec<(Group, Vec<Hit>)>, group: Group, hits: Vec<Hit>) {
    if !hits.is_empty() {
        groups.push((group, hits));
    }
}

/// A match quality: lower is better, field order is the ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Score {
    /// 0 prefix, 1 word-start, 2 substring, 3 subsequence.
    tier: u8,
    /// Where the match begins.
    pos: u32,
    /// Haystack length: shorter candidates win ties.
    len: u32,
}

impl Score {
    /// Degrade one tier: a `GenericName` match is worth less than a `Name` one.
    fn secondary(self) -> Self {
        Self {
            tier: self.tier.saturating_add(1),
            ..self
        }
    }
}

/// Score `needle` against `hay`, case-insensitively. `None` when the needle's
/// chars are not even a subsequence.
fn score(needle: &str, hay: &str) -> Option<Score> {
    let n: Vec<char> = needle.chars().flat_map(char::to_lowercase).collect();
    let h: Vec<char> = hay.chars().flat_map(char::to_lowercase).collect();
    let len = h.len() as u32;
    if n.is_empty() {
        // Everything ties on an empty query, keeping the caller's order.
        return Some(Score {
            tier: 0,
            pos: 0,
            len: 0,
        });
    }
    if h.starts_with(&n) {
        return Some(Score {
            tier: 0,
            pos: 0,
            len,
        });
    }
    // Word-start: a contiguous match right after a non-alphanumeric boundary.
    for (i, w) in h.windows(n.len()).enumerate().skip(1) {
        if w == n.as_slice() && !h[i - 1].is_alphanumeric() {
            return Some(Score {
                tier: 1,
                pos: i as u32,
                len,
            });
        }
    }
    // Substring: contiguous anywhere else. Position 0 was the prefix check.
    if let Some(i) = h.windows(n.len()).position(|w| w == n.as_slice()) {
        return Some(Score {
            tier: 2,
            pos: i as u32,
            len,
        });
    }
    // Subsequence: every char in order, ranked by where the match starts.
    let mut chars = h.iter().enumerate();
    let mut start = 0;
    for (matched, &c) in n.iter().enumerate() {
        loop {
            let (i, &hc) = chars.next()?;
            if hc == c {
                if matched == 0 {
                    start = i as u32;
                }
                break;
            }
        }
    }
    Some(Score {
        tier: 3,
        pos: start,
        len,
    })
}

/// The best score over a title plus secondary fields, which match one tier down.
fn scored(title: &str, secondary: &[&str], needle: &str) -> Option<Score> {
    score(needle, title)
        .into_iter()
        .chain(
            secondary
                .iter()
                .filter_map(|s| score(needle, s).map(Score::secondary)),
        )
        .min()
}

fn app_hits(index: &Index, needle: &str) -> Vec<Hit> {
    let mut hits: Vec<(Score, usize, Hit)> = Vec::new();
    for (i, app) in index.apps.iter().enumerate() {
        let secondary: Vec<&str> = app
            .generic
            .iter()
            .map(String::as_str)
            .chain(app.keywords.iter().map(String::as_str))
            .collect();
        if let Some(s) = scored(&app.name, &secondary, needle) {
            hits.push((
                s,
                i,
                Hit {
                    title: app.name.clone(),
                    supporting: app.generic.clone(),
                    tag: None,
                    kind: HitKind::App {
                        app: i,
                        action: None,
                    },
                },
            ));
        }
        // Actions rank one tier below the app's own name ("new" → New window).
        for (j, action) in app.actions.iter().enumerate() {
            if let Some(s) = score(needle, &action.name).map(Score::secondary) {
                hits.push((
                    s,
                    i,
                    Hit {
                        title: action.name.clone(),
                        supporting: Some(app.name.clone()),
                        tag: None,
                        kind: HitKind::App {
                            app: i,
                            action: Some(j),
                        },
                    },
                ));
            }
        }
    }
    // Stable sort keeps index order on ties; `i` is in the tuple for clarity.
    hits.sort_by_key(|(s, i, _)| (*s, *i));
    hits.truncate(APP_CAP);
    hits.into_iter().map(|(_, _, hit)| hit).collect()
}

fn window_hits(windows: &[WindowRef], needle: &str) -> Vec<Hit> {
    let mut hits: Vec<(Score, usize, Hit)> = windows
        .iter()
        .enumerate()
        .filter_map(|(i, w)| {
            let secondary: Vec<&str> = w.app_id.iter().map(String::as_str).collect();
            let s = scored(&w.title, &secondary, needle)?;
            Some((
                s,
                i,
                Hit {
                    title: w.title.clone(),
                    supporting: w.app_id.clone(),
                    tag: w.workspace.clone(),
                    kind: HitKind::Window { id: w.id },
                },
            ))
        })
        .collect();
    hits.sort_by_key(|(s, i, _)| (*s, *i));
    hits.truncate(WINDOW_CAP);
    hits.into_iter().map(|(_, _, hit)| hit).collect()
}

fn command_hits(commands: &[CommandRef<'_>], needle: &str) -> Vec<Hit> {
    let mut hits: Vec<(Score, usize, Hit)> = commands
        .iter()
        .enumerate()
        .filter_map(|(i, c)| {
            let secondary: Vec<&str> = c.supporting.into_iter().collect();
            let s = scored(c.title, &secondary, needle)?;
            Some((
                s,
                i,
                Hit {
                    title: c.title.to_owned(),
                    supporting: c.supporting.map(str::to_owned),
                    tag: None,
                    kind: HitKind::Command(i),
                },
            ))
        })
        .collect();
    hits.sort_by_key(|(s, i, _)| (*s, *i));
    hits.truncate(COMMAND_CAP);
    hits.into_iter().map(|(_, _, hit)| hit).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::launch::index::{App, AppAction, Exec};

    fn app(name: &str, keywords: &[&str]) -> App {
        App {
            id: format!("{name}.desktop"),
            name: name.to_owned(),
            generic: None,
            exec: Exec(vec![name.to_owned()]),
            icon: None,
            keywords: keywords.iter().map(|s| (*s).to_owned()).collect(),
            actions: Vec::new(),
            terminal: false,
            path: format!("/usr/share/applications/{name}.desktop").into(),
        }
    }

    fn index(names: &[&str]) -> Index {
        Index {
            apps: names.iter().map(|n| app(n, &[])).collect(),
        }
    }

    fn titles(results: &Results) -> Vec<&str> {
        results
            .groups
            .iter()
            .flat_map(|(_, hits)| hits.iter().map(|h| h.title.as_str()))
            .collect()
    }

    #[test]
    fn scope_prefixes() {
        assert_eq!(scope("con"), Scope::All("con"));
        assert_eq!(scope("> dark"), Scope::Commands("dark"));
        assert_eq!(scope("=1+1"), Scope::Math("1+1"));
        assert_eq!(scope("= 1 + 1"), Scope::Math("1 + 1"));
        assert_eq!(scope("/etc"), Scope::Files("etc"));
        assert_eq!(scope("  > x"), Scope::Commands("x"));
    }

    #[test]
    fn prefix_beats_word_start_beats_substring_beats_subsequence() {
        // One candidate per tier: prefix, word-start, mid-word, spread out.
        let index = index(&["Cornfield", "Console", "Recon", "Remote Console"]);
        let results = search(&index, &[], &[], "con");
        assert_eq!(
            titles(&results),
            // Console: prefix. Remote Console: word-start. Recon: substring
            // at pos 2. Cornfield: subsequence c..o..n.
            ["Console", "Remote Console", "Recon", "Cornfield"]
        );
    }

    #[test]
    fn keyword_match_ranks_below_name() {
        let mut index = index(&["Foo", "Baz"]);
        index.apps[1].keywords = vec!["foo".to_owned()];
        // "foo" prefixes Foo's name but only matches Baz's keyword.
        let results = search(&index, &[], &[], "foo");
        assert_eq!(titles(&results), ["Foo", "Baz"]);
    }

    #[test]
    fn empty_query_lists_defaults() {
        let index = index(&["Zed", "Alpha"]);
        let windows = [WindowRef {
            id: 7,
            title: "Doc".to_owned(),
            app_id: Some("app.doc".to_owned()),
            workspace: Some("Work".to_owned()),
        }];
        let results = search(&index, &windows, &[], "");
        let groups: Vec<Group> = results.groups.iter().map(|(g, _)| *g).collect();
        assert_eq!(groups, [Group::Apps, Group::Windows]);
        assert_eq!(index.apps[0].name, "Zed");
        let window = &results.groups[1].1[0];
        assert_eq!(window.tag.as_deref(), Some("Work"));
        assert_eq!(window.kind, HitKind::Window { id: 7 });
    }

    #[test]
    fn commands_scope_lists_only_commands() {
        let commands = [
            CommandRef {
                title: "Toggle dark style",
                supporting: Some("Currently on"),
                id: "dark",
            },
            CommandRef {
                title: "Lock screen",
                supporting: None,
                id: "lock",
            },
        ];
        let results = search(&index(&["Darkroom"]), &[], &commands, "> dark");
        assert_eq!(results.groups.len(), 1);
        assert_eq!(results.groups[0].0, Group::Commands);
        assert_eq!(results.groups[0].1[0].title, "Toggle dark style");
        assert_eq!(results.groups[0].1[0].kind, HitKind::Command(0));
        // Bare "dark" queries still reach apps too.
        let results = search(&index(&["Darkroom"]), &[], &commands, "dark");
        assert_eq!(results.groups[0].0, Group::Apps);
        assert_eq!(results.groups.len(), 2);
    }

    #[test]
    fn caps_limit_group_sizes() {
        let index = index(&["a1", "a2", "a3", "a4", "a5", "a6", "a7", "a8"]);
        let results = search(&index, &[], &[], "a");
        assert_eq!(results.groups[0].1.len(), APP_CAP);
    }

    #[test]
    fn action_hits_launch_the_action() {
        let mut index = index(&["Browser"]);
        index.apps[0].actions = vec![AppAction {
            id: "new-window".to_owned(),
            name: "New window".to_owned(),
            exec: Exec(vec!["browser".to_owned(), "--new-window".to_owned()]),
        }];
        let results = search(&index, &[], &[], "new win");
        let hit = &results.groups[0].1[0];
        assert_eq!(hit.title, "New window");
        assert_eq!(hit.supporting.as_deref(), Some("Browser"));
        assert_eq!(
            hit.kind,
            HitKind::App {
                app: 0,
                action: Some(0)
            }
        );
    }
}
