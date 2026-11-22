//! Wayland + SCTK 0.21 + calloop + EGL multi-surface layer-shell host.
//!
//! Shell applications implement [`ShellApp`] in 100% safe Rust and pass it to
//! [`run`]. This module manages:
//! - Multiple independent `LayerSurface`s (`Layer::Top`, `Layer::Overlay`, `Layer::Background`)
//! - Dynamic content-driven surface sizing (measuring the element tree before committing)
//!   with safe shadow-margin padding so islands and floating panels never clip!
//! - Fractional scaling (`wp_fractional_scale_v1` + `wp_viewport`)
//! - Shaped compositor blur (`ext-background-effect-v1` + `rounded_rect_bands`)
//! - Shaped Wayland input regions (`wl_surface.set_input_region`)
//! - Vsync-paced `wl_surface.frame` rendering with zero idle CPU when animations settle
//! - Optional scheduled timers for caret blinking, demo scripting, or clock ticks.

pub mod egl;
pub mod wl_egl;

use std::collections::HashMap;
use std::fmt;
use std::hash::Hash;
use std::time::{Duration, Instant};

use smithay_client_toolkit::background_effect::{BackgroundEffectHandler, BackgroundEffectState};
use smithay_client_toolkit::compositor::{
    CompositorHandler, CompositorState, FrameCallbackData, Region,
};
use smithay_client_toolkit::dispatch2::Dispatch2;
use smithay_client_toolkit::globals::GlobalData;
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::reexports::calloop::channel as calloop_channel;
use smithay_client_toolkit::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay_client_toolkit::reexports::calloop::{
    EventLoop, LoopHandle, LoopSignal, RegistrationToken,
};
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::reexports::client::globals::registry_queue_init;
use smithay_client_toolkit::reexports::client::protocol::{
    wl_keyboard, wl_output, wl_pointer, wl_seat, wl_surface,
};
use smithay_client_toolkit::reexports::client::{Connection, QueueHandle};
use smithay_client_toolkit::reexports::protocols::ext::background_effect::v1::client::ext_background_effect_surface_v1::ExtBackgroundEffectSurfaceV1;
use smithay_client_toolkit::reexports::protocols::wp::fractional_scale::v1::client::{
    wp_fractional_scale_manager_v1, wp_fractional_scale_v1,
};
use smithay_client_toolkit::reexports::protocols::wp::viewporter::client::{
    wp_viewport, wp_viewporter,
};
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::seat::keyboard::{
    KeyEvent, KeyboardHandler, Keysym, Modifiers, RawModifiers,
};
use smithay_client_toolkit::seat::pointer::{
    AxisScroll, PointerEvent, PointerEventKind, PointerHandler,
};
use smithay_client_toolkit::seat::{Capability, SeatHandler, SeatState};
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shell::wlr_layer::{
    Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
    LayerSurfaceConfigure,
};
use smithay_client_toolkit::{delegate_registry, registry_handlers};
use triton_design::shape::rounded_rect_bands;

use crate::anim::Animator;
use crate::damage::{DamageRect, DamageTracker};
use crate::error::EngineError;
use crate::gpu::{DrawList, Gpu};
use crate::image::ImageStore;
use crate::platform::egl::{Egl, EglSurface};
use crate::platform::wl_egl::{EglWindow, WlEgl};
use crate::text::TextEngine;
use crate::timer::TimerScheduler;
use crate::ui::{
    BlurRect, BuiltFrame, Element, InputEvent, InteractionState, Key, layout_and_paint,
    measure_tree,
};

/// Re-exported layer-shell enums for safe consumer use without importing SCTK directly.
pub use smithay_client_toolkit::shell::wlr_layer::{
    Anchor as SurfaceAnchor, KeyboardInteractivity as SurfaceKeyboard, Layer as SurfaceLayer,
};

/// Surface sizing policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceSize {
    /// Automatically size the Wayland layer surface to match the rendered UI tree's
    /// natural Taffy dimensions (measured before configure and on every frame).
    FitContent {
        /// Optional maximum logical bounds (width, height).
        max_w: Option<u32>,
        max_h: Option<u32>,
    },
    /// Fixed logical size `(width, height)`.
    Fixed(u32, u32),
    /// Stretch to fill the anchored edges (e.g., `TOP | BOTTOM | LEFT | RIGHT` for overlays).
    Stretch,
    /// Fixed maximum extent allocated once for spring-morphing surfaces:
    /// `layer.set_size` and the `wl_egl_window` are created at `(width, height)`
    /// and never resized during shape morphs, keeping `EGL_BUFFER_AGE_KHR` warm.
    /// Pair with `shaped_input` so input regions track the actual painted shape.
    StableExtent {
        /// Maximum logical width in px.
        width: u32,
        /// Maximum logical height in px.
        height: u32,
    },
}

/// Metadata describing a connected display output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputInfo {
    /// Compositor-assigned output name (e.g. `"eDP-1"`, `"DP-1"`).
    pub name: String,
    /// Logical dimensions `(width, height)` in compositor coordinates, if known.
    pub logical_size: Option<(u32, u32)>,
    /// Logical position `(x, y)` in compositor coordinates, if known.
    pub logical_position: Option<(i32, i32)>,
    /// Integer scale factor reported by `wl_output` (at least `1`).
    pub scale_factor: i32,
}

impl OutputInfo {
    /// Construct an [`OutputInfo`] with a name and logical size at origin `(0, 0)` and scale `1`.
    pub fn new(name: impl Into<String>, logical_size: (u32, u32)) -> Self {
        Self {
            name: name.into(),
            logical_size: Some(logical_size),
            logical_position: Some((0, 0)),
            scale_factor: 1,
        }
    }
}

/// Declarative specification of one layer-shell surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurfaceSpec {
    pub namespace: String,
    pub layer: Layer,
    pub anchor: Anchor,
    /// Margins `(top, right, bottom, left)` in logical px.
    pub margin: (i32, i32, i32, i32),
    pub keyboard: KeyboardInteractivity,
    pub exclusive_zone: i32,
    pub sizing: SurfaceSize,
    /// If true, pointer events outside painted input rects pass through to windows below.
    pub shaped_input: bool,
    /// Optional Wayland output name to bind this layer surface to (`None` = compositor default).
    pub output: Option<String>,
}

impl SurfaceSpec {
    /// Floating top-bar island anchored to `anchor` (e.g., `TOP | LEFT` or `TOP | RIGHT`).
    pub fn island(
        namespace: impl Into<String>,
        anchor: Anchor,
        margin: (i32, i32, i32, i32),
    ) -> Self {
        Self {
            namespace: namespace.into(),
            layer: Layer::Top,
            anchor,
            margin,
            keyboard: KeyboardInteractivity::None,
            exclusive_zone: 0,
            sizing: SurfaceSize::FitContent {
                max_w: Some(1400),
                max_h: Some(120),
            },
            shaped_input: true,
            output: None,
        }
    }

    /// Floating top-bar island that spring-morphs its shape inside a stable
    /// max extent (`max_w` x `max_h` logical px): the layer surface and EGL
    /// buffer are allocated once so partial damage stays warm through morphs.
    pub fn morphing_island(
        namespace: impl Into<String>,
        anchor: Anchor,
        margin: (i32, i32, i32, i32),
        max_w: u32,
        max_h: u32,
    ) -> Self {
        Self {
            namespace: namespace.into(),
            layer: Layer::Top,
            anchor,
            margin,
            keyboard: KeyboardInteractivity::None,
            exclusive_zone: 0,
            sizing: SurfaceSize::StableExtent {
                width: max_w,
                height: max_h,
            },
            shaped_input: true,
            output: None,
        }
    }

    /// Floating centered overlay panel (e.g., launcher / quick settings) that uses
    /// `FitContent` or `Stretch` with exclusive keyboard grab.
    pub fn overlay_panel(
        namespace: impl Into<String>,
        anchor: Anchor,
        margin: (i32, i32, i32, i32),
    ) -> Self {
        Self {
            namespace: namespace.into(),
            layer: Layer::Overlay,
            anchor,
            margin,
            keyboard: KeyboardInteractivity::Exclusive,
            exclusive_zone: -1,
            sizing: SurfaceSize::FitContent {
                max_w: Some(1400),
                max_h: Some(900),
            },
            shaped_input: true,
            output: None,
        }
    }

    /// Target a specific Wayland output by name (e.g. `"eDP-1"`).
    #[must_use]
    pub fn on_output(mut self, output: impl Into<String>) -> Self {
        self.output = Some(output.into());
        self
    }
}

/// Per-surface frame timing summary collected by the runtime.
#[derive(Debug, Clone)]
pub struct SurfaceMetrics {
    pub frames: u64,
    pub logical_size: (u32, u32),
    pub buffer_size: (u32, u32),
    pub scale: f32,
    pub interval_ms_mean: f64,
    pub interval_ms_p50: f64,
    pub interval_ms_p95: f64,
    pub interval_ms_max: f64,
}

/// Context passed into [`ShellApp::view`].
pub struct ViewContext<'a> {
    pub now: Instant,
    pub logical_size: (u32, u32),
    pub scale: f32,
    pub interaction: &'a InteractionState,
    pub text: &'a mut TextEngine,
    pub anim: &'a mut Animator,
    /// CPU-side image registry; views can inspect [`ImageStore::dimensions`]
    /// to size elements from source pixel dimensions.
    pub images: &'a ImageStore,
}

impl fmt::Debug for ViewContext<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ViewContext")
            .field("logical_size", &self.logical_size)
            .field("scale", &self.scale)
            .finish()
    }
}

/// Commands an app may return from [`ShellApp::update`] or [`ShellApp::on_timer`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum AppCommand<Id> {
    /// Request a redraw on a specific surface.
    Redraw(Id),
    /// Request a redraw on all live surfaces.
    RedrawAll,
    /// Focus a widget key on a specific surface.
    Focus(Id, crate::ui::NodeKey),
    /// Schedule a one-shot timer callback after `delay` with caller-defined `tag`.
    /// Re-scheduling an already-active `tag` refreshes its deadline.
    ScheduleTimer { tag: u32, delay: Duration },
    /// Cancel the timer scheduled for `tag` (no-op if absent).
    CancelTimer(u32),
    /// Pause the timer for `tag`, freezing its remaining duration (e.g. hover).
    PauseTimer(u32),
    /// Resume a paused timer for `tag` at `now + remaining`.
    ResumeTimer(u32),
    /// Toggle global reduced-motion mode (swaps springs for 120 ms fades and disables shakes).
    SetReduceMotion(bool),
    /// Exit the event loop cleanly.
    Quit,
}

/// Application trait implemented by Triton shell surfaces in 100% safe Rust.
pub trait ShellApp: 'static {
    type SurfaceId: Copy + Eq + Hash + Send + Sync + fmt::Debug + 'static;
    type Message: fmt::Debug + 'static;

    /// Declare all surfaces that should currently exist. Adding an ID creates the
    /// layer surface; removing an ID tears it down cleanly.
    fn surfaces(&self) -> Vec<(Self::SurfaceId, SurfaceSpec)>;

    /// Build the declarative UI tree for `id`.
    fn view(&self, id: Self::SurfaceId, ctx: &mut ViewContext<'_>) -> Element<Self::Message>;

    /// Handle a message emitted by UI interaction on `id`.
    fn update(
        &mut self,
        id: Self::SurfaceId,
        msg: Self::Message,
        now: Instant,
    ) -> Vec<AppCommand<Self::SurfaceId>>;

    /// Return `true` while `id` has an active spring/animation requiring vsync frames.
    /// When `false`, the engine stops requesting `wl_surface.frame` callbacks (0% idle CPU).
    fn is_animating(&self, id: Self::SurfaceId, now: Instant) -> bool;

    /// Optional initial commands executed right after the event loop starts.
    fn on_start(&mut self, _now: Instant) -> Vec<AppCommand<Self::SurfaceId>> {
        Vec::new()
    }

    /// Optional callback invoked on startup with the initial output inventory
    /// and whenever Wayland outputs are added, updated, or removed.
    fn on_outputs_changed(
        &mut self,
        _outputs: &[OutputInfo],
        _now: Instant,
    ) -> Vec<AppCommand<Self::SurfaceId>> {
        Vec::new()
    }

    /// Optional callback invoked when the pointer enters surface `id` at
    /// logical surface coordinates `pos`.
    fn on_pointer_enter(
        &mut self,
        _id: Self::SurfaceId,
        _pos: (f32, f32),
        _now: Instant,
    ) -> Vec<AppCommand<Self::SurfaceId>> {
        Vec::new()
    }

    /// Optional callback invoked when the pointer leaves surface `id`.
    fn on_pointer_leave(
        &mut self,
        _id: Self::SurfaceId,
        _now: Instant,
    ) -> Vec<AppCommand<Self::SurfaceId>> {
        Vec::new()
    }

    /// Optional callback for timers scheduled via [`AppCommand::ScheduleTimer`].
    fn on_timer(&mut self, _tag: u32, _now: Instant) -> Vec<AppCommand<Self::SurfaceId>> {
        Vec::new()
    }

    /// Optional hook to insert, update, or remove images in the engine's
    /// [`ImageStore`] (wallpapers, album art, avatars, screencopy frames).
    ///
    /// Called after `on_start`, after `update`, after `on_timer`, and before
    /// every `view`, giving apps `&mut self` + `&mut ImageStore` without
    /// `RefCell`. Queued uploads/deletions are drained to the GPU on the next
    /// rendered frame.
    fn sync_images(&mut self, _images: &mut ImageStore) {}

    /// Called once right before the event loop exits, with per-surface metrics.
    fn on_exit(&mut self, _metrics: &HashMap<Self::SurfaceId, SurfaceMetrics>) {}
}

/// Thread-safe sender for pushing typed external events into a running
/// [`ShellApp`] event loop (or a [`crate::HeadlessSession`]) without polling.
pub struct EventSender<E> {
    tx: calloop_channel::Sender<E>,
}

impl<E> Clone for EventSender<E> {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
        }
    }
}

impl<E> fmt::Debug for EventSender<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EventSender").finish_non_exhaustive()
    }
}

impl<E> EventSender<E> {
    /// Enqueue `event` and immediately wake the receiving `calloop` event loop.
    pub fn send(&self, event: E) -> Result<(), EngineError> {
        self.tx
            .send(event)
            .map_err(|e| EngineError::EventLoop(format!("external event send failed: {e}")))
    }
}

/// Receiving end of a `calloop`-backed external event channel created by [`event_channel`].
///
/// Pass to [`run_with_events`] for live Wayland execution, or drain
/// deterministically inside [`crate::HeadlessSession::drain_events`].
pub struct EventReceiver<E> {
    rx: calloop_channel::Channel<E>,
}

impl<E> fmt::Debug for EventReceiver<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EventReceiver").finish_non_exhaustive()
    }
}

impl<E> EventReceiver<E> {
    /// Non-blocking receive of the next queued external event, if any.
    pub fn try_recv(&self) -> Option<E> {
        self.rx.try_recv().ok()
    }

    /// Drain all currently queued external events in FIFO order without blocking.
    pub fn drain(&self) -> Vec<E> {
        let mut out = Vec::new();
        while let Some(ev) = self.try_recv() {
            out.push(ev);
        }
        out
    }
}

/// Create a `calloop`-backed external event channel `(EventSender, EventReceiver)`.
///
/// Background threads (IPC streams, Unix socket servers, async image decoders)
/// clone [`EventSender`] to wake the shell's `calloop` event loop with 0% idle CPU.
pub fn event_channel<E: Send + 'static>() -> (EventSender<E>, EventReceiver<E>) {
    let (tx, rx) = calloop_channel::channel();
    (EventSender { tx }, EventReceiver { rx })
}

/// Run a [`ShellApp`] on the Wayland + calloop + EGL/GLES3 runtime.
pub fn run<A: ShellApp>(app: A) -> Result<(), EngineError> {
    run_inner(
        app,
        None::<(
            EventReceiver<()>,
            fn(&mut A, (), Instant) -> Vec<AppCommand<A::SurfaceId>>,
        )>,
    )
}

/// Run a [`ShellApp`] on the Wayland + calloop + EGL/GLES3 runtime with a
/// `calloop`-backed [`EventReceiver`] delivering typed events from background threads.
pub fn run_with_events<A, E, F>(
    app: A,
    events: EventReceiver<E>,
    on_event: F,
) -> Result<(), EngineError>
where
    A: ShellApp,
    E: Send + 'static,
    F: FnMut(&mut A, E, Instant) -> Vec<AppCommand<A::SurfaceId>> + 'static,
{
    run_inner(app, Some((events, on_event)))
}

fn run_inner<A, E, F>(
    app: A,
    external_events: Option<(EventReceiver<E>, F)>,
) -> Result<(), EngineError>
where
    A: ShellApp,
    E: Send + 'static,
    F: FnMut(&mut A, E, Instant) -> Vec<AppCommand<A::SurfaceId>> + 'static,
{
    let conn = Connection::connect_to_env()
        .map_err(|e| EngineError::Wayland(format!("connect_to_env: {e}")))?;
    let (globals, mut event_queue) = registry_queue_init::<Runtime<A>>(&conn)
        .map_err(|e| EngineError::Wayland(format!("registry_queue_init: {e}")))?;
    let qh = event_queue.handle();

    let mut event_loop: EventLoop<'_, Runtime<A>> = EventLoop::try_new()
        .map_err(|e| EngineError::EventLoop(format!("EventLoop::try_new: {e}")))?;
    let loop_handle = event_loop.handle();

    let mut runtime = Runtime::new(
        app,
        &conn,
        &globals,
        &qh,
        loop_handle.clone(),
        event_loop.get_signal(),
    )?;

    // Roundtrip to populate initial wl_output / xdg_output metadata before
    // announcing outputs and syncing initial surfaces.
    let _ = event_queue.roundtrip(&mut runtime);
    let _ = event_queue.roundtrip(&mut runtime);

    WaylandSource::new(conn.clone(), event_queue)
        .insert(loop_handle.clone())
        .map_err(|e| EngineError::EventLoop(format!("insert WaylandSource: {e}")))?;

    if let Some((events, mut on_event)) = external_events {
        let qh_ev = qh.clone();
        loop_handle
            .insert_source(events.rx, move |event, (), rt: &mut Runtime<A>| {
                if let calloop_channel::Event::Msg(ev) = event {
                    let cmds = on_event(&mut rt.app, ev, Instant::now());
                    rt.sync_app_images();
                    if let Err(e) = rt.sync_surfaces(&qh_ev) {
                        tracing::error!(%e, "sync_surfaces after external event failed");
                    }
                    rt.apply_commands(cmds, &qh_ev);
                }
            })
            .map_err(|e| EngineError::EventLoop(format!("insert EventReceiver: {e}")))?;
    }

    runtime.started = true;
    runtime.notify_outputs(&qh, true);

    let start_cmds = runtime.app.on_start(Instant::now());
    runtime.sync_app_images();
    runtime.sync_surfaces(&qh)?;
    runtime.apply_commands(start_cmds, &qh);

    event_loop
        .run(None, &mut runtime, |_| {})
        .map_err(|e| EngineError::EventLoop(format!("event_loop.run: {e}")))?;

    let metrics = runtime.collect_metrics();
    runtime.app.on_exit(&metrics);
    runtime.destroy_all_surfaces();
    Ok(())
}

// ── Internal multi-surface Wayland/EGL runtime ──────────────────────────────

struct SurfaceHost<A: ShellApp> {
    spec: SurfaceSpec,
    layer: LayerSurface,
    window: EglWindow,
    egl_surface: EglSurface,
    viewport: Option<wp_viewport::WpViewport>,
    _fs: Option<wp_fractional_scale_v1::WpFractionalScaleV1>,
    effect: Option<ExtBackgroundEffectSurfaceV1>,
    logical: (u32, u32),
    buffer: (u32, u32),
    scale: f32,
    configured: bool,
    frame_pending: bool,
    needs_redraw: bool,
    interaction: InteractionState,
    animator: Animator,
    built: BuiltFrame<A::Message>,
    last_blur: Vec<BlurRect>,
    last_input: Vec<BlurRect>,
    /// Intervals between consecutive animation frames (ms).
    intervals: Vec<f64>,
    last_frame: Option<Instant>,
    frames: u64,
    /// Buffer-age damage history for partial-damage swaps.
    damage: DamageTracker,
    /// Previous frame's draw list for `DrawList::diff_bounds` content diffs.
    prev_draw_list: DrawList,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SurfaceTag<Id>(Id);

struct Runtime<A: ShellApp> {
    app: A,
    registry_state: RegistryState,
    output_state: OutputState,
    seat_state: SeatState,
    compositor: CompositorState,
    layer_shell: LayerShell,
    effect_state: BackgroundEffectState,
    fs_manager: Option<wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1>,
    viewporter: Option<wp_viewporter::WpViewporter>,

    egl: Egl,
    wl_egl: WlEgl,
    gpu: Option<Gpu>,
    text: TextEngine,
    images: ImageStore,
    draw_list: DrawList,

    hosts: HashMap<A::SurfaceId, SurfaceHost<A>>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    pointer: Option<wl_pointer::WlPointer>,
    keyboard_surface: Option<A::SurfaceId>,
    modifiers: Modifiers,
    loop_handle: LoopHandle<'static, Runtime<A>>,
    signal: LoopSignal,
    /// Central deadline scheduler for `AppCommand::ScheduleTimer` tags.
    timers: TimerScheduler<u32>,
    /// The single armed calloop timer (earliest deadline), if any.
    timer_token: Option<RegistrationToken>,
    /// Global reduced-motion flag forwarded to every surface's `Animator`.
    reduce_motion: bool,
    /// True once initial startup roundtrips complete and `on_outputs_changed` can fire.
    started: bool,
    /// Last output inventory delivered to `app.on_outputs_changed`.
    last_outputs: Vec<OutputInfo>,
}

impl<A: ShellApp> Runtime<A> {
    fn new(
        app: A,
        conn: &Connection,
        globals: &wayland_client::globals::GlobalList,
        qh: &QueueHandle<Runtime<A>>,
        loop_handle: LoopHandle<'static, Runtime<A>>,
        signal: LoopSignal,
    ) -> Result<Self, EngineError> {
        let compositor = CompositorState::bind(globals, qh)
            .map_err(|e| EngineError::Wayland(format!("wl_compositor: {e}")))?;
        let layer_shell = LayerShell::bind(globals, qh)
            .map_err(|e| EngineError::Wayland(format!("zwlr_layer_shell_v1: {e}")))?;
        let effect_state = BackgroundEffectState::new(globals, qh);
        let fs_manager = globals
            .bind::<wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1, _, _>(
                qh,
                1..=1,
                GlobalData,
            )
            .ok();
        let viewporter = globals
            .bind::<wp_viewporter::WpViewporter, _, _>(qh, 1..=1, GlobalData)
            .ok();

        tracing::info!(
            fractional_scale = fs_manager.is_some(),
            viewporter = viewporter.is_some(),
            bg_effect = effect_state.ext_background_effect_manager_v1().is_ok(),
            "bound Wayland globals"
        );

        let egl = Egl::from_connection(conn)?;
        let wl_egl = WlEgl::load()?;
        let text = TextEngine::new();

        Ok(Self {
            app,
            registry_state: RegistryState::new(globals),
            output_state: OutputState::new(globals, qh),
            seat_state: SeatState::new(globals, qh),
            compositor,
            layer_shell,
            effect_state,
            fs_manager,
            viewporter,
            egl,
            wl_egl,
            gpu: None,
            text,
            images: ImageStore::new(),
            draw_list: DrawList::default(),
            hosts: HashMap::new(),
            keyboard: None,
            pointer: None,
            keyboard_surface: None,
            modifiers: Modifiers::default(),
            loop_handle,
            signal,
            timers: TimerScheduler::new(),
            timer_token: None,
            reduce_motion: false,
            started: false,
            last_outputs: Vec::new(),
        })
    }

    fn collect_outputs(&self) -> Vec<OutputInfo> {
        let mut out: Vec<OutputInfo> = self
            .output_state
            .outputs()
            .filter_map(|wl_out| {
                let info = self.output_state.info(&wl_out)?;
                let scale_factor = info.scale_factor.max(1);
                let name = info
                    .name
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| format!("output-{}", info.id));
                let logical_size = info
                    .logical_size
                    .and_then(|(w, h)| {
                        (w > 0 && h > 0).then_some((
                            u32::try_from(w).unwrap_or(1),
                            u32::try_from(h).unwrap_or(1),
                        ))
                    })
                    .or_else(|| {
                        info.modes.iter().find(|m| m.current).and_then(|m| {
                            let (w, h) = m.dimensions;
                            (w > 0 && h > 0).then_some((
                                u32::try_from(w / scale_factor).unwrap_or(1).max(1),
                                u32::try_from(h / scale_factor).unwrap_or(1).max(1),
                            ))
                        })
                    });
                Some(OutputInfo {
                    name,
                    logical_size,
                    logical_position: info.logical_position,
                    scale_factor,
                })
            })
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    fn find_wl_output(&self, name: &str) -> Option<wl_output::WlOutput> {
        self.output_state.outputs().find(|wl_out| {
            self.output_state
                .info(wl_out)
                .and_then(|info| info.name)
                .as_deref()
                == Some(name)
        })
    }

    fn notify_outputs(&mut self, qh: &QueueHandle<Self>, force: bool) {
        let outputs = self.collect_outputs();
        if !force && outputs == self.last_outputs {
            return;
        }
        self.last_outputs.clone_from(&outputs);
        let cmds = self.app.on_outputs_changed(&outputs, Instant::now());
        self.sync_app_images();
        if let Err(e) = self.sync_surfaces(qh) {
            tracing::error!(%e, "sync_surfaces after output change failed");
        }
        self.apply_commands(cmds, qh);
    }

    fn destroy_surface(&mut self, id: A::SurfaceId) {
        if let Some(host) = self.hosts.remove(&id) {
            let _ = self.egl.make_current(None);
            self.egl.destroy_surface(host.egl_surface);
            drop(host);
        }
        if self.keyboard_surface == Some(id) {
            self.keyboard_surface = None;
        }
    }

    fn sync_surfaces(&mut self, qh: &QueueHandle<Self>) -> Result<(), EngineError> {
        let desired = self.app.surfaces();
        let desired_ids: Vec<A::SurfaceId> = desired.iter().map(|(id, _)| *id).collect();

        // Tear down removed surfaces cleanly (destroy EGLSurface before EglWindow).
        let to_remove: Vec<A::SurfaceId> = self
            .hosts
            .keys()
            .copied()
            .filter(|id| !desired_ids.contains(id))
            .collect();
        for id in to_remove {
            self.destroy_surface(id);
        }

        // Create missing surfaces or update changed specs.
        for (id, spec) in desired {
            let needs_recreate = self.hosts.get(&id).is_some_and(|host| {
                host.spec.output != spec.output
                    || host.spec.layer != spec.layer
                    || host.spec.namespace != spec.namespace
            });
            if needs_recreate {
                self.destroy_surface(id);
            }
            if let Some(host) = self.hosts.get_mut(&id) {
                if host.spec != spec {
                    host.layer.set_anchor(spec.anchor);
                    host.layer.set_margin(
                        spec.margin.0,
                        spec.margin.1,
                        spec.margin.2,
                        spec.margin.3,
                    );
                    host.layer.set_keyboard_interactivity(spec.keyboard);
                    host.layer.set_exclusive_zone(spec.exclusive_zone);
                    if host.spec.shaped_input != spec.shaped_input {
                        host.last_input.clear();
                    }
                    host.spec = spec;
                    host.damage.mark_full();
                    host.needs_redraw = true;
                    queue_frame(host, qh);
                }
            } else {
                self.create_surface(id, spec, qh)?;
            }
        }
        Ok(())
    }

    fn create_surface(
        &mut self,
        id: A::SurfaceId,
        spec: SurfaceSpec,
        qh: &QueueHandle<Self>,
    ) -> Result<(), EngineError> {
        let initial_logical = self.compute_initial_size(id, &spec);
        let wl = self.compositor.create_surface(qh);
        let target_output = spec.output.as_deref().and_then(|name| {
            let found = self.find_wl_output(name);
            if found.is_none() {
                tracing::warn!(
                    output = name,
                    ?id,
                    "requested Wayland output not found; falling back to compositor default"
                );
            }
            found
        });
        let layer = self.layer_shell.create_layer_surface(
            qh,
            wl,
            spec.layer,
            Some(spec.namespace.clone()),
            target_output.as_ref(),
        );
        layer.set_anchor(spec.anchor);
        layer.set_margin(spec.margin.0, spec.margin.1, spec.margin.2, spec.margin.3);
        layer.set_keyboard_interactivity(spec.keyboard);
        layer.set_exclusive_zone(spec.exclusive_zone);

        match spec.sizing {
            SurfaceSize::Stretch => {
                // Anchored edges determine size; 0 tells the compositor to fill.
                layer.set_size(0, 0);
            }
            SurfaceSize::Fixed(w, h) => {
                layer.set_size(w.max(1), h.max(1));
            }
            SurfaceSize::FitContent { .. } => {
                layer.set_size(initial_logical.0.max(1), initial_logical.1.max(1));
            }
            SurfaceSize::StableExtent { width, height } => {
                // Allocated once: morph animations never resize the EGL buffer.
                layer.set_size(width.max(1), height.max(1));
            }
        }

        let window =
            self.wl_egl
                .create_window(layer.wl_surface(), initial_logical.0, initial_logical.1)?;
        let egl_surface = self.egl.create_surface(&window)?;
        let tag = SurfaceTag(id);
        let fs = self
            .fs_manager
            .as_ref()
            .map(|m| m.get_fractional_scale(layer.wl_surface(), qh, tag));
        let viewport = self
            .viewporter
            .as_ref()
            .map(|v| v.get_viewport(layer.wl_surface(), qh, tag));
        let effect = self
            .effect_state
            .get_background_effect(layer.wl_surface(), qh)
            .ok();

        // Initialize the shared GLES3 renderer once the first EGL surface exists.
        if self.gpu.is_none() {
            self.egl.make_current(Some(egl_surface))?;
            self.egl.set_swap_interval(0);
            self.gpu = Some(Gpu::new(self.egl.glow())?);
        }

        layer.commit();

        self.hosts.insert(
            id,
            SurfaceHost {
                spec,
                layer,
                window,
                egl_surface,
                viewport,
                _fs: fs,
                effect,
                logical: initial_logical,
                buffer: initial_logical,
                scale: 1.0,
                configured: false,
                frame_pending: false,
                needs_redraw: true,
                interaction: InteractionState {
                    caps_lock: self.modifiers.caps_lock,
                    ..InteractionState::default()
                },
                animator: Animator::new(),
                built: BuiltFrame::default(),
                last_blur: Vec::new(),
                last_input: Vec::new(),
                intervals: Vec::new(),
                last_frame: None,
                frames: 0,
                damage: {
                    let mut d = DamageTracker::new();
                    d.mark_full();
                    d
                },
                prev_draw_list: DrawList::default(),
            },
        );
        Ok(())
    }

    /// Pre-measure content-sized surfaces before initial `layer.commit()` so
    /// the compositor configures the exact final width/height on frame 1 (zero
    /// 1-frame flash or clipping!).
    fn compute_initial_size(&mut self, id: A::SurfaceId, spec: &SurfaceSpec) -> (u32, u32) {
        match spec.sizing {
            SurfaceSize::Fixed(w, h)
            | SurfaceSize::StableExtent {
                width: w,
                height: h,
            } => (w.max(1), h.max(1)),
            SurfaceSize::Stretch => (640, 480),
            SurfaceSize::FitContent { max_w, max_h } => {
                let now = Instant::now();
                let interaction = InteractionState::default();
                let mut anim = Animator::new();
                anim.begin_frame(now, self.reduce_motion);
                let mut ctx = ViewContext {
                    now,
                    logical_size: (max_w.unwrap_or(1200), max_h.unwrap_or(800)),
                    scale: 1.0,
                    interaction: &interaction,
                    text: &mut self.text,
                    anim: &mut anim,
                    images: &self.images,
                };
                let tree = self.app.view(id, &mut ctx);
                anim.end_frame();
                let (w, h) = measure_tree(
                    &tree,
                    (max_w.map(|v| v as f32), max_h.map(|v| v as f32)),
                    &mut self.text,
                );
                (w.ceil().max(1.0) as u32, h.ceil().max(1.0) as u32)
            }
        }
    }

    fn apply_commands(&mut self, cmds: Vec<AppCommand<A::SurfaceId>>, qh: &QueueHandle<Self>) {
        for cmd in cmds {
            match cmd {
                AppCommand::Redraw(id) => {
                    if let Some(host) = self.hosts.get_mut(&id) {
                        host.needs_redraw = true;
                        queue_frame(host, qh);
                    }
                }
                AppCommand::RedrawAll => {
                    for host in self.hosts.values_mut() {
                        host.needs_redraw = true;
                        queue_frame(host, qh);
                    }
                }
                AppCommand::Focus(id, key) => {
                    if let Some(host) = self.hosts.get_mut(&id) {
                        host.interaction.focused = Some(key);
                        host.needs_redraw = true;
                        queue_frame(host, qh);
                    }
                }
                AppCommand::ScheduleTimer { tag, delay } => {
                    self.timers.schedule(tag, delay, Instant::now());
                    self.rearm_timer(qh);
                }
                AppCommand::CancelTimer(tag) => {
                    self.timers.cancel(tag);
                    self.rearm_timer(qh);
                }
                AppCommand::PauseTimer(tag) => {
                    self.timers.pause(tag, Instant::now());
                    self.rearm_timer(qh);
                }
                AppCommand::ResumeTimer(tag) => {
                    self.timers.resume(tag, Instant::now());
                    self.rearm_timer(qh);
                }
                AppCommand::SetReduceMotion(enabled) => {
                    if self.reduce_motion != enabled {
                        self.reduce_motion = enabled;
                        for host in self.hosts.values_mut() {
                            host.needs_redraw = true;
                            queue_frame(host, qh);
                        }
                    }
                }
                AppCommand::Quit => {
                    self.signal.stop();
                }
            }
        }
    }

    /// Stage any app image inserts/updates/removals and invalidate surface
    /// damage when GPU texture data changed (so same-`ImageId` updates like
    /// live screencopy frames are never skipped by `DrawList::diff_bounds`).
    fn sync_app_images(&mut self) {
        self.app.sync_images(&mut self.images);
        if self.images.has_pending_changes() {
            for host in self.hosts.values_mut() {
                host.damage.mark_full();
            }
        }
    }

    /// Re-arm the single calloop timer for the scheduler's earliest deadline.
    /// Called after every timer mutation and after each dispatch so refreshed,
    /// cancelled, paused, and newly-expired timers always re-align the OS timer.
    fn rearm_timer(&mut self, qh: &QueueHandle<Self>) {
        if let Some(tok) = self.timer_token.take() {
            self.loop_handle.remove(tok);
        }
        let Some(deadline) = self.timers.next_deadline() else {
            return;
        };
        let delay = deadline.saturating_duration_since(Instant::now());
        let qh_clone = qh.clone();
        let source = Timer::from_duration(delay);
        match self
            .loop_handle
            .insert_source(source, move |_, (), rt: &mut Runtime<A>| {
                rt.timer_token = None;
                let now = Instant::now();
                let expired = rt.timers.poll_expired(now);
                let mut cmds = Vec::new();
                for tag in expired {
                    cmds.extend(rt.app.on_timer(tag, now));
                }
                rt.apply_commands(cmds, &qh_clone);
                rt.sync_app_images();
                let _ = rt.sync_surfaces(&qh_clone);
                if rt.timer_token.is_none() {
                    rt.rearm_timer(&qh_clone);
                }
                TimeoutAction::Drop
            }) {
            Ok(tok) => self.timer_token = Some(tok),
            Err(e) => tracing::error!(?e, "insert timer source failed"),
        }
    }

    fn resize_host(&mut self, id: A::SurfaceId, lw: u32, lh: u32) {
        if let Some(host) = self.hosts.get_mut(&id) {
            host.logical = (lw.max(1), lh.max(1));
            host.configured = true;
            host.needs_redraw = true;
            host.damage.mark_full();
            rebuffer_host(host);
        }
    }

    fn rescale_host(&mut self, id: A::SurfaceId, scale_120: u32) {
        let scale = (scale_120 as f32 / 120.0).max(0.5);
        if let Some(host) = self.hosts.get_mut(&id)
            && (host.scale - scale).abs() >= 0.001
        {
            host.scale = scale;
            rebuffer_host(host);
            host.damage.mark_full();
            host.needs_redraw = true;
        }
    }

    fn present(&mut self, id: A::SurfaceId, qh: &QueueHandle<Self>) {
        let now = Instant::now();
        self.text.begin_frame();
        // Let the app stage image inserts/updates/removals before view + paint.
        self.sync_app_images();

        let (logical, scale, sizing) = {
            let Some(host) = self.hosts.get_mut(&id) else {
                return;
            };
            host.needs_redraw = false;
            (host.logical, host.scale, host.spec.sizing)
        };

        // 1. Build the declarative element tree safely.
        let element = {
            let Some(host) = self.hosts.get_mut(&id) else {
                return;
            };
            host.animator.begin_frame(now, self.reduce_motion);
            let mut ctx = ViewContext {
                now,
                logical_size: logical,
                scale,
                interaction: &host.interaction,
                text: &mut self.text,
                anim: &mut host.animator,
                images: &self.images,
            };
            let el = self.app.view(id, &mut ctx);
            host.animator.end_frame();
            el
        };

        // 2. Determine available layout constraints.
        let avail = match sizing {
            SurfaceSize::FitContent { max_w, max_h } => {
                (max_w.map(|v| v as f32), max_h.map(|v| v as f32))
            }
            SurfaceSize::Fixed(..) | SurfaceSize::Stretch => {
                (Some(logical.0 as f32), Some(logical.1 as f32))
            }
            SurfaceSize::StableExtent { width, height } => {
                (Some(width as f32), Some(height as f32))
            }
        };

        // 3. Layout & paint into DrawList.
        let built = {
            let Some(host) = self.hosts.get(&id) else {
                return;
            };
            layout_and_paint(
                element,
                avail,
                scale,
                &host.interaction,
                &mut self.text,
                &self.images,
                &mut self.draw_list,
            )
        };

        // 4. Dynamic FitContent resize: if content grew or shrank (e.g. a spring-
        //    morphing workspace island), update layer size + EGL buffer + viewport
        //    *before* swapping so there is NEVER clipping!
        if let SurfaceSize::FitContent { .. } = sizing {
            let want_w = built.content_size.0.ceil().max(1.0) as u32;
            let want_h = built.content_size.1.ceil().max(1.0) as u32;
            if (want_w != logical.0 || want_h != logical.1)
                && let Some(host) = self.hosts.get_mut(&id)
            {
                host.logical = (want_w, want_h);
                host.layer.set_size(want_w, want_h);
                host.damage.mark_full();
                rebuffer_host(host);
            }
        }

        // 5. Update shaped compositor blur and input regions when changed.
        let Some((bw, bh, egl_surface, regions_changed)) = self.sync_surface_regions(id, built)
        else {
            return;
        };

        // 5b. Compute this frame's content-diff dirty rect for buffer-age
        //     partial damage, then save it as the next frame's baseline.
        let current_dirty = {
            let Some(host) = self.hosts.get_mut(&id) else {
                return;
            };
            let dirty = self.draw_list.diff_bounds(&host.prev_draw_list);
            host.prev_draw_list.clone_from(&self.draw_list);
            dirty
        };

        // 6. Render with GLES3 and swap with damage (arming the next vsync
        //    frame callback atomically on the swap commit).
        let swapped = self.render_and_swap(id, bw, bh, egl_surface, current_dirty, qh);
        if regions_changed
            && !swapped
            && let Some(host) = self.hosts.get(&id)
        {
            host.layer.wl_surface().commit();
        }

        // 7. Sync surface lifecycle (e.g. closing panel finished its spring) and re-arm pacing.
        if let Err(e) = self.sync_surfaces(qh) {
            tracing::error!(%e, "sync_surfaces failed");
        }
        self.finish_present_pacing(id, now, swapped, qh);
    }

    fn finish_present_pacing(
        &mut self,
        id: A::SurfaceId,
        now: Instant,
        swapped: bool,
        qh: &QueueHandle<Self>,
    ) {
        let app_animating = self.app.is_animating(id, now);
        let Some(host) = self.hosts.get_mut(&id) else {
            return;
        };
        let still_animating = host.animator.is_animating() || app_animating;
        if swapped {
            // `frame_pending` was armed atomically before `eglSwapBuffers`.
            if !still_animating && !host.needs_redraw {
                host.last_frame = None;
            }
            return;
        }
        host.frame_pending = false;
        if still_animating || host.needs_redraw {
            queue_frame(host, qh);
        } else {
            host.last_frame = None;
        }
    }

    fn sync_surface_regions(
        &mut self,
        id: A::SurfaceId,
        built: BuiltFrame<A::Message>,
    ) -> Option<(u32, u32, EglSurface, bool)> {
        let host = self.hosts.get_mut(&id)?;
        let mut changed = false;
        if host.last_blur != built.blur_rects {
            update_blur_region(&self.compositor, host.effect.as_ref(), &built.blur_rects);
            host.last_blur.clone_from(&built.blur_rects);
            changed = true;
        }
        if host.spec.shaped_input && host.last_input != built.input_rects {
            update_input_region(
                &self.compositor,
                host.layer.wl_surface(),
                &built.input_rects,
            );
            host.last_input.clone_from(&built.input_rects);
            changed = true;
        }
        host.built = built;
        host.frames += 1;
        Some((host.buffer.0, host.buffer.1, host.egl_surface, changed))
    }

    fn render_and_swap(
        &mut self,
        id: A::SurfaceId,
        bw: u32,
        bh: u32,
        egl_surface: EglSurface,
        current_dirty: Option<DamageRect>,
        qh: &QueueHandle<Self>,
    ) -> bool {
        if let Err(e) = self.egl.make_current(Some(egl_surface)) {
            tracing::error!(?id, %e, "eglMakeCurrent failed");
            return false;
        }
        self.egl.set_swap_interval(0);
        let age = self.egl.buffer_age(egl_surface);
        let surface_size = (
            i32::try_from(bw).unwrap_or(i32::MAX),
            i32::try_from(bh).unwrap_or(i32::MAX),
        );
        let Some(host) = self.hosts.get(&id) else {
            return false;
        };
        let effective = host.damage.compute_damage(age, current_dirty, surface_size);
        // Nothing to repaint (or an empty rect): skip the redraw AND the swap.
        // `record_frame` only tracks actually-presented swaps, matching what
        // `EGL_BUFFER_AGE_KHR` counts, so skipped frames leave history intact.
        let Some(dmg) = effective else {
            return false;
        };
        if dmg.is_empty() {
            return false;
        }
        if let Some(gpu) = &mut self.gpu {
            let mut atlas = self.text.atlas_upload();
            gpu.render(
                bw,
                bh,
                &self.draw_list,
                &mut atlas,
                &mut self.images,
                Some(dmg),
            );
        }
        // Attach the next vsync frame callback BEFORE `eglSwapBuffers` so the
        // buffer attach and frame callback commit atomically in ONE commit.
        let armed_frame = if let Some(host) = self.hosts.get_mut(&id)
            && !host.frame_pending
        {
            host.layer
                .wl_surface()
                .frame(qh, FrameCallbackData(host.layer.wl_surface().clone()));
            host.frame_pending = true;
            true
        } else {
            false
        };
        let egl_rect = dmg.to_egl_rect(i32::try_from(bh).unwrap_or(i32::MAX));
        match self.egl.swap(egl_surface, &[egl_rect]) {
            Ok(()) => {
                if let Some(host) = self.hosts.get_mut(&id) {
                    host.damage.record_frame(current_dirty);
                }
                true
            }
            Err(e) => {
                if armed_frame && let Some(host) = self.hosts.get_mut(&id) {
                    host.frame_pending = false;
                }
                tracing::error!(?id, %e, "eglSwapBuffers failed");
                false
            }
        }
    }

    fn schedule_present(&mut self, id: A::SurfaceId, qh: &QueueHandle<Self>) {
        let can_present_now = self
            .hosts
            .get(&id)
            .is_some_and(|h| h.configured && !h.frame_pending);
        if can_present_now {
            self.present(id, qh);
        } else if let Some(host) = self.hosts.get_mut(&id) {
            queue_frame(host, qh);
        }
    }

    fn dispatch_ui_event_deferred(
        &mut self,
        id: A::SurfaceId,
        event: &InputEvent,
        qh: &QueueHandle<Self>,
    ) -> bool {
        let now = Instant::now();
        let (msgs, dirty) = {
            let Some(host) = self.hosts.get_mut(&id) else {
                return false;
            };
            host.built.dispatch_event(event, &mut host.interaction)
        };

        let mut all_cmds = Vec::new();
        for msg in msgs {
            all_cmds.extend(self.app.update(id, msg, now));
        }
        self.sync_app_images();
        if let Err(e) = self.sync_surfaces(qh) {
            tracing::error!(%e, "sync_surfaces after input failed");
        }
        self.apply_commands(all_cmds, qh);

        if let Some(host) = self.hosts.get_mut(&id)
            && (dirty || host.animator.is_animating() || self.app.is_animating(id, now))
        {
            host.needs_redraw = true;
            return true;
        }
        false
    }

    fn dispatch_ui_event(&mut self, id: A::SurfaceId, event: &InputEvent, qh: &QueueHandle<Self>) {
        if self.dispatch_ui_event_deferred(id, event, qh) {
            self.schedule_present(id, qh);
        }
    }

    fn surface_id_for_wl(&self, surface: &wl_surface::WlSurface) -> Option<A::SurfaceId> {
        self.hosts
            .iter()
            .find(|(_, h)| h.layer.wl_surface() == surface)
            .map(|(id, _)| *id)
    }

    fn collect_metrics(&self) -> HashMap<A::SurfaceId, SurfaceMetrics> {
        self.hosts
            .iter()
            .map(|(&id, h)| {
                let mut iv = h.intervals.clone();
                iv.sort_by(f64::total_cmp);
                let pct = |q: f64| {
                    if iv.is_empty() {
                        0.0
                    } else {
                        let idx = ((iv.len() - 1) as f64 * q).round() as usize;
                        iv[idx]
                    }
                };
                let mean = if iv.is_empty() {
                    0.0
                } else {
                    iv.iter().sum::<f64>() / iv.len() as f64
                };
                (
                    id,
                    SurfaceMetrics {
                        frames: h.frames,
                        logical_size: h.logical,
                        buffer_size: h.buffer,
                        scale: h.scale,
                        interval_ms_mean: mean,
                        interval_ms_p50: pct(0.50),
                        interval_ms_p95: pct(0.95),
                        interval_ms_max: pct(1.0),
                    },
                )
            })
            .collect()
    }

    fn destroy_all_surfaces(&mut self) {
        let _ = self.egl.make_current(None);
        for (_, host) in self.hosts.drain() {
            self.egl.destroy_surface(host.egl_surface);
        }
    }
}

fn rebuffer_host<A: ShellApp>(host: &mut SurfaceHost<A>) {
    let (lw, lh) = host.logical;
    let bw = (lw as f32 * host.scale).round().max(1.0) as u32;
    let bh = (lh as f32 * host.scale).round().max(1.0) as u32;
    host.buffer = (bw, bh);
    host.window.resize(bw, bh);
    if let Some(vp) = &host.viewport {
        vp.set_destination(
            i32::try_from(lw).unwrap_or(1),
            i32::try_from(lh).unwrap_or(1),
        );
    }
}

fn queue_frame<A: ShellApp>(host: &mut SurfaceHost<A>, qh: &QueueHandle<Runtime<A>>) {
    if host.frame_pending || !host.configured {
        return;
    }
    host.layer
        .wl_surface()
        .frame(qh, FrameCallbackData(host.layer.wl_surface().clone()));
    host.frame_pending = true;
    host.layer.wl_surface().commit();
}

fn update_blur_region(
    compositor: &CompositorState,
    effect: Option<&ExtBackgroundEffectSurfaceV1>,
    rects: &[BlurRect],
) {
    let Some(effect) = effect else {
        return;
    };
    if rects.is_empty() {
        effect.set_blur_region(None);
        return;
    }
    if let Ok(region) = Region::new(compositor) {
        for br in rects {
            let ox = br.rect.x.round() as i32;
            let oy = br.rect.y.round() as i32;
            let w = br.rect.w.round().max(1.0) as u32;
            let h = br.rect.h.round().max(1.0) as u32;
            for (bx, by, bw, bh) in rounded_rect_bands(w, h, br.radius) {
                region.add(ox + bx, oy + by, bw, bh);
            }
        }
        effect.set_blur_region(Some(region.wl_region()));
    }
}

fn update_input_region(
    compositor: &CompositorState,
    surface: &wl_surface::WlSurface,
    rects: &[BlurRect],
) {
    if rects.is_empty() {
        // `set_input_region(None)` means an *infinite* input region, not empty!
        // Only an explicit empty region makes a shaped surface click-through.
        if let Ok(region) = Region::new(compositor) {
            surface.set_input_region(Some(region.wl_region()));
        }
        return;
    }
    if let Ok(region) = Region::new(compositor) {
        for br in rects {
            let ox = br.rect.x.floor() as i32;
            let oy = br.rect.y.floor() as i32;
            let w = br.rect.w.ceil().max(1.0) as u32;
            let h = br.rect.h.ceil().max(1.0) as u32;
            for (bx, by, bw, bh) in rounded_rect_bands(w, h, br.radius) {
                region.add(ox + bx, oy + by, bw, bh);
            }
        }
        surface.set_input_region(Some(region.wl_region()));
    }
}

fn map_keysym(keysym: Keysym, utf8: Option<&str>) -> Key {
    match keysym {
        Keysym::Escape => Key::Escape,
        Keysym::Return | Keysym::KP_Enter => Key::Enter,
        Keysym::Tab | Keysym::ISO_Left_Tab => Key::Tab,
        Keysym::BackSpace => Key::BackSpace,
        Keysym::Delete | Keysym::KP_Delete => Key::Delete,
        Keysym::Left | Keysym::KP_Left => Key::Left,
        Keysym::Right | Keysym::KP_Right => Key::Right,
        Keysym::Up | Keysym::KP_Up => Key::Up,
        Keysym::Down | Keysym::KP_Down => Key::Down,
        Keysym::Home | Keysym::KP_Home => Key::Home,
        Keysym::End | Keysym::KP_End => Key::End,
        Keysym::space => Key::Space,
        _ => utf8
            .and_then(|s| s.chars().next())
            .filter(|c| !c.is_control())
            .map_or(Key::Other, Key::Char),
    }
}

// ── SCTK Handler Implementations ─────────────────────────────────────────────

impl<A: ShellApp> CompositorHandler for Runtime<A> {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        new_factor: i32,
    ) {
        // Fallback when wp_fractional_scale_v1 is absent.
        if self.fs_manager.is_none()
            && let Some(id) = self.surface_id_for_wl(surface)
        {
            self.rescale_host(id, u32::try_from(new_factor.max(1)).unwrap_or(1) * 120);
            if let Some(host) = self.hosts.get_mut(&id) {
                queue_frame(host, qh);
            }
        }
    }

    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_transform: wl_output::Transform,
    ) {
    }

    fn frame(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
        let Some(id) = self.surface_id_for_wl(surface) else {
            return;
        };
        let now = Instant::now();
        let should_present = {
            let Some(host) = self.hosts.get_mut(&id) else {
                return;
            };
            host.frame_pending = false;
            if let Some(last) = host.last_frame {
                host.intervals
                    .push(now.saturating_duration_since(last).as_secs_f64() * 1000.0);
            }
            host.last_frame = Some(now);
            host.needs_redraw || host.animator.is_animating() || self.app.is_animating(id, now)
        };
        if should_present {
            self.present(id, qh);
        }
    }

    fn surface_enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }
}

impl<A: ShellApp> OutputHandler for Runtime<A> {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, qh: &QueueHandle<Self>, _: wl_output::WlOutput) {
        if self.started {
            self.notify_outputs(qh, false);
        }
    }
    fn update_output(&mut self, _: &Connection, qh: &QueueHandle<Self>, _: wl_output::WlOutput) {
        if self.started {
            self.notify_outputs(qh, false);
        }
    }
    fn output_destroyed(&mut self, _: &Connection, qh: &QueueHandle<Self>, _: wl_output::WlOutput) {
        if self.started {
            self.notify_outputs(qh, false);
        }
    }
}

impl<A: ShellApp> LayerShellHandler for Runtime<A> {
    fn closed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, layer: &LayerSurface) {
        if let Some(id) = self.surface_id_for_wl(layer.wl_surface())
            && let Some(host) = self.hosts.remove(&id)
        {
            let _ = self.egl.make_current(None);
            self.egl.destroy_surface(host.egl_surface);
        }
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        let Some(id) = self.surface_id_for_wl(layer.wl_surface()) else {
            return;
        };
        let (cw, ch) = configure.new_size;
        let cur = self.hosts.get(&id).map_or((1, 1), |h| h.logical);
        let w = if cw == 0 { cur.0 } else { cw };
        let h = if ch == 0 { cur.1 } else { ch };
        self.resize_host(id, w, h);
        self.present(id, qh);
    }
}

impl<A: ShellApp> SeatHandler for Runtime<A> {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }
    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}

    fn new_capability(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        match capability {
            Capability::Keyboard if self.keyboard.is_none() => {
                if let Ok(kb) = self.seat_state.get_keyboard(qh, &seat, None) {
                    self.keyboard = Some(kb);
                }
            }
            Capability::Pointer if self.pointer.is_none() => {
                if let Ok(ptr) = self.seat_state.get_pointer(qh, &seat) {
                    self.pointer = Some(ptr);
                }
            }
            _ => {}
        }
    }

    fn remove_capability(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        match capability {
            Capability::Keyboard => drop(self.keyboard.take()),
            Capability::Pointer => drop(self.pointer.take()),
            _ => {}
        }
    }

    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl<A: ShellApp> KeyboardHandler for Runtime<A> {
    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        surface: &wl_surface::WlSurface,
        _: u32,
        _: &[u32],
        _: &[Keysym],
    ) {
        self.keyboard_surface = self.surface_id_for_wl(surface);
    }

    fn leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        surface: &wl_surface::WlSurface,
        _: u32,
    ) {
        if self.keyboard_surface == self.surface_id_for_wl(surface) {
            self.keyboard_surface = None;
        }
    }

    fn press_key(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        event: KeyEvent,
    ) {
        self.handle_key_event(event, qh);
    }

    fn repeat_key(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        event: KeyEvent,
    ) {
        self.handle_key_event(event, qh);
    }

    fn release_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        _: KeyEvent,
    ) {
    }

    fn update_modifiers(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        modifiers: Modifiers,
        _: RawModifiers,
        _: u32,
    ) {
        let caps_changed = modifiers.caps_lock != self.modifiers.caps_lock;
        self.modifiers = modifiers;
        if caps_changed {
            for host in self.hosts.values_mut() {
                host.interaction.caps_lock = modifiers.caps_lock;
                host.needs_redraw = true;
                queue_frame(host, qh);
            }
        }
    }
}

impl<A: ShellApp> Runtime<A> {
    fn handle_key_event(&mut self, event: KeyEvent, qh: &QueueHandle<Self>) {
        let Some(id) = self.keyboard_surface else {
            return;
        };
        let key = map_keysym(event.keysym, event.utf8.as_deref());
        let ev = InputEvent::KeyDown {
            key,
            utf8: event.utf8,
            ctrl: self.modifiers.ctrl,
            shift: self.modifiers.shift,
            alt: self.modifiers.alt,
            caps_lock: self.modifiers.caps_lock,
        };
        self.dispatch_ui_event(id, &ev, qh);
    }
}

impl<A: ShellApp> PointerHandler for Runtime<A> {
    fn pointer_frame(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        _pointer: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        // Coalesce all pointer events (motion + high-rate scroll axes) in this
        // Wayland frame before presenting once per dirty surface.
        let mut dirty_surfaces: Vec<A::SurfaceId> = Vec::new();
        for event in events {
            let Some(id) = self.surface_id_for_wl(&event.surface) else {
                continue;
            };
            let (x, y) = (event.position.0 as f32, event.position.1 as f32);
            let dirty = match event.kind {
                PointerEventKind::Enter { .. } => {
                    let cmds = self.app.on_pointer_enter(id, (x, y), Instant::now());
                    self.sync_app_images();
                    if let Err(e) = self.sync_surfaces(qh) {
                        tracing::error!(%e, "sync_surfaces after pointer enter failed");
                    }
                    self.apply_commands(cmds, qh);
                    self.dispatch_ui_event_deferred(id, &InputEvent::PointerMove { x, y }, qh)
                }
                PointerEventKind::Motion { .. } => {
                    self.dispatch_ui_event_deferred(id, &InputEvent::PointerMove { x, y }, qh)
                }
                PointerEventKind::Leave { .. } => {
                    let ui_dirty =
                        self.dispatch_ui_event_deferred(id, &InputEvent::PointerLeave, qh);
                    let cmds = self.app.on_pointer_leave(id, Instant::now());
                    self.sync_app_images();
                    if let Err(e) = self.sync_surfaces(qh) {
                        tracing::error!(%e, "sync_surfaces after pointer leave failed");
                    }
                    self.apply_commands(cmds, qh);
                    ui_dirty
                }
                PointerEventKind::Press { button, .. } => self.dispatch_ui_event_deferred(
                    id,
                    &InputEvent::PointerDown { x, y, button },
                    qh,
                ),
                PointerEventKind::Release { button, .. } => {
                    self.dispatch_ui_event_deferred(id, &InputEvent::PointerUp { x, y, button }, qh)
                }
                PointerEventKind::Axis { vertical, .. } => {
                    let delta = scroll_delta_px(vertical);
                    if delta.abs() > 0.01 {
                        self.dispatch_ui_event_deferred(
                            id,
                            &InputEvent::Scroll {
                                x,
                                y,
                                delta_y: delta,
                            },
                            qh,
                        )
                    } else {
                        false
                    }
                }
            };
            if dirty && !dirty_surfaces.contains(&id) {
                dirty_surfaces.push(id);
            }
        }
        for id in dirty_surfaces {
            self.schedule_present(id, qh);
        }
    }
}

fn scroll_delta_px(axis: AxisScroll) -> f32 {
    if axis.absolute.abs() > 0.001 {
        axis.absolute as f32
    } else if axis.discrete != 0 {
        axis.discrete as f32 * 36.0
    } else {
        0.0
    }
}

impl<A: ShellApp> BackgroundEffectHandler for Runtime<A> {
    fn background_effect_state(&mut self) -> &mut BackgroundEffectState {
        &mut self.effect_state
    }

    fn update_capabilities(&mut self) {
        tracing::debug!(
            caps = ?self.effect_state.capabilities(),
            "updated ext-background-effect-v1 capabilities"
        );
    }
}

// ── Dispatch2 impls for Wayland globals/objects outside SCTK's built-in traits

impl<A: ShellApp> Dispatch2<wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1, Runtime<A>>
    for GlobalData
{
    fn event(
        &self,
        _: &mut Runtime<A>,
        _: &wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1,
        _: wp_fractional_scale_manager_v1::Event,
        _: &Connection,
        _: &QueueHandle<Runtime<A>>,
    ) {
    }
}

impl<A: ShellApp> Dispatch2<wp_viewporter::WpViewporter, Runtime<A>> for GlobalData {
    fn event(
        &self,
        _: &mut Runtime<A>,
        _: &wp_viewporter::WpViewporter,
        _: wp_viewporter::Event,
        _: &Connection,
        _: &QueueHandle<Runtime<A>>,
    ) {
    }
}

impl<A: ShellApp> Dispatch2<wp_fractional_scale_v1::WpFractionalScaleV1, Runtime<A>>
    for SurfaceTag<A::SurfaceId>
{
    fn event(
        &self,
        rt: &mut Runtime<A>,
        _: &wp_fractional_scale_v1::WpFractionalScaleV1,
        event: wp_fractional_scale_v1::Event,
        _: &Connection,
        qh: &QueueHandle<Runtime<A>>,
    ) {
        if let wp_fractional_scale_v1::Event::PreferredScale { scale } = event {
            rt.rescale_host(self.0, scale);
            if let Some(host) = rt.hosts.get_mut(&self.0)
                && host.configured
            {
                queue_frame(host, qh);
            }
        }
    }
}

impl<A: ShellApp> Dispatch2<wp_viewport::WpViewport, Runtime<A>> for SurfaceTag<A::SurfaceId> {
    fn event(
        &self,
        _: &mut Runtime<A>,
        _: &wp_viewport::WpViewport,
        _: wp_viewport::Event,
        _: &Connection,
        _: &QueueHandle<Runtime<A>>,
    ) {
    }
}

impl<A: ShellApp> ProvidesRegistryState for Runtime<A> {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState];
}

delegate_registry!(@<A: ShellApp> Runtime<A>);
smithay_client_toolkit::delegate_dispatch2!(@<A: ShellApp> Runtime<A>);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_channel_wakes_calloop_event_loop_from_background_thread() {
        let (tx, rx) = event_channel::<&'static str>();
        let mut event_loop: EventLoop<'_, Vec<&'static str>> =
            EventLoop::try_new().expect("create calloop EventLoop");
        let signal = event_loop.get_signal();

        event_loop
            .handle()
            .insert_source(rx.rx, move |event, (), received: &mut Vec<&'static str>| {
                if let calloop_channel::Event::Msg(msg) = event {
                    received.push(msg);
                    if received.len() == 2 {
                        signal.stop();
                    }
                }
            })
            .expect("insert EventReceiver into calloop");

        let sender_thread = std::thread::Builder::new()
            .name("test-calloop-wakeup".into())
            .spawn(move || {
                tx.send("niri-workspace").unwrap();
                tx.send("triton-msg").unwrap();
            })
            .expect("spawn sender thread");

        let mut received = Vec::new();
        event_loop
            .run(Some(Duration::from_secs(2)), &mut received, |_| {})
            .expect("calloop event loop runs");
        sender_thread.join().expect("sender thread joins");

        assert_eq!(received, vec!["niri-workspace", "triton-msg"]);
    }
}
