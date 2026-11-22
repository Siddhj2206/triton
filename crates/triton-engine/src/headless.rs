//! Headless deterministic session driver, key-addressed scripted input,
//! zero-dependency PNG encoding, and displayless offscreen GPU rendering.
//!
//! 100% safe Rust: all GLES3/EGL FFI stays quarantined inside
//! [`crate::platform::egl`] and [`crate::gpu`].

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};

use crate::anim::Animator;
use crate::error::EngineError;
use crate::gpu::{DrawList, Gpu};
use crate::image::ImageStore;
use crate::platform::egl::Egl;
use crate::platform::{AppCommand, EventReceiver, OutputInfo, ShellApp, SurfaceSize, ViewContext};
use crate::style::{Rect, Rgba};
use crate::text::TextEngine;
use crate::ui::{
    BuiltFrame, Element, InputEvent, InteractionState, Key, NodeKey, layout_and_paint,
};

/// Default left mouse button code (Linux `BTN_LEFT`).
const BTN_LEFT: u32 = 272;

/// Deterministic headless UI session owning a virtual clock, interaction state,
/// animator, text engine, image store, draw list, and last built frame.
pub struct HeadlessSession<M> {
    size: (f32, f32),
    scale: f32,
    now: Instant,
    reduce_motion: bool,
    interaction: InteractionState,
    anim: Animator,
    text: TextEngine,
    images: ImageStore,
    draw_list: DrawList,
    frame: BuiltFrame<M>,
    outputs: Vec<OutputInfo>,
}

impl<M> std::fmt::Debug for HeadlessSession<M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HeadlessSession")
            .field("size", &self.size)
            .field("scale", &self.scale)
            .field("reduce_motion", &self.reduce_motion)
            .field("nodes", &self.frame.nodes.len())
            .finish_non_exhaustive()
    }
}

impl<M> HeadlessSession<M> {
    /// Create a new headless session at logical `size` and display `scale`.
    pub fn new(size: (f32, f32), scale: f32) -> Self {
        Self {
            size: (size.0.max(1.0), size.1.max(1.0)),
            scale: scale.max(0.5),
            now: Instant::now(),
            reduce_motion: false,
            interaction: InteractionState::default(),
            anim: Animator::new(),
            text: TextEngine::new(),
            images: ImageStore::new(),
            draw_list: DrawList::default(),
            frame: BuiltFrame::default(),
            outputs: Vec::new(),
        }
    }

    pub fn size(&self) -> (f32, f32) {
        self.size
    }

    pub fn set_size(&mut self, size: (f32, f32)) {
        self.size = (size.0.max(1.0), size.1.max(1.0));
    }

    pub fn scale(&self) -> f32 {
        self.scale
    }

    pub fn set_scale(&mut self, scale: f32) {
        self.scale = scale.max(0.5);
    }

    pub fn now(&self) -> Instant {
        self.now
    }

    /// Advance the virtual clock by `dt`.
    pub fn advance(&mut self, dt: Duration) {
        self.now += dt;
    }

    pub fn set_reduce_motion(&mut self, enabled: bool) {
        self.reduce_motion = enabled;
    }

    pub fn reduce_motion(&self) -> bool {
        self.reduce_motion
    }

    pub fn interaction(&self) -> &InteractionState {
        &self.interaction
    }

    pub fn interaction_mut(&mut self) -> &mut InteractionState {
        &mut self.interaction
    }

    pub fn images(&self) -> &ImageStore {
        &self.images
    }

    pub fn images_mut(&mut self) -> &mut ImageStore {
        &mut self.images
    }

    pub fn text(&self) -> &TextEngine {
        &self.text
    }

    pub fn text_mut(&mut self) -> &mut TextEngine {
        &mut self.text
    }

    pub fn draw_list(&self) -> &DrawList {
        &self.draw_list
    }

    pub fn frame(&self) -> &BuiltFrame<M> {
        &self.frame
    }

    pub fn is_animating(&self) -> bool {
        self.anim.is_animating()
    }

    /// Sync images from a [`ShellApp`] into this session's [`ImageStore`].
    pub fn sync_app_images<A: ShellApp>(&mut self, app: &mut A) {
        app.sync_images(&mut self.images);
    }

    /// Return the current headless output inventory.
    pub fn outputs(&self) -> &[OutputInfo] {
        &self.outputs
    }

    /// Replace the headless output inventory and deliver `on_outputs_changed` to `app`.
    pub fn set_outputs<A: ShellApp>(
        &mut self,
        app: &mut A,
        outputs: Vec<OutputInfo>,
    ) -> Vec<AppCommand<A::SurfaceId>> {
        self.outputs = outputs;
        let cmds = app.on_outputs_changed(&self.outputs, self.now);
        app.sync_images(&mut self.images);
        cmds
    }

    /// Add or update an output (matched by `output.name`) and notify `app`.
    pub fn add_output<A: ShellApp>(
        &mut self,
        app: &mut A,
        output: OutputInfo,
    ) -> Vec<AppCommand<A::SurfaceId>> {
        if let Some(existing) = self.outputs.iter_mut().find(|o| o.name == output.name) {
            *existing = output;
        } else {
            self.outputs.push(output);
        }
        let cmds = app.on_outputs_changed(&self.outputs, self.now);
        app.sync_images(&mut self.images);
        cmds
    }

    /// Remove an output by `name` and notify `app` (no-op if absent).
    pub fn remove_output<A: ShellApp>(
        &mut self,
        app: &mut A,
        name: &str,
    ) -> Vec<AppCommand<A::SurfaceId>> {
        let before = self.outputs.len();
        self.outputs.retain(|o| o.name != name);
        if self.outputs.len() == before {
            return Vec::new();
        }
        let cmds = app.on_outputs_changed(&self.outputs, self.now);
        app.sync_images(&mut self.images);
        cmds
    }

    /// Drain all pending external events from `receiver` in FIFO order, invoking
    /// `on_event` at the session's virtual clock and syncing images afterward.
    pub fn drain_events<A, E, F>(
        &mut self,
        app: &mut A,
        receiver: &EventReceiver<E>,
        mut on_event: F,
    ) -> Vec<AppCommand<A::SurfaceId>>
    where
        A: ShellApp,
        F: FnMut(&mut A, E, Instant) -> Vec<AppCommand<A::SurfaceId>>,
    {
        let mut cmds = Vec::new();
        for ev in receiver.drain() {
            cmds.extend(on_event(app, ev, self.now));
        }
        app.sync_images(&mut self.images);
        cmds
    }

    /// Render a single frame from `view` at the session's current `size` and virtual clock.
    pub fn render(
        &mut self,
        view: impl FnOnce(&mut ViewContext<'_>) -> Element<M>,
    ) -> &BuiltFrame<M> {
        let avail = (Some(self.size.0), Some(self.size.1));
        self.render_with_avail(avail, false, view)
    }

    /// Render a content-sized frame (matching [`SurfaceSize::FitContent`]) and
    /// update the session's logical `size` to the measured `content_size`.
    pub fn render_fit(
        &mut self,
        max_size: (Option<f32>, Option<f32>),
        view: impl FnOnce(&mut ViewContext<'_>) -> Element<M>,
    ) -> &BuiltFrame<M> {
        self.render_with_avail(max_size, true, view)
    }

    /// Render one surface of an actual [`ShellApp`] using its declared [`SurfaceSize`].
    pub fn render_surface<A: ShellApp<Message = M>>(
        &mut self,
        app: &mut A,
        id: A::SurfaceId,
        sizing: SurfaceSize,
    ) -> &BuiltFrame<M> {
        app.sync_images(&mut self.images);
        match sizing {
            SurfaceSize::FitContent { max_w, max_h } => self
                .render_fit((max_w.map(|v| v as f32), max_h.map(|v| v as f32)), |ctx| {
                    app.view(id, ctx)
                }),
            SurfaceSize::Fixed(w, h)
            | SurfaceSize::StableExtent {
                width: w,
                height: h,
            } => {
                self.set_size((w.max(1) as f32, h.max(1) as f32));
                self.render(|ctx| app.view(id, ctx))
            }
            SurfaceSize::Stretch => self.render(|ctx| app.view(id, ctx)),
        }
    }

    fn render_with_avail(
        &mut self,
        avail: (Option<f32>, Option<f32>),
        adopt_content_size: bool,
        view: impl FnOnce(&mut ViewContext<'_>) -> Element<M>,
    ) -> &BuiltFrame<M> {
        self.text.begin_frame();
        self.anim.begin_frame(self.now, self.reduce_motion);
        let logical_size = (
            avail.0.unwrap_or(self.size.0).round().max(1.0) as u32,
            avail.1.unwrap_or(self.size.1).round().max(1.0) as u32,
        );
        let elem = {
            let mut ctx = ViewContext {
                now: self.now,
                logical_size,
                scale: self.scale,
                interaction: &self.interaction,
                text: &mut self.text,
                anim: &mut self.anim,
                images: &self.images,
            };
            view(&mut ctx)
        };
        self.anim.end_frame();

        self.frame = layout_and_paint(
            elem,
            avail,
            self.scale,
            &self.interaction,
            &mut self.text,
            &self.images,
            &mut self.draw_list,
        );
        if adopt_content_size {
            self.size = (
                self.frame.content_size.0.ceil().max(1.0),
                self.frame.content_size.1.ceil().max(1.0),
            );
        }
        &self.frame
    }

    /// Simulate the pointer entering surface `id` at logical coordinates `pos`.
    ///
    /// Invokes [`ShellApp::on_pointer_enter`] at the session's virtual clock and
    /// dispatches `InputEvent::PointerMove` so widget hover hit-testing updates
    /// identically to the live Wayland runtime.
    pub fn pointer_enter<A: ShellApp<Message = M>>(
        &mut self,
        app: &mut A,
        id: A::SurfaceId,
        pos: (f32, f32),
    ) -> Vec<AppCommand<A::SurfaceId>> {
        let mut cmds = app.on_pointer_enter(id, pos, self.now);
        let msgs = self.dispatch(&InputEvent::PointerMove { x: pos.0, y: pos.1 });
        for msg in msgs {
            cmds.extend(app.update(id, msg, self.now));
        }
        app.sync_images(&mut self.images);
        cmds
    }

    /// Simulate the pointer leaving surface `id`.
    ///
    /// Clears widget hover/press state via `InputEvent::PointerLeave` and
    /// invokes [`ShellApp::on_pointer_leave`] at the session's virtual clock.
    pub fn pointer_leave<A: ShellApp<Message = M>>(
        &mut self,
        app: &mut A,
        id: A::SurfaceId,
    ) -> Vec<AppCommand<A::SurfaceId>> {
        let mut cmds = Vec::new();
        let msgs = self.dispatch(&InputEvent::PointerLeave);
        for msg in msgs {
            cmds.extend(app.update(id, msg, self.now));
        }
        cmds.extend(app.on_pointer_leave(id, self.now));
        app.sync_images(&mut self.images);
        cmds
    }

    /// Dispatch a raw [`InputEvent`] against the last rendered frame.
    pub fn dispatch(&mut self, event: &InputEvent) -> Vec<M> {
        self.frame.dispatch_event(event, &mut self.interaction).0
    }

    /// Dispatch a raw [`InputEvent`], also returning whether interaction state changed.
    pub fn dispatch_dirty(&mut self, event: &InputEvent) -> (Vec<M>, bool) {
        self.frame.dispatch_event(event, &mut self.interaction)
    }

    /// Resolve a string key or slash-separated key path (e.g. `"slider/knob"`)
    /// to its visible (clipped) hit rectangle in logical surface coordinates.
    pub fn hit_rect(&self, key: &str) -> Option<Rect> {
        let path: Vec<NodeKey> = key
            .split('/')
            .filter(|s| !s.is_empty())
            .map(NodeKey::named)
            .collect();
        let node = self.frame.node_by_path(&path)?;
        let visible = match node.clip {
            Some(c) => node.rect.intersect(c),
            None => node.rect,
        };
        (!visible.is_empty()).then_some(visible)
    }

    /// Resolve a [`NodeKey`] to its visible hit rectangle in logical surface coords.
    pub fn hit_rect_key(&self, key: NodeKey) -> Option<Rect> {
        let node = self.frame.node_by_key(key)?;
        let visible = match node.clip {
            Some(c) => node.rect.intersect(c),
            None => node.rect,
        };
        (!visible.is_empty()).then_some(visible)
    }

    /// Move to, press, and release the center of the element addressed by `key`.
    pub fn tap_key(&mut self, key: &str) -> Result<Vec<M>, EngineError> {
        let rect = self.require_rect(key)?;
        let (x, y) = rect_center(rect);
        let mut msgs = self.dispatch(&InputEvent::PointerMove { x, y });
        msgs.extend(self.dispatch(&InputEvent::PointerDown {
            x,
            y,
            button: BTN_LEFT,
        }));
        msgs.extend(self.dispatch(&InputEvent::PointerUp {
            x,
            y,
            button: BTN_LEFT,
        }));
        Ok(msgs)
    }

    /// Move the pointer to the center of the element addressed by `key`.
    pub fn hover_key(&mut self, key: &str) -> Result<Vec<M>, EngineError> {
        let rect = self.require_rect(key)?;
        let (x, y) = rect_center(rect);
        Ok(self.dispatch(&InputEvent::PointerMove { x, y }))
    }

    /// Drag across the element addressed by `key` from normalized fraction
    /// `from_frac` `(0..=1, 0..=1)` to `to_frac`.
    pub fn drag_key(
        &mut self,
        key: &str,
        from_frac: (f32, f32),
        to_frac: (f32, f32),
    ) -> Result<Vec<M>, EngineError> {
        let rect = self.require_rect(key)?;
        let x0 = rect.x + rect.w * from_frac.0.clamp(0.0, 1.0);
        let y0 = rect.y + rect.h * from_frac.1.clamp(0.0, 1.0);
        let x1 = rect.x + rect.w * to_frac.0.clamp(0.0, 1.0);
        let y1 = rect.y + rect.h * to_frac.1.clamp(0.0, 1.0);
        let mut msgs = self.dispatch(&InputEvent::PointerMove { x: x0, y: y0 });
        msgs.extend(self.dispatch(&InputEvent::PointerDown {
            x: x0,
            y: y0,
            button: BTN_LEFT,
        }));
        msgs.extend(self.dispatch(&InputEvent::PointerMove { x: x1, y: y1 }));
        msgs.extend(self.dispatch(&InputEvent::PointerUp {
            x: x1,
            y: y1,
            button: BTN_LEFT,
        }));
        Ok(msgs)
    }

    /// Dispatch a wheel scroll event centered on the element addressed by `key`.
    pub fn scroll_key(&mut self, key: &str, delta_y: f32) -> Result<Vec<M>, EngineError> {
        let rect = self.require_rect(key)?;
        let (x, y) = rect_center(rect);
        Ok(self.dispatch(&InputEvent::Scroll { x, y, delta_y }))
    }

    /// Dispatch a keyboard key press event.
    pub fn send_key(&mut self, key: Key, utf8: Option<&str>) -> Vec<M> {
        self.dispatch(&InputEvent::KeyDown {
            key,
            utf8: utf8.map(str::to_owned),
            ctrl: false,
            shift: false,
            alt: false,
            caps_lock: self.interaction.caps_lock,
        })
    }

    /// Serialize the last rendered frame's semantic tree to pretty JSON.
    pub fn tree_json(&self) -> String {
        self.frame.tree_json()
    }

    /// Execute a timestamped JSON [`Script`] against `view` and message handler
    /// `on_msg`, advancing virtual time deterministically between events.
    pub fn run_script(
        &mut self,
        script: &Script,
        mut view: impl FnMut(&mut ViewContext<'_>) -> Element<M>,
        mut on_msg: impl FnMut(M, Instant),
    ) -> Result<ScriptOutcome, EngineError> {
        let t0 = self.now;
        self.render(&mut view);

        let mut ordered: Vec<&ScriptEvent> = script.events.iter().collect();
        ordered.sort_by(|a, b| a.t.total_cmp(&b.t));

        let mut marks = HashMap::new();
        let mut events_dispatched = 0usize;

        for ev in ordered {
            let target_now = t0 + Duration::from_secs_f64(ev.t.max(0.0));
            if target_now > self.now {
                self.now = target_now;
                self.render(&mut view);
            }

            if let ScriptEventKind::Mark { name } = &ev.kind {
                marks.insert(name.clone(), ev.t);
                continue;
            }

            let (msgs, dirty) = self.dispatch_script_event(&ev.kind)?;
            events_dispatched += 1;

            let had_msgs = !msgs.is_empty();
            for m in msgs {
                on_msg(m, self.now);
            }
            if had_msgs || dirty {
                self.render(&mut view);
            }
        }

        Ok(ScriptOutcome {
            marks,
            events_dispatched,
        })
    }

    fn dispatch_script_event(
        &mut self,
        kind: &ScriptEventKind,
    ) -> Result<(Vec<M>, bool), EngineError> {
        match kind {
            ScriptEventKind::Tap { target, button } => {
                let (x, y) = self.resolve_target(target)?;
                let (mut msgs, d0) = self.dispatch_dirty(&InputEvent::PointerMove { x, y });
                let (m1, d1) = self.dispatch_dirty(&InputEvent::PointerDown {
                    x,
                    y,
                    button: *button,
                });
                let (m2, d2) = self.dispatch_dirty(&InputEvent::PointerUp {
                    x,
                    y,
                    button: *button,
                });
                msgs.extend(m1);
                msgs.extend(m2);
                Ok((msgs, d0 || d1 || d2))
            }
            ScriptEventKind::Move { target } => {
                let (x, y) = self.resolve_target(target)?;
                Ok(self.dispatch_dirty(&InputEvent::PointerMove { x, y }))
            }
            ScriptEventKind::Down { target, button } => {
                let (x, y) = self.resolve_target(target)?;
                Ok(self.dispatch_dirty(&InputEvent::PointerDown {
                    x,
                    y,
                    button: *button,
                }))
            }
            ScriptEventKind::Up { target, button } => {
                let (x, y) = self.resolve_target(target)?;
                Ok(self.dispatch_dirty(&InputEvent::PointerUp {
                    x,
                    y,
                    button: *button,
                }))
            }
            ScriptEventKind::Scroll { target, delta_y } => {
                let (x, y) = self.resolve_target(target)?;
                Ok(self.dispatch_dirty(&InputEvent::Scroll {
                    x,
                    y,
                    delta_y: *delta_y,
                }))
            }
            ScriptEventKind::Key {
                key,
                utf8,
                ctrl,
                shift,
                alt,
                caps_lock,
            } => Ok(self.dispatch_dirty(&InputEvent::KeyDown {
                key: *key,
                utf8: utf8.clone(),
                ctrl: *ctrl,
                shift: *shift,
                alt: *alt,
                caps_lock: caps_lock.unwrap_or(self.interaction.caps_lock),
            })),
            ScriptEventKind::Mark { .. } => Ok((Vec::new(), false)),
        }
    }

    fn require_rect(&self, key: &str) -> Result<Rect, EngineError> {
        self.hit_rect(key)
            .ok_or_else(|| EngineError::Script(format!("no visible node matches key '{key}'")))
    }

    fn resolve_target(&self, target: &ScriptTarget) -> Result<(f32, f32), EngineError> {
        match target {
            ScriptTarget::Key(k) => Ok(rect_center(self.require_rect(k)?)),
            ScriptTarget::Point(x, y) => Ok((*x, *y)),
        }
    }
}

fn rect_center(r: Rect) -> (f32, f32) {
    (r.x + r.w * 0.5, r.y + r.h * 0.5)
}

// ── JSON Script schema ───────────────────────────────────────────────────────

/// Pointer location target in a [`ScriptEvent`].
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum ScriptTarget {
    /// Element key or slash-separated key path (e.g. `"slider/knob"`).
    Key(String),
    /// Explicit logical surface coordinates `(x, y)`.
    Point(f32, f32),
}

/// Payload of a single timestamped event in a [`Script`].
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum ScriptEventKind {
    Tap {
        target: ScriptTarget,
        button: u32,
    },
    Move {
        target: ScriptTarget,
    },
    Down {
        target: ScriptTarget,
        button: u32,
    },
    Up {
        target: ScriptTarget,
        button: u32,
    },
    Scroll {
        target: ScriptTarget,
        delta_y: f32,
    },
    Key {
        key: Key,
        utf8: Option<String>,
        ctrl: bool,
        shift: bool,
        alt: bool,
        caps_lock: Option<bool>,
    },
    Mark {
        name: String,
    },
}

/// One timestamped event in a headless replay [`Script`].
#[derive(Debug, Clone, PartialEq)]
pub struct ScriptEvent {
    /// Virtual time in seconds from script start.
    pub t: f64,
    pub kind: ScriptEventKind,
}

/// Result summary returned by [`HeadlessSession::run_script`].
#[derive(Debug, Clone, PartialEq)]
pub struct ScriptOutcome {
    /// Named time marks encountered during execution (`name -> t` seconds).
    pub marks: HashMap<String, f64>,
    /// Number of input events dispatched (excludes `mark` entries).
    pub events_dispatched: usize,
}

/// Deterministic JSON input script (`{"events": [...]}`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Script {
    pub events: Vec<ScriptEvent>,
}

impl Script {
    /// Parse a script from JSON text:
    /// `{"events": [{"t": 0.1, "type": "tap", "key": "btn"}, ...]}`.
    pub fn from_json(input: &str) -> Result<Self, EngineError> {
        let root: serde_json::Value = serde_json::from_str(input)
            .map_err(|e| EngineError::Script(format!("invalid script JSON: {e}")))?;
        let arr = root
            .get("events")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| {
                EngineError::Script("script JSON must contain an 'events' array".into())
            })?;

        let mut events = Vec::with_capacity(arr.len());
        for (idx, item) in arr.iter().enumerate() {
            events.push(parse_script_event(item, idx)?);
        }

        Ok(Self { events })
    }
}

fn parse_script_event(item: &serde_json::Value, idx: usize) -> Result<ScriptEvent, EngineError> {
    let t = item
        .get("t")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(0.0);
    let ty = item
        .get("type")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| EngineError::Script(format!("event[{idx}] missing string 'type'")))?;

    let button = item
        .get("button")
        .and_then(serde_json::Value::as_u64)
        .and_then(|v| u32::try_from(v).ok())
        .unwrap_or(BTN_LEFT);

    let kind = match ty {
        "tap" => ScriptEventKind::Tap {
            target: parse_target(item, idx)?,
            button,
        },
        "move" => ScriptEventKind::Move {
            target: parse_target(item, idx)?,
        },
        "down" => ScriptEventKind::Down {
            target: parse_target(item, idx)?,
            button,
        },
        "up" => ScriptEventKind::Up {
            target: parse_target(item, idx)?,
            button,
        },
        "scroll" => {
            let delta_y = item
                .get("delta_y")
                .and_then(serde_json::Value::as_f64)
                .ok_or_else(|| {
                    EngineError::Script(format!("event[{idx}] scroll missing 'delta_y'"))
                })? as f32;
            ScriptEventKind::Scroll {
                target: parse_target(item, idx)?,
                delta_y,
            }
        }
        "key" => {
            let key_str = item
                .get("key")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| EngineError::Script(format!("event[{idx}] key missing 'key'")))?;
            let key = parse_key(key_str);
            let utf8 = item
                .get("utf8")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
                .or_else(|| match key {
                    Key::Char(c) => Some(c.to_string()),
                    Key::Space => Some(" ".to_owned()),
                    _ => None,
                });
            ScriptEventKind::Key {
                key,
                utf8,
                ctrl: item
                    .get("ctrl")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false),
                shift: item
                    .get("shift")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false),
                alt: item
                    .get("alt")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false),
                caps_lock: item.get("caps_lock").and_then(serde_json::Value::as_bool),
            }
        }
        "mark" => {
            let name = item
                .get("name")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| EngineError::Script(format!("event[{idx}] mark missing 'name'")))?
                .to_owned();
            ScriptEventKind::Mark { name }
        }
        other => {
            return Err(EngineError::Script(format!(
                "event[{idx}] unknown type '{other}'"
            )));
        }
    };

    Ok(ScriptEvent { t, kind })
}

fn parse_target(item: &serde_json::Value, idx: usize) -> Result<ScriptTarget, EngineError> {
    if let Some(key) = item.get("key").and_then(serde_json::Value::as_str) {
        return Ok(ScriptTarget::Key(key.to_owned()));
    }
    if let (Some(x), Some(y)) = (
        item.get("x").and_then(serde_json::Value::as_f64),
        item.get("y").and_then(serde_json::Value::as_f64),
    ) {
        return Ok(ScriptTarget::Point(x as f32, y as f32));
    }
    Err(EngineError::Script(format!(
        "event[{idx}] must specify 'key' or numeric 'x'/'y'"
    )))
}

fn parse_key(name: &str) -> Key {
    match name {
        "Escape" | "Esc" => Key::Escape,
        "Enter" | "Return" => Key::Enter,
        "Tab" => Key::Tab,
        "BackSpace" | "Backspace" => Key::BackSpace,
        "Delete" | "Del" => Key::Delete,
        "Left" | "ArrowLeft" => Key::Left,
        "Right" | "ArrowRight" => Key::Right,
        "Up" | "ArrowUp" => Key::Up,
        "Down" | "ArrowDown" => Key::Down,
        "Home" => Key::Home,
        "End" => Key::End,
        "Space" | " " => Key::Space,
        other => {
            let mut chars = other.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => Key::Char(c),
                _ => Key::Other,
            }
        }
    }
}

// ── Offscreen GPU renderer ───────────────────────────────────────────────────

/// Offscreen displayless renderer binding [`Egl::headless`] + [`Gpu`] to
/// render any [`HeadlessSession`] directly to straight RGBA8 or PNG files.
///
/// Field drop order is load-bearing: `gpu` drops before `egl` so GLES3 objects
/// are deleted while the headless EGL context is still live.
#[derive(Debug)]
pub struct HeadlessRenderer {
    gpu: Gpu,
    egl: Egl,
}

impl HeadlessRenderer {
    /// Initialize a displayless EGL 1.5 + GLES3 renderer.
    pub fn new() -> Result<Self, EngineError> {
        let egl = Egl::headless()?;
        egl.make_current(None)?;
        let gpu = Gpu::new(egl.glow())?;
        Ok(Self { gpu, egl })
    }

    /// Render the last frame recorded in `session` to straight RGBA8 pixels
    /// over a transparent background, returning `(pixel_width, pixel_height, rgba)`.
    pub fn render_rgba<M>(
        &mut self,
        session: &mut HeadlessSession<M>,
    ) -> Result<(u32, u32, Vec<u8>), EngineError> {
        self.render_rgba_with_backdrop(session, Rgba::TRANSPARENT)
    }

    /// Render the last frame recorded in `session` over an explicit `backdrop`
    /// color, returning `(pixel_width, pixel_height, rgba)`.
    pub fn render_rgba_with_backdrop<M>(
        &mut self,
        session: &mut HeadlessSession<M>,
        backdrop: Rgba,
    ) -> Result<(u32, u32, Vec<u8>), EngineError> {
        self.egl.make_current(None)?;
        let bw = (session.size.0 * session.scale).round().max(1.0) as u32;
        let bh = (session.size.1 * session.scale).round().max(1.0) as u32;
        let rgba = self.gpu.render_to_rgba_with_backdrop(
            bw,
            bh,
            backdrop,
            &session.draw_list,
            &mut session.text,
            &mut session.images,
        )?;
        Ok((bw, bh, rgba))
    }

    /// Render the last frame of `session` and write it as a PNG to `path`.
    pub fn snap_png<M>(
        &mut self,
        session: &mut HeadlessSession<M>,
        path: impl AsRef<Path>,
    ) -> Result<(u32, u32), EngineError> {
        self.snap_png_with_backdrop(session, Rgba::TRANSPARENT, path)
    }

    /// Render the last frame of `session` over `backdrop` and write it as a PNG to `path`.
    pub fn snap_png_with_backdrop<M>(
        &mut self,
        session: &mut HeadlessSession<M>,
        backdrop: Rgba,
        path: impl AsRef<Path>,
    ) -> Result<(u32, u32), EngineError> {
        let (w, h, rgba) = self.render_rgba_with_backdrop(session, backdrop)?;
        write_png(path, w, h, &rgba)?;
        Ok((w, h))
    }
}

// ── Zero-dependency safe PNG encoder (zlib stored blocks + CRC32 + Adler32) ──

/// Encode straight `RGBA8` pixels (top-to-bottom row order) into a valid PNG byte stream.
pub fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, EngineError> {
    if width == 0 || height == 0 {
        return Err(EngineError::Io("PNG dimensions must be non-zero".into()));
    }
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|px| px.checked_mul(4))
        .ok_or_else(|| EngineError::Io("PNG image dimensions overflow".into()))?;
    if rgba.len() != expected {
        return Err(EngineError::Io(format!(
            "PNG RGBA buffer length {} != expected {expected} for {width}x{height}",
            rgba.len()
        )));
    }

    let row_bytes = width as usize * 4;
    let raw_len = (row_bytes + 1) * height as usize;
    let mut scanlines = Vec::with_capacity(raw_len);
    for row in rgba.chunks_exact(row_bytes) {
        scanlines.push(0u8); // filter type 0 (None)
        scanlines.extend_from_slice(row);
    }

    // Wrap scanlines in a valid RFC 1950 zlib stream using uncompressed DEFLATE stored blocks.
    let block_count = scanlines.len().div_ceil(65_535).max(1);
    let mut zlib = Vec::with_capacity(2 + scanlines.len() + block_count * 5 + 4);
    zlib.push(0x78); // CMF: DEFLATE, 32K window
    zlib.push(0x01); // FLG: no dict, check bits so (0x78*256 + 0x01) % 31 == 0
    if scanlines.is_empty() {
        zlib.extend_from_slice(&[0x01, 0x00, 0x00, 0xFF, 0xFF]);
    } else {
        let total = scanlines.len();
        let mut offset = 0usize;
        while offset < total {
            let len = (total - offset).min(65_535);
            let is_final = u8::from(offset + len == total);
            let len_u16 = len as u16;
            let nlen_u16 = !len_u16;
            zlib.push(is_final);
            zlib.extend_from_slice(&len_u16.to_le_bytes());
            zlib.extend_from_slice(&nlen_u16.to_le_bytes());
            zlib.extend_from_slice(&scanlines[offset..offset + len]);
            offset += len;
        }
    }
    zlib.extend_from_slice(&adler32(&scanlines).to_be_bytes());

    let mut png = Vec::with_capacity(8 + 25 + 12 + zlib.len() + 12);
    png.extend_from_slice(&[137, 80, 78, 71, 13, 10, 26, 10]);

    let mut ihdr = [0u8; 13];
    ihdr[0..4].copy_from_slice(&width.to_be_bytes());
    ihdr[4..8].copy_from_slice(&height.to_be_bytes());
    ihdr[8] = 8; // bit depth
    ihdr[9] = 6; // color type: RGBA
    ihdr[10] = 0; // compression: DEFLATE
    ihdr[11] = 0; // filter: adaptive
    ihdr[12] = 0; // interlace: none
    write_png_chunk(&mut png, *b"IHDR", &ihdr);
    write_png_chunk(&mut png, *b"IDAT", &zlib);
    write_png_chunk(&mut png, *b"IEND", &[]);
    Ok(png)
}

/// Encode straight `RGBA8` pixels and write them to a PNG file at `path`.
pub fn write_png(
    path: impl AsRef<Path>,
    width: u32,
    height: u32,
    rgba: &[u8],
) -> Result<(), EngineError> {
    let bytes = encode_png(width, height, rgba)?;
    if let Some(parent) = path.as_ref().parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .map_err(|e| EngineError::Io(format!("create_dir_all {}: {e}", parent.display())))?;
    }
    std::fs::write(path.as_ref(), bytes)
        .map_err(|e| EngineError::Io(format!("write {}: {e}", path.as_ref().display())))
}

fn write_png_chunk(out: &mut Vec<u8>, chunk_type: [u8; 4], data: &[u8]) {
    let len = u32::try_from(data.len()).unwrap_or(u32::MAX);
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(&chunk_type);
    out.extend_from_slice(data);
    let crc = crc32_update(crc32_update(0xFFFF_FFFF, &chunk_type), data) ^ 0xFFFF_FFFF;
    out.extend_from_slice(&crc.to_be_bytes());
}

fn crc32_update(mut crc: u32, bytes: &[u8]) -> u32 {
    for &b in bytes {
        crc ^= u32::from(b);
        for _ in 0..8 {
            let mask = 0u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    crc
}

fn adler32(bytes: &[u8]) -> u32 {
    const MOD_ADLER: u32 = 65_521;
    let mut a = 1u32;
    let mut b = 0u32;
    for &byte in bytes {
        a = (a + u32::from(byte)) % MOD_ADLER;
        b = (b + a) % MOD_ADLER;
    }
    (b << 16) | a
}
