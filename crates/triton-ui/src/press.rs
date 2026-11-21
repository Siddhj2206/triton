//! [`Pressable`]: Triton's button widget. Every button, chip, nav item, segment,
//! menu row and selection control is one.
//!
//! It replaces iced's button so state can move instead of jump:
//! - the hover and press layers fade on the fast effects spring;
//! - corner radii spring between shapes (press tightens a button to 12 px, a
//!   toggled icon button squares off) on the fast spatial spring;
//! - the switch thumb slides and grows on the default spatial spring.
//!
//! Reduce motion swaps every spring for the 120 ms fade. A pressable asks for frames
//! only while something is moving, so an idle shell draws nothing.

use std::cell::Cell;
use std::time::Instant;

use iced_core::alignment::Vertical;
use iced_core::layout::{self, Layout};
use iced_core::renderer::{self, Quad};
use iced_core::text::{self, LineHeight, Shaping, Wrapping};
use iced_core::widget::{Operation, Tree, Widget, tree};
use iced_core::{
    Background, Border, Clipboard, Color, Element, Event, Length, Padding, Point, Rectangle, Shell,
    Size, Vector, mouse, overlay, touch, window,
};
use iced_widget::Container;
use triton_design::color::{STATE_LAYER, with_alpha};
use triton_design::{Animated, Icon, IconStyle, Motion, SIZE, Spring};

use crate::button::{ButtonClass, Kind, Size as ControlSize};
use crate::paint::{self, Paint, PaintCache};
use crate::{Renderer, Theme};

/// Settling thresholds: a hundredth of a layer, a twentieth of a pixel.
const EPS_AMOUNT: f32 = 0.002;
const EPS_PX: f32 = 0.05;

/// Switch thumb diameters: off, on, and while pressed.
const THUMB_OFF: f32 = 16.0;
const THUMB_ON: f32 = 24.0;
const THUMB_PRESSED: f32 = 28.0;
/// Thumb centers from the track's left edge, off and on.
const THUMB_X_OFF: f32 = 16.0;
const THUMB_X_ON: f32 = 36.0;
const CHECK: f32 = 16.0;

pub struct Pressable<'a, M> {
    inner: Container<'a, M, Theme, Renderer>,
    class: ButtonClass,
    on_press: Option<M>,
}

impl<M> std::fmt::Debug for Pressable<'_, M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pressable")
            .field("class", &self.class)
            .field("enabled", &self.on_press.is_some())
            .finish_non_exhaustive()
    }
}

/// A pressable around `content`. Constructors in [`button`](crate::button) and
/// [`control`](crate::control) set its class.
pub(crate) fn pressable<'a, M>(
    content: impl Into<Element<'a, M, Theme, Renderer>>,
) -> Pressable<'a, M> {
    Pressable {
        inner: iced_widget::container(content),
        class: ButtonClass::Button {
            kind: Kind::Filled,
            size: ControlSize::M,
        },
        on_press: None,
    }
}

impl<M> Pressable<'_, M> {
    /// Without a message the control is disabled.
    #[must_use]
    pub fn on_press(mut self, message: M) -> Self {
        self.on_press = Some(message);
        self
    }

    #[must_use]
    pub fn on_press_maybe(mut self, message: Option<M>) -> Self {
        self.on_press = message;
        self
    }

    #[must_use]
    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.inner = self.inner.width(width);
        self
    }

    #[must_use]
    pub fn height(mut self, height: impl Into<Length>) -> Self {
        self.inner = self.inner.height(height);
        self
    }

    #[must_use]
    pub fn padding(mut self, padding: impl Into<Padding>) -> Self {
        self.inner = self.inner.padding(padding);
        self
    }

    #[must_use]
    pub(crate) fn class(mut self, class: ButtonClass) -> Self {
        self.class = class;
        self
    }

    fn switch_on(&self) -> Option<bool> {
        match self.class {
            ButtonClass::Switch { on } => Some(on),
            _ => None,
        }
    }
}

/// Where each animated quantity is headed.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Targets {
    hover: f32,
    press: f32,
    radii: [f32; 4],
    on: f32,
}

#[derive(Debug)]
struct Motions {
    hover: Animated,
    press: Animated,
    radii: [Animated; 4],
    on: Animated,
}

impl Motions {
    fn new(t: Targets, reduce: bool, now: Instant) -> Motions {
        let fx = Motion::pick(Spring::EFFECTS_FAST, reduce);
        let shape = Motion::pick(Spring::SPATIAL_FAST, reduce);
        let slide = Motion::pick(Spring::SPATIAL, reduce);
        Motions {
            hover: Animated::new(t.hover, fx, EPS_AMOUNT, now),
            press: Animated::new(t.press, fx, EPS_AMOUNT, now),
            radii: t.radii.map(|r| Animated::new(r, shape, EPS_PX, now)),
            on: Animated::new(t.on, slide, EPS_AMOUNT, now),
        }
    }

    fn all(&mut self) -> impl Iterator<Item = &mut Animated> {
        [&mut self.hover, &mut self.press, &mut self.on]
            .into_iter()
            .chain(self.radii.iter_mut())
    }

    /// Retargets; true if anything changed.
    fn retarget(&mut self, t: Targets, now: Instant) -> bool {
        let goals = [t.hover, t.press, t.on].into_iter().chain(t.radii);
        let mut changed = false;
        for (anim, goal) in self.all().zip(goals) {
            if (anim.target() - goal).abs() > f32::EPSILON {
                anim.set_target(goal, now);
                changed = true;
            }
        }
        changed
    }

    fn settled(&mut self, now: Instant) -> bool {
        self.all().all(|a| a.is_settled(now))
    }

    fn sample(&self, now: Instant) -> Targets {
        Targets {
            hover: self.hover.value(now),
            press: self.press.value(now),
            radii: [0, 1, 2, 3].map(|i| self.radii[i].value(now)),
            on: self.on.value(now),
        }
    }
}

#[derive(Debug, Default)]
struct State {
    hovered: bool,
    pressed: bool,
    motions: Option<Motions>,
    /// The frame time the motions were last sampled at.
    now: Option<Instant>,
    /// Reduce motion, as of the last draw (update has no theme).
    reduce: Cell<bool>,
    cache: PaintCache,
}

impl<M> Pressable<'_, M> {
    fn targets(&self, hovered: bool, pressed: bool) -> Targets {
        let enabled = self.on_press.is_some();
        Targets {
            hover: f32::from(u8::from(hovered && enabled)),
            press: f32::from(u8::from(pressed && enabled)),
            radii: self.class.radii(pressed && enabled),
            on: f32::from(u8::from(self.switch_on() == Some(true))),
        }
    }

    fn animate(&self, state: &mut State, now: Instant, frame: bool, shell: &mut Shell<'_, M>) {
        let targets = self.targets(state.hovered, state.pressed);
        let reduce = state.reduce.get();
        let Some(motions) = state.motions.as_mut() else {
            state.motions = Some(Motions::new(targets, reduce, now));
            state.now = Some(now);
            return;
        };
        if motions.retarget(targets, now) {
            shell.request_redraw();
        }
        if frame {
            state.now = Some(now);
            if !motions.settled(now) {
                shell.request_redraw();
            }
        }
    }
}

fn dim(c: Color) -> Color {
    with_alpha(c, STATE_LAYER.disabled_content)
}

/// The switch: a track whose colors follow `on`, and a thumb that slides from the
/// left dot to the right disc, growing while pressed, with the check fading in.
fn draw_switch(
    renderer: &mut Renderer,
    theme: &Theme,
    bounds: Rectangle,
    t: Targets,
    enabled: bool,
    cache: &PaintCache,
) {
    use iced_core::Renderer as _;
    use iced_core::text::Renderer as _;
    let mut colors = theme.switch_colors(t.on);
    if !enabled {
        colors = colors.map(dim);
    }
    let [track, outline, thumb, check] = colors;
    let layer = STATE_LAYER.hover * t.hover + (STATE_LAYER.press - STATE_LAYER.hover) * t.press;
    let paint = Paint {
        fill: Some(triton_design::color::over(
            track,
            theme.scheme.on_surface,
            layer,
        )),
        border: Some((outline, 2.0)),
        radii: t.radii,
        smooth: false,
        ..Paint::default()
    };
    let _ = paint::draw(renderer, bounds, paint, cache);

    let rest = THUMB_OFF + (THUMB_ON - THUMB_OFF) * t.on.clamp(0.0, 1.0);
    let d = rest + (THUMB_PRESSED - rest) * t.press;
    let cx = bounds.x + THUMB_X_OFF + (THUMB_X_ON - THUMB_X_OFF) * t.on;
    let cy = bounds.center_y();
    renderer.fill_quad(
        Quad {
            bounds: Rectangle::new(Point::new(cx - d / 2.0, cy - d / 2.0), Size::new(d, d)),
            border: Border::default().rounded(d / 2.0),
            snap: false,
            ..Quad::default()
        },
        Background::Color(thumb),
    );
    if check.a > 0.01 {
        renderer.fill_text(
            text::Text {
                content: Icon::Check.codepoint().to_string(),
                bounds: Size::new(d, d),
                size: CHECK.into(),
                line_height: LineHeight::Absolute(CHECK.into()),
                font: Icon::font(IconStyle::Filled),
                align_x: text::Alignment::Center,
                align_y: Vertical::Center,
                shaping: Shaping::Basic,
                wrapping: Wrapping::None,
            },
            Point::new(cx, cy),
            check,
            bounds,
        );
    }
}

impl<M: Clone> Widget<M, Theme, Renderer> for Pressable<'_, M> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.inner as &dyn Widget<M, Theme, Renderer>)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(&[&self.inner as &dyn Widget<M, Theme, Renderer>]);
    }

    fn size(&self) -> Size<Length> {
        self.inner.size()
    }

    fn size_hint(&self) -> Size<Length> {
        self.inner.size_hint()
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.inner.layout(&mut tree.children[0], renderer, limits)
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        self.inner
            .operate(&mut tree.children[0], layout, renderer, operation);
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, M>,
        viewport: &Rectangle,
    ) {
        self.inner.update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
        if shell.is_event_captured() {
            return;
        }
        let state = tree.state.downcast_mut::<State>();
        let bounds = layout.bounds();
        let over = cursor.is_over(bounds);
        let enabled = self.on_press.is_some();
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
            | Event::Touch(touch::Event::FingerPressed { .. })
                if enabled && over =>
            {
                state.pressed = true;
                shell.capture_event();
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
            | Event::Touch(touch::Event::FingerLifted { .. })
                if state.pressed =>
            {
                state.pressed = false;
                if over && let Some(message) = self.on_press.clone() {
                    shell.publish(message);
                    shell.capture_event();
                }
            }
            Event::Touch(touch::Event::FingerLost { .. }) => state.pressed = false,
            _ => {}
        }
        state.hovered = over;
        let (now, frame) = match event {
            Event::Window(window::Event::RedrawRequested(now)) => (*now, true),
            _ => (Instant::now(), false),
        };
        self.animate(state, now, frame, shell);
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_ref::<State>();
        state.reduce.set(theme.reduce_motion);
        let bounds = layout.bounds();
        let enabled = self.on_press.is_some();
        let t = match (&state.motions, state.now) {
            (Some(m), Some(now)) => m.sample(now),
            _ => self.targets(false, false),
        };
        let visible = bounds.intersects(viewport);
        let (content, layered) = if self.switch_on().is_some() {
            if visible {
                draw_switch(renderer, theme, bounds, t, enabled, &state.cache);
            }
            (style.text_color, false)
        } else {
            let (paint, content) =
                theme.press_paint(self.class, t.hover, t.press, !enabled, t.radii);
            let layered = visible && paint::draw(renderer, bounds, paint, &state.cache);
            (content, layered)
        };
        let style = renderer::Style {
            text_color: content,
        };
        let draw = |r: &mut Renderer| {
            self.inner.draw(
                &tree.children[0],
                r,
                theme,
                &style,
                layout,
                cursor,
                viewport,
            );
        };
        if layered {
            iced_core::Renderer::with_layer(renderer, *viewport, draw);
        } else {
            draw(renderer);
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        if self.on_press.is_some() && cursor.is_over(layout.bounds()) {
            mouse::Interaction::Pointer
        } else {
            self.inner
                .mouse_interaction(&tree.children[0], layout, cursor, viewport, renderer)
        }
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, M, Theme, Renderer>> {
        self.inner.overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}

impl<'a, M: Clone + 'a> From<Pressable<'a, M>> for Element<'a, M, Theme, Renderer> {
    fn from(p: Pressable<'a, M>) -> Self {
        Element::new(p)
    }
}

// The on thumb keeps 4 px clear of the track's right edge.
const _: () = assert!(THUMB_X_ON + THUMB_ON / 2.0 + 4.0 <= SIZE.switch.0);
