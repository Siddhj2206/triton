//! `.t-slider`: the Material 3 Expressive slider.
//!
//! 44 px tall: a 16 px track split at the value by a 4 × 44 handle, with a 4 px gap
//! on each side of the handle. The active part has 8 px outer and 2 px inner
//! corners. iced's slider cannot draw the gap or the split radii, so this is our
//! own widget. The value is always `0.0..=1.0`; services map their own ranges.
//!
//! Input: press or drag anywhere on it, scroll over it (no modifier: a shell slider
//! under the pointer is meant to be scrolled), arrow keys while hovered.

use iced_core::border::Radius;
use iced_core::keyboard::{self, key};
use iced_core::layout::{self, Layout};
use iced_core::mouse;
use iced_core::renderer::{self, Quad};
use iced_core::touch;
use iced_core::widget::{Tree, Widget, tree};
use iced_core::{
    Background, Border, Clipboard, Color, Element, Event, Length, Rectangle, Shell, Size,
};
use triton_design::SIZE;
use triton_design::color::with_alpha;

use crate::{Renderer, Theme};

/// Value change per scroll line and per arrow key.
const STEP: f32 = 0.05;
/// Pixels of smooth scrolling per [`STEP`].
const PIXELS_PER_STEP: f32 = 40.0;
/// Space between the handle and each part of the track.
const GAP: f32 = 4.0;
/// Outer and inner corner radii of the track parts.
const OUTER: f32 = 8.0;
const INNER: f32 = 2.0;
/// On Glass the inactive track is the content color at this alpha.
const GLASS_TRACK: f32 = 0.2;

pub struct Slider<'a, M> {
    value: f32,
    on_change: Box<dyn Fn(f32) -> M + 'a>,
    on_release: Option<M>,
    on_glass: bool,
    width: Length,
}

/// A slider at `value` (`0.0..=1.0`).
pub fn slider<'a, M>(value: f32, on_change: impl Fn(f32) -> M + 'a) -> Slider<'a, M> {
    Slider {
        value: value.clamp(0.0, 1.0),
        on_change: Box::new(on_change),
        on_release: None,
        on_glass: false,
        width: Length::Fill,
    }
}

impl<M> std::fmt::Debug for Slider<'_, M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Slider")
            .field("value", &self.value)
            .field("on_glass", &self.on_glass)
            .finish_non_exhaustive()
    }
}

impl<M> Slider<'_, M> {
    /// Sent once when a drag ends (apply-on-release settings).
    #[must_use]
    pub fn on_release(mut self, message: M) -> Self {
        self.on_release = Some(message);
        self
    }

    /// The slider sits on Glass: the inactive track becomes a translucent veil.
    #[must_use]
    pub fn on_glass(mut self, on_glass: bool) -> Self {
        self.on_glass = on_glass;
        self
    }

    #[must_use]
    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.width = width.into();
        self
    }

    fn emit(&self, value: f32, shell: &mut Shell<'_, M>) {
        let value = value.clamp(0.0, 1.0);
        if (value - self.value).abs() > f32::EPSILON {
            shell.publish((self.on_change)(value));
        }
    }
}

#[derive(Debug, Default)]
struct State {
    dragging: bool,
    /// Smooth-scroll pixels not yet turned into a step.
    scroll: f32,
}

/// The value under `x` within `bounds`.
fn locate(bounds: Rectangle, x: f32) -> f32 {
    if bounds.width <= 0.0 {
        return 0.0;
    }
    ((x - bounds.x) / bounds.width).clamp(0.0, 1.0)
}

/// Track and handle rectangles for `value` in `bounds`: (active, inactive, handle).
/// A part shorter than its own corner is dropped rather than drawn inverted.
fn parts(bounds: Rectangle, value: f32) -> (Option<Rectangle>, Option<Rectangle>, Rectangle) {
    let (handle_w, handle_h) = SIZE.slider_handle;
    let track = SIZE.slider_track;
    let half = handle_w / 2.0;
    let x = bounds.x + half + (bounds.width - handle_w) * value;
    let track_y = bounds.center_y() - track / 2.0;
    let handle = Rectangle {
        x: x - half,
        y: bounds.center_y() - handle_h / 2.0,
        width: handle_w,
        height: handle_h,
    };
    let active_w = x - half - GAP - bounds.x;
    let inactive_x = x + half + GAP;
    let inactive_w = bounds.x + bounds.width - inactive_x;
    let min = INNER * 2.0;
    let active = (active_w >= min).then_some(Rectangle {
        x: bounds.x,
        y: track_y,
        width: active_w,
        height: track,
    });
    let inactive = (inactive_w >= min).then_some(Rectangle {
        x: inactive_x,
        y: track_y,
        width: inactive_w,
        height: track,
    });
    (active, inactive, handle)
}

fn quad(renderer: &mut Renderer, bounds: Rectangle, radius: Radius, color: Color) {
    use iced_core::Renderer as _;
    renderer.fill_quad(
        Quad {
            bounds,
            border: Border {
                radius,
                ..Border::default()
            },
            snap: true,
            ..Quad::default()
        },
        Background::Color(color),
    );
}

impl<M: Clone> Widget<M, Theme, Renderer> for Slider<'_, M> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn size(&self) -> Size<Length> {
        Size::new(self.width, Length::Fixed(SIZE.slider_handle.1))
    }

    fn layout(
        &mut self,
        _tree: &mut Tree,
        _renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        layout::atomic(limits, self.width, SIZE.slider_handle.1)
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, M>,
        _viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_mut::<State>();
        let bounds = layout.bounds();
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
            | Event::Touch(touch::Event::FingerPressed { .. }) => {
                if let Some(p) = cursor.position_over(bounds) {
                    state.dragging = true;
                    self.emit(locate(bounds, p.x), shell);
                    shell.capture_event();
                }
            }
            Event::Mouse(mouse::Event::CursorMoved { .. })
            | Event::Touch(touch::Event::FingerMoved { .. }) => {
                if state.dragging {
                    if let Some(p) = cursor.land().position() {
                        self.emit(locate(bounds, p.x), shell);
                    }
                    shell.capture_event();
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
            | Event::Touch(touch::Event::FingerLifted { .. } | touch::Event::FingerLost { .. }) => {
                if state.dragging {
                    state.dragging = false;
                    if let Some(m) = self.on_release.clone() {
                        shell.publish(m);
                    }
                }
            }
            Event::Mouse(mouse::Event::WheelScrolled { delta }) if cursor.is_over(bounds) => {
                let steps = match *delta {
                    mouse::ScrollDelta::Lines { y, .. } => y,
                    mouse::ScrollDelta::Pixels { y, .. } => {
                        state.scroll += y;
                        let whole = (state.scroll / PIXELS_PER_STEP).trunc();
                        state.scroll -= whole * PIXELS_PER_STEP;
                        whole
                    }
                };
                if steps != 0.0 {
                    self.emit(self.value + steps * STEP, shell);
                }
                shell.capture_event();
            }
            Event::Keyboard(keyboard::Event::KeyPressed {
                key: keyboard::Key::Named(named),
                ..
            }) if cursor.is_over(bounds) => {
                let delta = match named {
                    key::Named::ArrowRight | key::Named::ArrowUp => STEP,
                    key::Named::ArrowLeft | key::Named::ArrowDown => -STEP,
                    _ => return,
                };
                self.emit(self.value + delta, shell);
                shell.capture_event();
            }
            _ => {}
        }
    }

    fn draw(
        &self,
        _tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
        let s = &theme.scheme;
        let (active, inactive, handle) = parts(layout.bounds(), self.value);
        let rest = if self.on_glass {
            with_alpha(s.on_surface, GLASS_TRACK)
        } else {
            s.surface_container_highest
        };
        if let Some(r) = active {
            let radius = Radius {
                top_left: OUTER,
                top_right: INNER,
                bottom_right: INNER,
                bottom_left: OUTER,
            };
            quad(renderer, r, radius, s.primary);
        }
        if let Some(r) = inactive {
            let radius = Radius {
                top_left: INNER,
                top_right: OUTER,
                bottom_right: OUTER,
                bottom_left: INNER,
            };
            quad(renderer, r, radius, rest);
        }
        quad(
            renderer,
            handle,
            Radius::from(handle.width / 2.0),
            s.primary,
        );
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &Renderer,
    ) -> mouse::Interaction {
        if tree.state.downcast_ref::<State>().dragging {
            mouse::Interaction::Grabbing
        } else if cursor.is_over(layout.bounds()) {
            mouse::Interaction::Pointer
        } else {
            mouse::Interaction::default()
        }
    }
}

impl<'a, M: Clone + 'a> From<Slider<'a, M>> for Element<'a, M, Theme, Renderer> {
    fn from(slider: Slider<'a, M>) -> Self {
        Element::new(slider)
    }
}
