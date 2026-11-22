//! Rune-style headless dev runner for `triton`: `--snap`, `--tree`, and
//! `--script` backed by `triton-engine`'s [`HeadlessSession`] and offscreen
//! EGL [`HeadlessRenderer`].
//!
//! Bar, OSD, and Wallpaper surfaces render to PNG and semantic
//! `tree_json()` in milliseconds — no Wayland, no compositor, and always the
//! [`BackendMode::Fake`] dev state, so the host `~/.config` and live session
//! are never touched.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use thiserror::Error;
use triton_design::color::Variant;
use triton_engine::{
    AppCommand, EngineError, HeadlessRenderer, HeadlessSession, Rgba, Script, ScriptOutcome,
    ShellApp,
};

use crate::bar::BarClock;
use crate::osd::OsdPayload;
use crate::shell::{SurfaceId, TritonShell};
use crate::state::{BackendMode, BarMode, ShellEvent, SurfaceRole, variant_as_str};

/// Default logical→physical scale for headless rendering (design canvas).
pub const DEFAULT_SCALE: f32 = 1.75;

/// Default virtual settle budget in milliseconds: timers fire at their
/// deadlines and enter morphs complete before output is written.
pub const DEFAULT_SETTLE_MS: u64 = 800;

/// Deterministic wall clock for snapshots (Wed 15 Oct 2025, 09:41 UTC).
const SNAPSHOT_UNIX_SECS: u64 = 1_760_521_260;

/// Flat backdrops behind `--snap` PNGs (matches the showcase renderer).
const BACKDROP_DARK: Rgba = Rgba::new(0.11, 0.12, 0.15, 1.0);
const BACKDROP_LIGHT: Rgba = Rgba::new(0.84, 0.86, 0.90, 1.0);

/// Parsed `--snap`/`--tree`/`--script` dev flags (see [`crate::cli::parse_args`]).
#[derive(Debug, Clone, PartialEq)]
pub struct HeadlessArgs {
    /// `--snap <FILE.png|DIR>`: write offscreen EGL PNG snapshot(s).
    pub snap: Option<PathBuf>,
    /// `--tree <FILE.json|DIR|->`: write semantic tree JSON (`-` = stdout).
    pub tree: Option<PathBuf>,
    /// `--script <FILE.json>`: replay a timestamped input-event script.
    pub script: Option<PathBuf>,
    /// `--surface <bar|osd|wallpaper|all>` (repeatable; default `bar`).
    pub surfaces: Vec<SurfaceRole>,
    /// `--variant <dark|light|both>` (default `dark`).
    pub variants: Vec<Variant>,
    /// `--mode <autohide|pinned|hidden>`: Bar mode during the run.
    pub bar_mode: Option<BarMode>,
    /// `--osd <volume|brightness|mic|layout|workspace|battery>`: canned payload
    /// shown while the `osd` surface renders.
    pub osd: Option<OsdPayload>,
    /// `--reveal`: simulate a hover on the Autohide reveal edge so the Bar
    /// islands are shown in snapshots.
    pub reveal: bool,
    /// `--scale <N>`: logical→physical scale (default [`DEFAULT_SCALE`]).
    pub scale: f32,
    /// `--size <WxH>`: canvas for `Stretch` surfaces like the wallpaper
    /// (default: the fake dev output's logical size).
    pub size: Option<(u32, u32)>,
    /// `--settle <MS>`: virtual settle budget before output
    /// (default [`DEFAULT_SETTLE_MS`]).
    pub settle_ms: u64,
    /// `--reduce-motion`: 120 ms linear fades instead of springs.
    pub reduce_motion: bool,
    /// `--transparent`: keep alpha in PNGs instead of a flat backdrop.
    pub transparent: bool,
}

impl Default for HeadlessArgs {
    fn default() -> Self {
        Self {
            snap: None,
            tree: None,
            script: None,
            surfaces: Vec::new(),
            variants: Vec::new(),
            bar_mode: None,
            osd: None,
            reveal: false,
            scale: DEFAULT_SCALE,
            size: None,
            settle_ms: DEFAULT_SETTLE_MS,
            reduce_motion: false,
            transparent: false,
        }
    }
}

/// Errors produced by the headless dev runner.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum HeadlessError {
    /// Engine-level failure (EGL init, script parse, layout).
    #[error("headless engine error: {0}")]
    Engine(#[from] EngineError),
    /// Filesystem failure while reading `--script` or writing artifacts.
    #[error("headless I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// A requested surface is not mounted (e.g. `bar` under `--mode hidden`).
    #[error("surface '{0}' is not mounted in this shell state")]
    SurfaceNotMounted(&'static str),
}

/// Summary of one headless run: artifacts written and script outcome.
#[derive(Debug, Clone, Default)]
pub struct HeadlessReport {
    /// PNG files written by `--snap`.
    pub snapshots: Vec<PathBuf>,
    /// JSON files written by `--tree` (`PathBuf::from("-")` for stdout).
    pub trees: Vec<PathBuf>,
    /// Input events dispatched by `--script` across all surfaces.
    pub events_dispatched: usize,
    /// Named script marks (`name` → virtual seconds from script start).
    pub marks: HashMap<String, f64>,
}

/// Run the headless dev loop: build a fake-backend [`TritonShell`], apply the
/// requested state, optionally replay `--script`, then write `--snap` PNGs
/// and `--tree` JSON for every `(--surface, --variant)` combination.
pub fn run(args: &HeadlessArgs) -> Result<HeadlessReport, HeadlessError> {
    let script = args
        .script
        .as_deref()
        .map(|path| -> Result<Script, HeadlessError> {
            Ok(Script::from_json(&fs::read_to_string(path)?)?)
        })
        .transpose()?;
    let mut renderer = args
        .snap
        .is_some()
        .then(HeadlessRenderer::new)
        .transpose()?;

    let variants = if args.variants.is_empty() {
        vec![Variant::Dark]
    } else {
        args.variants.clone()
    };
    let surfaces = if args.surfaces.is_empty() {
        vec![SurfaceRole::Bar]
    } else {
        dedup_roles(&args.surfaces)
    };
    let single_target = surfaces.len() == 1 && variants.len() == 1;
    let snap_is_file = has_extension(args.snap.as_deref(), "png");
    let tree_is_file =
        args.tree.as_deref() == Some(Path::new("-")) || has_extension(args.tree.as_deref(), "json");

    let mut report = HeadlessReport::default();

    for &variant in &variants {
        let mut session =
            HeadlessSession::<crate::ShellMessage>::new(session_size(args), args.scale);
        session.set_reduce_motion(args.reduce_motion);

        let mut app = TritonShell::new(BackendMode::Fake);
        app.set_bar_clock(BarClock::fixed(SNAPSHOT_UNIX_SECS, 0, 0, session.now()));

        let settle_until = session.now() + Duration::from_millis(args.settle_ms);
        let mut cmds = setup_events(&mut app, args, variant, &surfaces, session.now());
        // A hover on the reveal edge raises the Autohide islands for snaps.
        if args.reveal
            && surfaces.contains(&SurfaceRole::Bar)
            && let Some((id, _)) = find_surface(&app, SurfaceRole::Bar)
        {
            cmds.extend(session.pointer_enter(&mut app, id, (10.0, 1.0)));
        }
        pump_commands(&mut app, &mut session, cmds, settle_until);
        if session.now() < settle_until {
            session.advance(settle_until - session.now());
        }

        for &role in &surfaces {
            let (id, spec) =
                find_surface(&app, role).ok_or(HeadlessError::SurfaceNotMounted(role.as_str()))?;

            session.render_surface(&mut app, id, spec.sizing);
            if let Some(ref script) = script {
                let mut pending = Vec::new();
                // `view` borrows `&app` while `update` needs `&mut app`; the
                // RefCell borrows never overlap because run_script calls the
                // two closures strictly in alternation.
                let outcome: ScriptOutcome = {
                    let cell = RefCell::new(&mut app);
                    session.run_script(
                        script,
                        |ctx| cell.borrow().view(id, ctx),
                        |msg, now| pending.extend(cell.borrow_mut().update(id, msg, now)),
                    )?
                };
                report.events_dispatched += outcome.events_dispatched;
                report.marks.extend(outcome.marks);
                // Fire any timers the interactions armed, then settle.
                pump_commands(&mut app, &mut session, pending, settle_until);
            }
            session.render_surface(&mut app, id, spec.sizing);

            let stem = format!("{}-{}", role.as_str(), variant_as_str(variant));
            if let (Some(base), Some(r)) = (&args.snap, renderer.as_mut()) {
                let out = artifact_path(base, snap_is_file, single_target, &stem, "png");
                if let Some(parent) = out.parent()
                    && !parent.as_os_str().is_empty()
                {
                    fs::create_dir_all(parent)?;
                }
                r.snap_png_with_backdrop(&mut session, backdrop(args, variant), &out)?;
                report.snapshots.push(out);
            }
            if let Some(base) = &args.tree {
                let json = session.tree_json();
                if base.as_os_str() == "-" {
                    println!("{json}");
                    report.trees.push(PathBuf::from("-"));
                } else {
                    let out = artifact_path(base, tree_is_file, single_target, &stem, "json");
                    if let Some(parent) = out.parent()
                        && !parent.as_os_str().is_empty()
                    {
                        fs::create_dir_all(parent)?;
                    }
                    fs::write(&out, json)?;
                    report.trees.push(out);
                }
            }
        }
    }

    Ok(report)
}

fn dedup_roles(surfaces: &[SurfaceRole]) -> Vec<SurfaceRole> {
    let mut out = Vec::with_capacity(surfaces.len());
    for &role in surfaces {
        if !out.contains(&role) {
            out.push(role);
        }
    }
    out
}

fn has_extension(path: Option<&Path>, ext: &str) -> bool {
    path.is_some_and(|p| p.extension().is_some_and(|e| e == ext))
}

fn artifact_path(
    base: &Path,
    is_file: bool,
    single_target: bool,
    stem: &str,
    ext: &str,
) -> PathBuf {
    if single_target && is_file {
        base.to_path_buf()
    } else {
        base.join(format!("{stem}.{ext}"))
    }
}

fn find_surface(
    app: &TritonShell,
    role: SurfaceRole,
) -> Option<(SurfaceId, triton_engine::SurfaceSpec)> {
    app.surfaces().into_iter().find(|(id, _)| id.role == role)
}

fn session_size(args: &HeadlessArgs) -> (f32, f32) {
    args.size
        .map_or((1920.0, 1080.0), |(w, h)| (w as f32, h as f32))
}

fn backdrop(args: &HeadlessArgs, variant: Variant) -> Rgba {
    if args.transparent {
        Rgba::TRANSPARENT
    } else {
        match variant {
            Variant::Dark => BACKDROP_DARK,
            Variant::Light => BACKDROP_LIGHT,
        }
    }
}

/// Apply the requested shell state (theme variant, reduce-motion, Bar mode,
/// canned OSD payload) and return the resulting [`AppCommand`]s for the
/// timer pump.
fn setup_events(
    app: &mut TritonShell,
    args: &HeadlessArgs,
    variant: Variant,
    surfaces: &[SurfaceRole],
    now: Instant,
) -> Vec<AppCommand<SurfaceId>> {
    let mut cmds = Vec::new();
    if args.reduce_motion {
        cmds.extend(app.handle_event(ShellEvent::SetReduceMotion(true), now));
    }
    cmds.extend(app.handle_event(
        ShellEvent::ApplyTheme {
            scheme: None,
            variant: Some(variant),
            mean_luminance: None,
        },
        now,
    ));
    if let Some(mode) = args.bar_mode {
        cmds.extend(app.handle_event(ShellEvent::SetBarMode(mode), now));
    }
    if surfaces.contains(&SurfaceRole::Osd) {
        let payload = args.osd.clone().unwrap_or(OsdPayload::Volume {
            level: 0.65,
            muted: false,
        });
        cmds.extend(app.handle_event(
            ShellEvent::ShowOsd {
                payload,
                output: None,
            },
            now,
        ));
    }
    cmds
}

/// Drain [`AppCommand`]s into a pending-timer table, then fire timers at their
/// virtual deadlines until none remain before `until`. `ScheduleTimer` re-arms
/// refresh the deadline exactly like the live event loop; `PauseTimer` /
/// `ResumeTimer` carry remaining time so hover-pause semantics survive.
fn pump_commands(
    app: &mut TritonShell,
    session: &mut HeadlessSession<crate::ShellMessage>,
    mut cmds: Vec<AppCommand<SurfaceId>>,
    until: Instant,
) {
    let mut pending: HashMap<u32, Instant> = HashMap::new();
    let mut paused: HashMap<u32, Duration> = HashMap::new();

    loop {
        for cmd in cmds.drain(..) {
            match cmd {
                AppCommand::ScheduleTimer { tag, delay } => {
                    pending.insert(tag, session.now() + delay);
                }
                AppCommand::CancelTimer(tag) => {
                    pending.remove(&tag);
                    paused.remove(&tag);
                }
                AppCommand::PauseTimer(tag) => {
                    if let Some(deadline) = pending.remove(&tag) {
                        paused.insert(tag, deadline.saturating_duration_since(session.now()));
                    }
                }
                AppCommand::ResumeTimer(tag) => {
                    if let Some(remaining) = paused.remove(&tag) {
                        pending.insert(tag, session.now() + remaining);
                    }
                }
                AppCommand::SetReduceMotion(enabled) => session.set_reduce_motion(enabled),
                AppCommand::Quit => return,
                // Redraw/Focus and any future command kinds need no headless
                // handling — the frame is re-rendered explicitly afterwards.
                _ => {}
            }
        }

        let next = pending
            .iter()
            .min_by_key(|(_, deadline)| *deadline)
            .map(|(&tag, &deadline)| (tag, deadline));
        let Some((tag, deadline)) = next else { break };
        if deadline > until {
            break;
        }
        pending.remove(&tag);
        if deadline > session.now() {
            session.advance(deadline - session.now());
        }
        cmds = app.on_timer(tag, session.now());
    }
}
