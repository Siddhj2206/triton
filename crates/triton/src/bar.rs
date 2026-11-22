//! Triton top bar controller, autohide state machine, minute-aligned live clock,
//! and `triton-kit` view composition.
//!
//! Supports all three display modes from `docs/design-spec.md`:
//! - **Autohide:** 2 px top reveal edge per output triggering floating 56 px
//!   Glass islands (offset 8 px from the top edge, dropping in on
//!   `Spring::SPATIAL`) after a 120 ms normal delay (configurable), a 300 ms
//!   delay when the output has a fullscreen window, and a 600 ms hide delay.
//! - **Pinned:** Full-width 56 px Mica strip with `exclusive_zone: 56` and
//!   adaptive bright-wallpaper alpha (`Material::fill_adaptive`).
//! - **Hidden:** Cleanly unmounts bar surfaces.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use triton_design::icon::{Icon, IconStyle};
use triton_design::material::{Material, ShadowToken};
use triton_design::motion::{Animated, Motion, Spring};
use triton_design::shape::{RADII, SIZE, SPACE};
use triton_design::typography::TypeStyle;
use triton_engine::{
    AppCommand, DropShadow, Edges, Element, FontSpec, NodeKey, OutputInfo, Rgba, SubtreeTransform,
    SurfaceAnchor, SurfaceKeyboard, SurfaceLayer, SurfaceSize, SurfaceSpec, Theme, ViewContext,
    box_, icon, row, text,
};
use triton_kit::{
    Badge, ButtonSize, ButtonTone, IconButton, PillStatus, badged, icon_button, island, panel,
    pill, shadow_gutter, status_pill,
};

use crate::shell::{ShellMessage, SurfaceId};
use crate::state::{BarMode, OutputKey, ShellState, StatusSummary, SurfaceRole, WorkspaceInfo};

/// Timer tag for minute-aligned live clock ticks (0% idle CPU between ticks).
pub const TIMER_BAR_CLOCK: u32 = 2;

/// Height in logical pixels of the Autohide top reveal edge.
pub const BAR_REVEAL_EDGE_HEIGHT: f32 = 2.0;

/// Top offset in logical pixels for floating Autohide Glass islands.
pub const BAR_TOP_OFFSET: f32 = 8.0;

/// Default pointer-hover delay before revealing the Autohide bar (`120 ms`).
pub const BAR_REVEAL_DELAY_DEFAULT: Duration = Duration::from_millis(120);

/// Extended pointer-hover delay when the output has a fullscreen window (`300 ms`).
pub const BAR_REVEAL_DELAY_FULLSCREEN: Duration = Duration::from_millis(300);

/// Pointer-leave delay before retracting a revealed Autohide bar (`600 ms`).
pub const BAR_HIDE_DELAY: Duration = Duration::from_millis(600);

/// Stable max extent height for the floating Autohide bar (8 px top offset +
/// 56 px bar height + `ShadowToken::FLOATING` bottom gutter).
pub const BAR_AUTOHIDE_EXTENT_H: u32 = 224;

/// Default logical width used when an output has not reported its logical size.
const DEFAULT_OUTPUT_WIDTH: u32 = 1920;

/// Maximum logical width of the focused window title before horizontal edge fade.
const BAR_TITLE_MAX_W: f32 = 320.0;

/// Edge-fade width on each horizontal edge of the focused window title.
const BAR_TITLE_EDGE_FADE: f32 = 24.0;

/// Settle threshold for the 0..=1 bar reveal animation.
const REVEAL_EPSILON: f32 = 0.001;

/// Base tag for per-output autohide reveal/hide timers (keeps tags disjoint from
/// fixed tags like `TIMER_OSD_HIDE = 1` and `TIMER_BAR_CLOCK = 2`).
const TIMER_BAR_OUTPUT_BASE: u32 = 0x1000;

/// Deterministic timer tag for scheduling Autohide reveal on `output`.
pub const fn bar_reveal_timer_tag(output: OutputKey) -> u32 {
    TIMER_BAR_OUTPUT_BASE + output.0 * 2
}

/// Deterministic timer tag for scheduling Autohide hide on `output`.
pub const fn bar_hide_timer_tag(output: OutputKey) -> u32 {
    TIMER_BAR_OUTPUT_BASE + output.0 * 2 + 1
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BarTimerKind {
    Reveal,
    Hide,
}

const fn decode_bar_timer_tag(tag: u32) -> Option<(OutputKey, BarTimerKind)> {
    if tag < TIMER_BAR_OUTPUT_BASE {
        return None;
    }
    let offset = tag - TIMER_BAR_OUTPUT_BASE;
    let key = OutputKey(offset / 2);
    if key.0 == 0 {
        return None;
    }
    let kind = if offset.is_multiple_of(2) {
        BarTimerKind::Reveal
    } else {
        BarTimerKind::Hide
    };
    Some((key, kind))
}

/// Configurable Autohide timing parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BarConfig {
    /// Hover delay before revealing on a normal desktop (default `120 ms`).
    pub reveal_delay: Duration,
    /// Hover delay before revealing when the output has a fullscreen window (`300 ms`).
    pub fullscreen_reveal_delay: Duration,
    /// Leave delay before retracting a revealed bar (`600 ms`).
    pub hide_delay: Duration,
}

impl Default for BarConfig {
    fn default() -> Self {
        Self {
            reveal_delay: BAR_REVEAL_DELAY_DEFAULT,
            fullscreen_reveal_delay: BAR_REVEAL_DELAY_FULLSCREEN,
            hide_delay: BAR_HIDE_DELAY,
        }
    }
}

/// Deterministic wall-clock source anchored to a monotonic [`Instant`].
///
/// Advances strictly from the `now: Instant` argument so headless tests can
/// verify exact minute-boundary scheduling and live formatting without sleeping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BarClock {
    anchor_unix_secs: u64,
    anchor_subsec_nanos: u32,
    utc_offset_secs: i64,
    anchor: Instant,
}

impl BarClock {
    /// Anchor the clock to the current [`SystemTime`] at `now`.
    pub fn system(now: Instant) -> Self {
        let since_epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO);
        Self {
            anchor_unix_secs: since_epoch.as_secs(),
            anchor_subsec_nanos: since_epoch.subsec_nanos(),
            utc_offset_secs: 0,
            anchor: now,
        }
    }

    /// Construct a deterministic clock anchored at a specific Unix timestamp
    /// (plus `subsec_nanos` and signed `utc_offset_secs`) at `now`.
    pub fn fixed(unix_secs: u64, subsec_nanos: u32, utc_offset_secs: i64, now: Instant) -> Self {
        Self {
            anchor_unix_secs: unix_secs,
            anchor_subsec_nanos: subsec_nanos % 1_000_000_000,
            utc_offset_secs,
            anchor: now,
        }
    }

    /// Return `(local_unix_secs, subsec_nanos)` at `now`.
    fn wall_parts(&self, now: Instant) -> (i64, u32) {
        let elapsed = now.saturating_duration_since(self.anchor);
        let total_nanos = u64::from(self.anchor_subsec_nanos) + u64::from(elapsed.subsec_nanos());
        let carry_secs = total_nanos / 1_000_000_000;
        let subsec_nanos = (total_nanos % 1_000_000_000) as u32;
        let utc_secs = self
            .anchor_unix_secs
            .saturating_add(elapsed.as_secs())
            .saturating_add(carry_secs);
        let local_secs = i64::try_from(utc_secs)
            .unwrap_or(i64::MAX)
            .saturating_add(self.utc_offset_secs);
        (local_secs, subsec_nanos)
    }

    /// Exact duration until the next `:00.000` minute boundary.
    pub fn delay_until_next_minute(&self, now: Instant) -> Duration {
        const NANOS_PER_SEC: u64 = 1_000_000_000;
        const NANOS_PER_MIN: u64 = 60 * NANOS_PER_SEC;

        let (local_secs, subsec_nanos) = self.wall_parts(now);
        let sec_in_min = local_secs.rem_euclid(60) as u64;
        let elapsed_in_min = sec_in_min * NANOS_PER_SEC + u64::from(subsec_nanos);
        let remaining = NANOS_PER_MIN.saturating_sub(elapsed_in_min);
        if remaining == 0 {
            Duration::from_secs(60)
        } else {
            Duration::from_nanos(remaining)
        }
    }

    /// Format `HH:MM` (24-hour) at `now`.
    pub fn format_time(&self, now: Instant) -> String {
        let (local_secs, _) = self.wall_parts(now);
        let secs_in_day = local_secs.rem_euclid(86_400);
        let hour = secs_in_day / 3600;
        let minute = (secs_in_day % 3600) / 60;
        format!("{hour:02}:{minute:02}")
    }

    /// Format short weekday and date (e.g. `"Thu 01 Jan"`) at `now`.
    pub fn format_date(&self, now: Instant) -> String {
        const WEEKDAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
        const MONTHS: [&str; 12] = [
            "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
        ];

        let (local_secs, _) = self.wall_parts(now);
        let days = local_secs.div_euclid(86_400);
        let weekday = WEEKDAYS[days.rem_euclid(7) as usize];
        let (_year, month, day) = civil_from_days(days);
        let month_name = MONTHS[(month.saturating_sub(1) as usize).min(11)];
        format!("{weekday} {day:02} {month_name}")
    }
}

/// Howard Hinnant's civil-from-days algorithm (UTC epoch `1970-01-01` = day 0).
fn civil_from_days(days_since_epoch: i64) -> (i64, u32, u32) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe.cast_signed() + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = y + i64::from(m <= 2);
    (year, m as u32, d as u32)
}

#[derive(Debug, Clone)]
struct OutputBarState {
    pointer_inside: bool,
    revealed: bool,
    reveal_anim: Animated,
}

impl OutputBarState {
    fn new(reduce_motion: bool, now: Instant) -> Self {
        Self {
            pointer_inside: false,
            revealed: false,
            reveal_anim: Animated::new(
                0.0,
                Motion::pick(Spring::SPATIAL, reduce_motion),
                REVEAL_EPSILON,
                now,
            ),
        }
    }
}

/// Per-output Bar state machine, live clock, and `triton-kit` view builder.
#[derive(Debug)]
pub struct BarController {
    config: BarConfig,
    clock: BarClock,
    outputs: HashMap<OutputKey, OutputBarState>,
}

impl BarController {
    /// Create a new controller with default Autohide timings and system clock.
    pub fn new(now: Instant) -> Self {
        Self {
            config: BarConfig::default(),
            clock: BarClock::system(now),
            outputs: HashMap::new(),
        }
    }

    /// Borrow the active [`BarConfig`].
    pub fn config(&self) -> &BarConfig {
        &self.config
    }

    /// Update the normal (non-fullscreen) Autohide reveal delay.
    pub fn set_reveal_delay(&mut self, delay: Duration) {
        self.config.reveal_delay = delay;
    }

    /// Borrow the active [`BarClock`].
    pub fn clock(&self) -> &BarClock {
        &self.clock
    }

    /// Override the [`BarClock`] (for deterministic tests or timezone updates).
    pub fn set_clock(&mut self, clock: BarClock) {
        self.clock = clock;
    }

    /// Schedule the initial minute-aligned clock timer on shell start.
    pub fn on_start(&self, now: Instant) -> Vec<AppCommand<SurfaceId>> {
        vec![AppCommand::ScheduleTimer {
            tag: TIMER_BAR_CLOCK,
            delay: self.clock.delay_until_next_minute(now),
        }]
    }

    /// Reconcile per-output bar state when outputs are added or removed.
    pub fn sync_outputs(
        &mut self,
        active_keys: &HashSet<OutputKey>,
        reduce_motion: bool,
        now: Instant,
    ) -> Vec<AppCommand<SurfaceId>> {
        let mut cmds = Vec::new();
        let removed: Vec<OutputKey> = self
            .outputs
            .keys()
            .copied()
            .filter(|k| !active_keys.contains(k))
            .collect();
        for key in removed {
            self.outputs.remove(&key);
            cmds.push(AppCommand::CancelTimer(bar_reveal_timer_tag(key)));
            cmds.push(AppCommand::CancelTimer(bar_hide_timer_tag(key)));
        }
        for &key in active_keys {
            self.outputs
                .entry(key)
                .or_insert_with(|| OutputBarState::new(reduce_motion, now));
        }
        cmds
    }

    /// Reset transient Autohide timers/state when switching away from Autohide.
    pub fn on_mode_changed(&mut self, mode: BarMode, now: Instant) -> Vec<AppCommand<SurfaceId>> {
        if mode == BarMode::Autohide {
            return Vec::new();
        }
        let mut cmds = Vec::new();
        for (&key, out) in &mut self.outputs {
            out.pointer_inside = false;
            out.revealed = false;
            out.reveal_anim.snap(0.0, now);
            cmds.push(AppCommand::CancelTimer(bar_reveal_timer_tag(key)));
            cmds.push(AppCommand::CancelTimer(bar_hide_timer_tag(key)));
        }
        cmds
    }

    /// Swap all per-output reveal animations between `Spring::SPATIAL` and the
    /// 120 ms reduced-motion fade without jumping.
    pub fn set_reduce_motion(&mut self, enabled: bool, now: Instant) {
        let motion = Motion::pick(Spring::SPATIAL, enabled);
        for out in self.outputs.values_mut() {
            out.reveal_anim.set_motion(motion, now);
        }
    }

    /// Handle pointer entering a Bar surface.
    pub fn on_pointer_enter(
        &mut self,
        id: SurfaceId,
        state: &ShellState,
        output_name: &str,
        now: Instant,
    ) -> Vec<AppCommand<SurfaceId>> {
        if id.role != SurfaceRole::Bar || state.bar_mode != BarMode::Autohide {
            return Vec::new();
        }
        let out = self
            .outputs
            .entry(id.output)
            .or_insert_with(|| OutputBarState::new(state.reduce_motion, now));
        out.pointer_inside = true;

        let mut cmds = vec![AppCommand::CancelTimer(bar_hide_timer_tag(id.output))];
        if out.revealed {
            out.reveal_anim.set_target(1.0, now);
            cmds.push(AppCommand::Redraw(id));
        } else {
            let delay = if output_has_fullscreen(state, output_name) {
                self.config.fullscreen_reveal_delay
            } else {
                self.config.reveal_delay
            };
            cmds.push(AppCommand::ScheduleTimer {
                tag: bar_reveal_timer_tag(id.output),
                delay,
            });
        }
        cmds
    }

    /// Handle pointer leaving a Bar surface.
    pub fn on_pointer_leave(
        &mut self,
        id: SurfaceId,
        state: &ShellState,
        now: Instant,
    ) -> Vec<AppCommand<SurfaceId>> {
        if id.role != SurfaceRole::Bar || state.bar_mode != BarMode::Autohide {
            return Vec::new();
        }
        let out = self
            .outputs
            .entry(id.output)
            .or_insert_with(|| OutputBarState::new(state.reduce_motion, now));
        out.pointer_inside = false;

        let mut cmds = vec![AppCommand::CancelTimer(bar_reveal_timer_tag(id.output))];
        if out.revealed {
            cmds.push(AppCommand::ScheduleTimer {
                tag: bar_hide_timer_tag(id.output),
                delay: self.config.hide_delay,
            });
        }
        cmds
    }

    /// Handle Bar-owned timer tags (`TIMER_BAR_CLOCK` and per-output autohide
    /// reveal/hide timers). Returns `None` if `tag` does not belong to the Bar.
    pub fn on_timer(
        &mut self,
        tag: u32,
        state: &ShellState,
        now: Instant,
    ) -> Option<Vec<AppCommand<SurfaceId>>> {
        if tag == TIMER_BAR_CLOCK {
            let mut cmds = vec![AppCommand::ScheduleTimer {
                tag: TIMER_BAR_CLOCK,
                delay: self.clock.delay_until_next_minute(now),
            }];
            if state.bar_mode != BarMode::Hidden {
                cmds.push(AppCommand::RedrawAll);
            }
            return Some(cmds);
        }

        let (output_key, kind) = decode_bar_timer_tag(tag)?;
        let out = self.outputs.get_mut(&output_key)?;
        if state.bar_mode != BarMode::Autohide {
            return Some(Vec::new());
        }
        let surface_id = SurfaceId {
            role: SurfaceRole::Bar,
            output: output_key,
        };
        match kind {
            BarTimerKind::Reveal => {
                if out.pointer_inside {
                    out.revealed = true;
                    out.reveal_anim.set_target(1.0, now);
                    return Some(vec![AppCommand::Redraw(surface_id)]);
                }
            }
            BarTimerKind::Hide => {
                if !out.pointer_inside {
                    out.revealed = false;
                    out.reveal_anim.set_target(0.0, now);
                    return Some(vec![AppCommand::Redraw(surface_id)]);
                }
            }
        }
        Some(Vec::new())
    }

    /// Returns `true` only while the Bar reveal/retract animation for `output`
    /// is still moving (0% idle CPU when settled).
    pub fn is_animating(&self, output: OutputKey, now: Instant) -> bool {
        self.outputs
            .get(&output)
            .is_some_and(|out| !out.reveal_anim.is_settled(now))
    }

    /// Returns the declarative [`SurfaceSpec`] for `output` in `mode`, or `None`
    /// when `BarMode::Hidden`.
    pub fn surface_spec(mode: BarMode, output: &OutputInfo) -> Option<SurfaceSpec> {
        let width = output.logical_size.map_or(DEFAULT_OUTPUT_WIDTH, |(w, _)| w);
        match mode {
            BarMode::Hidden => None,
            BarMode::Autohide => Some(SurfaceSpec {
                namespace: SurfaceRole::Bar.namespace().to_owned(),
                layer: SurfaceLayer::Top,
                anchor: SurfaceAnchor::TOP | SurfaceAnchor::LEFT | SurfaceAnchor::RIGHT,
                margin: (0, 0, 0, 0),
                keyboard: SurfaceKeyboard::None,
                exclusive_zone: 0,
                sizing: SurfaceSize::StableExtent {
                    width,
                    height: BAR_AUTOHIDE_EXTENT_H,
                },
                shaped_input: true,
                output: Some(output.name.clone()),
            }),
            BarMode::Pinned => Some(SurfaceSpec {
                namespace: SurfaceRole::Bar.namespace().to_owned(),
                layer: SurfaceLayer::Top,
                anchor: SurfaceAnchor::TOP | SurfaceAnchor::LEFT | SurfaceAnchor::RIGHT,
                margin: (0, 0, 0, 0),
                keyboard: SurfaceKeyboard::None,
                exclusive_zone: SIZE.bar_height as i32,
                sizing: SurfaceSize::Fixed(width, SIZE.bar_height as u32),
                shaped_input: false,
                output: Some(output.name.clone()),
            }),
        }
    }

    /// Build the Bar element tree for `id` using `triton-kit` components exclusively.
    pub fn view(
        &self,
        id: SurfaceId,
        output_name: &str,
        state: &ShellState,
        ctx: &mut ViewContext<'_>,
    ) -> Element<ShellMessage> {
        match state.bar_mode {
            BarMode::Pinned => self.view_pinned(output_name, state, ctx),
            BarMode::Autohide | BarMode::Hidden => self.view_autohide(id, output_name, state, ctx),
        }
    }

    fn view_pinned(
        &self,
        output_name: &str,
        state: &ShellState,
        ctx: &mut ViewContext<'_>,
    ) -> Element<ShellMessage> {
        let theme = &state.theme;
        let bright_backdrop = state.wallpaper.is_bright_backdrop();
        let adaptive_fill =
            Material::Mica.fill_adaptive(&theme.scheme, theme.variant, bright_backdrop);

        let workspaces = view_workspaces(output_name, state, ctx);
        let title = view_focused_title(output_name, state);
        let right = self.view_right_cluster(output_name, state, ctx);

        let mut content = row()
            .w_full()
            .items_center()
            .justify_between()
            .gap(SPACE[2])
            .child(workspaces);
        if let Some(title_el) = title {
            content = content.child(title_el);
        }
        content = content.child(right);

        panel(Material::Mica, RADII.tiled, theme)
            .id(NodeKey::named(&format!("bar-pinned-{output_name}")))
            .bg(Rgba::from_color(adaptive_fill))
            .w_full()
            .h(SIZE.bar_height)
            .px(SPACE[3])
            .justify_center()
            .child(content)
    }

    fn view_autohide(
        &self,
        id: SurfaceId,
        output_name: &str,
        state: &ShellState,
        ctx: &mut ViewContext<'_>,
    ) -> Element<ShellMessage> {
        let theme = &state.theme;

        // Always keep the 2 px top reveal edge mounted so pointer hover can
        // trigger reveal while the rest of the surface remains click-through.
        let reveal_edge = box_()
            .id(NodeKey::named(&format!("bar-reveal-edge-{output_name}")))
            .w_full()
            .h(BAR_REVEAL_EDGE_HEIGHT)
            .bg(Rgba::TRANSPARENT)
            .hoverable();

        let (revealed, progress) = self.outputs.get(&id.output).map_or((false, 0.0), |out| {
            (out.revealed, out.reveal_anim.value(ctx.now))
        });

        let mut root = box_().w_full().h_full().child(reveal_edge);

        if revealed || progress > REVEAL_EPSILON {
            let gutter_top = DropShadow::from(ShadowToken::FLOATING).gutter().top;
            let opacity = progress.clamp(0.0, 1.0);
            let translate_y = if state.reduce_motion {
                0.0
            } else {
                -(SIZE.bar_height + BAR_TOP_OFFSET) * (1.0 - progress)
            };

            let left_island = shadow_gutter(
                island(theme)
                    .id(NodeKey::named(&format!("bar-island-left-{output_name}")))
                    .child(view_workspaces(output_name, state, ctx)),
            );

            let right_island = shadow_gutter(
                island(theme)
                    .id(NodeKey::named(&format!("bar-island-right-{output_name}")))
                    .child(self.view_right_cluster(output_name, state, ctx)),
            );

            let mut islands_row = row()
                .id(NodeKey::named(&format!("bar-islands-{output_name}")))
                .absolute()
                .inset_xy(0.0, BAR_TOP_OFFSET - gutter_top)
                .w_full()
                .justify_between()
                .items_center()
                .transform(SubtreeTransform {
                    translate_y,
                    opacity,
                    ..SubtreeTransform::IDENTITY
                })
                .child(left_island);

            if let Some(title_el) = view_focused_title(output_name, state) {
                let center_island = shadow_gutter(
                    island(theme)
                        .id(NodeKey::named(&format!("bar-island-center-{output_name}")))
                        .px(SPACE[3])
                        .child(title_el),
                );
                islands_row = islands_row.child(center_island);
            }

            islands_row = islands_row.child(right_island);
            root = root.child(islands_row);
        }

        root
    }

    fn view_right_cluster(
        &self,
        output_name: &str,
        state: &ShellState,
        ctx: &mut ViewContext<'_>,
    ) -> Element<ShellMessage> {
        let theme = &state.theme;
        let clock_el = view_clock(&self.clock, output_name, theme, ctx.now);
        let status_el = view_status_pill(output_name, &state.status, theme, ctx);
        let bell_el = view_notification_bell(output_name, &state.status, theme, ctx);

        row()
            .gap(SPACE[1])
            .items_center()
            .child(clock_el)
            .child(status_el)
            .child(bell_el)
    }
}

fn output_has_fullscreen(state: &ShellState, output_name: &str) -> bool {
    let Some(ref win) = state.focused_window else {
        return false;
    };
    if !win.is_fullscreen {
        return false;
    }
    match win.output.as_deref() {
        Some(win_out) => win_out == output_name,
        None => state.focused_output.as_deref() == Some(output_name),
    }
}

fn focused_window_for_output<'a>(
    state: &'a ShellState,
    output_name: &str,
) -> Option<&'a crate::state::FocusedWindow> {
    let win = state.focused_window.as_ref()?;
    match win.output.as_deref() {
        Some(win_out) if win_out == output_name => Some(win),
        None if state.focused_output.as_deref() == Some(output_name) => Some(win),
        _ => None,
    }
}

fn view_workspaces(
    output_name: &str,
    state: &ShellState,
    ctx: &mut ViewContext<'_>,
) -> Element<ShellMessage> {
    let theme = &state.theme;
    let mut workspaces: Vec<&WorkspaceInfo> = state
        .workspaces
        .iter()
        .filter(|w| w.output == output_name)
        .collect();
    workspaces.sort_by_key(|w| w.idx);

    let ring = match state.bar_mode {
        BarMode::Pinned => Rgba::from_color(Material::Mica.fill_adaptive(
            &theme.scheme,
            theme.variant,
            state.wallpaper.is_bright_backdrop(),
        )),
        BarMode::Autohide | BarMode::Hidden => theme.material_fill(Material::Glass),
    };

    let mut ws_row = row().gap(SPACE[0]).items_center();
    for ws in workspaces {
        let label = ws.name.clone().unwrap_or_else(|| ws.idx.to_string());
        let ws_id = ws.id;
        let key = NodeKey::named(&format!("bar-ws-{output_name}-{ws_id}"));
        let mut el = pill(key, label, ws.is_active, theme, ctx.anim, move || {
            ShellMessage::FocusWorkspace(ws_id)
        });
        let occupied = ws.active_window_id.is_some();
        if !ws.is_active && !occupied && !ws.is_urgent {
            el = el.opacity(0.6);
        }
        if ws.is_urgent {
            el = badged(el, Badge::Dot(ring), theme);
        }
        ws_row = ws_row.child(el);
    }
    ws_row
}

fn view_focused_title(output_name: &str, state: &ShellState) -> Option<Element<ShellMessage>> {
    let win = focused_window_for_output(state, output_name)?;
    if win.title.is_empty() {
        return None;
    }
    let theme = &state.theme;
    Some(
        box_()
            .id(NodeKey::named(&format!("bar-title-{output_name}")))
            .max_w(BAR_TITLE_MAX_W)
            .px(SPACE[1])
            .clip()
            .edge_fade(Edges::new(
                0.0,
                BAR_TITLE_EDGE_FADE,
                0.0,
                BAR_TITLE_EDGE_FADE,
            ))
            .child(text(
                win.title.clone(),
                TypeStyle::LABEL_L,
                Rgba::from_color(theme.scheme.on_surface),
            )),
    )
}

fn view_clock(
    clock: &BarClock,
    output_name: &str,
    theme: &Theme,
    now: Instant,
) -> Element<ShellMessage> {
    row()
        .id(NodeKey::named(&format!("bar-clock-{output_name}")))
        .px(SPACE[2])
        .gap(SPACE[1])
        .items_center()
        .child(text(
            clock.format_time(now),
            FontSpec::display(22.0).weight(600),
            Rgba::from_color(theme.scheme.on_surface),
        ))
        .child(text(
            clock.format_date(now),
            FontSpec::ui(12.0).weight(500),
            Rgba::from_color(theme.scheme.on_surface_variant),
        ))
}

fn pill_status_for(status: &StatusSummary) -> PillStatus {
    if status
        .battery_percent
        .is_some_and(|pct| pct <= 15 && !status.charging)
    {
        return PillStatus::Urgent;
    }
    if !status.network_connected || status.muted {
        return PillStatus::Attention;
    }
    PillStatus::Primary
}

fn status_fg(status: PillStatus, theme: &Theme) -> Rgba {
    let color = match status {
        PillStatus::Primary => theme.scheme.on_primary_container,
        PillStatus::Attention => theme.scheme.on_tertiary_container,
        PillStatus::Urgent => theme.scheme.on_error_container,
        PillStatus::Neutral | _ => theme.scheme.on_surface,
    };
    Rgba::from_color(color)
}

fn volume_icon(status: &StatusSummary) -> Icon {
    if status.muted || status.volume_percent == 0 {
        Icon::VolumeOff
    } else if status.volume_percent < 40 {
        Icon::VolumeDown
    } else {
        Icon::VolumeUp
    }
}

fn battery_icon(percent: u8, charging: bool) -> Icon {
    if charging {
        return Icon::BatteryChargingFull;
    }
    match percent {
        0..=15 => Icon::BatteryAlert,
        16..=30 => Icon::Battery2Bar,
        31..=50 => Icon::Battery3Bar,
        51..=70 => Icon::Battery4Bar,
        71..=85 => Icon::Battery5Bar,
        86..=95 => Icon::Battery6Bar,
        _ => Icon::BatteryFull,
    }
}

fn view_status_pill(
    output_name: &str,
    status: &StatusSummary,
    theme: &Theme,
    ctx: &mut ViewContext<'_>,
) -> Element<ShellMessage> {
    let tone = pill_status_for(status);
    let fg = status_fg(tone, theme);

    let mut items: Vec<Element<ShellMessage>> = vec![
        icon(
            if status.network_connected {
                Icon::Wifi
            } else {
                Icon::WifiOff
            },
            IconStyle::Outlined,
            SIZE.icon_m,
            fg,
        ),
        row()
            .gap(SPACE[0])
            .items_center()
            .child(icon(
                volume_icon(status),
                IconStyle::Outlined,
                SIZE.icon_m,
                fg,
            ))
            .child(text(
                format!("{}%", status.volume_percent),
                TypeStyle::LABEL_L,
                fg,
            )),
    ];

    if let Some(pct) = status.battery_percent {
        items.push(
            row()
                .gap(SPACE[0])
                .items_center()
                .child(icon(
                    battery_icon(pct, status.charging),
                    IconStyle::Outlined,
                    SIZE.icon_m,
                    fg,
                ))
                .child(text(format!("{pct}%"), TypeStyle::LABEL_L, fg)),
        );
    }

    status_pill(
        NodeKey::named(&format!("bar-status-{output_name}")),
        tone,
        theme,
        ctx.anim,
        Some(Box::new(|| ShellMessage::StatusPillClicked)),
        items,
    )
}

fn view_notification_bell(
    output_name: &str,
    status: &StatusSummary,
    theme: &Theme,
    ctx: &mut ViewContext<'_>,
) -> Element<ShellMessage> {
    let bell = icon_button(
        theme,
        ctx.interaction,
        ctx.anim,
        IconButton {
            key: NodeKey::named(&format!("bar-bell-{output_name}")),
            icon: Icon::Notifications,
            toggled: false,
            tone: ButtonTone::Veil,
            size: ButtonSize::M,
            disabled: false,
            on_click: Some(Box::new(|| ShellMessage::NotificationBellClicked)),
        },
    );
    if status.unread_notifications > 0 {
        badged(bell, Badge::Count(status.unread_notifications), theme)
    } else {
        bell
    }
}
