//! Scrollbars and dividers.
//!
//! The scrollbar is a 4 px thumb, 4 px from the edge, no rail: on-surface at 50%.
//! Over it (or dragging) it widens to 10 px and 75%, on the fast spatial spring.

use std::cell::Cell;
use std::time::Instant;

use iced_core::Renderer as _;
use iced_core::event::Event;
use iced_core::layout::{self, Layout};
use iced_core::mouse::{self, ScrollDelta};
use iced_core::renderer::{self, Quad};
use iced_core::widget::{self, Tree, Widget};
use iced_core::window;
use iced_core::{
    Background, Border, Clipboard, Color, Element, Length, Point, Rectangle, Size, Vector,
};
use iced_widget::rule::{self, FillMode};
use triton_design::color::with_alpha;
use triton_design::material::veil_hi;
use triton_design::{Animated, Motion, Spring};

use crate::{Renderer, Theme};

const THUMB: f32 = 4.0;
const THUMB_HI: f32 = 10.0;
const MARGIN: f32 = 4.0;
const MIN_THUMB: f32 = 24.0;
const REST: f32 = 0.5;
const HOVER: f32 = 0.75;
/// A line-delta wheel tick scrolls this far.
const STEP: f32 = 48.0;
/// The hover/grab band, this wide off the right edge.
const BAND: f32 = THUMB_HI + MARGIN * 2.0;
const EPS: f32 = 0.001;

/// Vertical scrolling with Triton's scrollbar.
pub fn scroll<'a, M: 'a>(content: impl Into<Element<'a, M, Theme, Renderer>>) -> Scroll<'a, M> {
    Scroll {
        content: content.into(),
        width: Length::Fill,
        height: Length::Fill,
    }
}

/// Our own scrollable: iced's fixes the rail width at build time, so the
/// widening thumb needed a widget. Vertical only; no rail, no auto-scroll.
pub struct Scroll<'a, M> {
    content: Element<'a, M, Theme, Renderer>,
    width: Length,
    height: Length,
}

impl<M> Scroll<'_, M> {
    /// The view's width; `Fill` by default.
    #[must_use]
    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.width = width.into();
        self
    }

    /// The view's height; `Fill` by default.
    #[must_use]
    pub fn height(mut self, height: impl Into<Length>) -> Self {
        self.height = height.into();
        self
    }
}

impl<M> std::fmt::Debug for Scroll<'_, M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Scroll").finish_non_exhaustive()
    }
}

#[derive(Debug, Default)]
struct ScrollState {
    offset: f32,
    /// Where in the thumb the press grabbed it (px from its top).
    drag: Option<f32>,
    /// Pointer over the band, or a drag in flight.
    hot: bool,
    widen: Option<Animated>,
    now: Option<Instant>,
    reduce: Cell<bool>,
}

/// View metrics: `Some((max_offset, thumb_h, thumb_travel))` when scrollable.
fn metrics(view_h: f32, content_h: f32) -> Option<(f32, f32, f32)> {
    let max = content_h - view_h;
    (max > 1.0).then(|| {
        let thumb_h = (view_h * view_h / content_h)
            .clamp(0.0, view_h)
            .max(MIN_THUMB);
        (max, thumb_h.min(view_h), view_h - thumb_h.min(view_h))
    })
}

/// The band the pointer widens and can drag in.
fn band(bounds: Rectangle) -> Rectangle {
    Rectangle {
        x: bounds.x + bounds.width - BAND,
        width: BAND,
        ..bounds
    }
}

impl<M> Scroll<'_, M> {
    /// The thumb rect at offset `t` of the widen animation.
    fn thumb(bounds: Rectangle, off: f32, content_h: f32, t: f32) -> Option<Rectangle> {
        let (max, thumb_h, travel) = metrics(bounds.height, content_h)?;
        let w = THUMB + (THUMB_HI - THUMB) * t;
        Some(Rectangle {
            x: bounds.x + bounds.width - MARGIN - w,
            y: bounds.y + off.clamp(0.0, max) / max * travel,
            width: w,
            height: thumb_h,
        })
    }
}

impl<M: Clone> Widget<M, Theme, Renderer> for Scroll<'_, M> {
    fn tag(&self) -> widget::tree::Tag {
        widget::tree::Tag::of::<ScrollState>()
    }

    fn state(&self) -> widget::tree::State {
        widget::tree::State::new(ScrollState::default())
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(self.content.as_widget())]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(&[self.content.as_widget()]);
    }

    fn size(&self) -> Size<Length> {
        Size::new(self.width, self.height)
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let child_limits =
            layout::Limits::new(Size::ZERO, Size::new(limits.max().width, f32::INFINITY));
        let child = self
            .content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, &child_limits)
            .move_to(Point::ORIGIN);
        let size = limits.resolve(self.width, self.height, Size::ZERO);
        layout::Node::with_children(size, vec![child])
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        self.content
            .as_widget_mut()
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
        shell: &mut iced_core::Shell<'_, M>,
        viewport: &Rectangle,
    ) {
        self.content.as_widget_mut().update(
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
        let state = tree.state.downcast_mut::<ScrollState>();
        let bounds = layout.bounds();
        let content_h = layout.children().next().map_or(0.0, |l| l.bounds().height);
        let m = metrics(bounds.height, content_h);
        let t = state
            .widen
            .as_ref()
            .map_or(0.0, |w| w.value(state.now.unwrap_or_else(Instant::now)));
        match event {
            Event::Mouse(mouse::Event::WheelScrolled { delta }) if cursor.is_over(bounds) => {
                if let Some((max, _, _)) = m {
                    let dy = match delta {
                        ScrollDelta::Lines { y, .. } => -y * STEP,
                        ScrollDelta::Pixels { y, .. } => -y,
                    };
                    state.offset = (state.offset + dy).clamp(0.0, max);
                    shell.capture_event();
                }
            }
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
                if m.is_some() && cursor.is_over(band(bounds)) =>
            {
                if let Some(thumb) = Self::thumb(bounds, state.offset, content_h, t)
                    && let (Some(p), Some((max, _, travel))) = (cursor.position(), m)
                {
                    // A press on the thumb drags; beside it centers the thumb.
                    let grab = if thumb.contains(p) {
                        p.y - thumb.y
                    } else {
                        thumb.height / 2.0
                    };
                    state.offset = ((p.y - grab - bounds.y) / travel * max).clamp(0.0, max);
                    state.drag = Some(grab);
                    shell.capture_event();
                }
            }
            Event::Mouse(mouse::Event::CursorMoved { position }) => {
                if let (Some(grab), Some((max, _, travel))) = (state.drag, m) {
                    state.offset = ((position.y - grab - bounds.y) / travel * max).clamp(0.0, max);
                    shell.capture_event();
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                state.drag = None;
            }
            _ => {}
        }
        state.hot = m.is_some() && (state.drag.is_some() || cursor.is_over(band(bounds)));
        let (now, frame) = match event {
            Event::Window(window::Event::RedrawRequested(now)) => (*now, true),
            _ => (Instant::now(), false),
        };
        let target = f32::from(u8::from(state.hot));
        let widen = state.widen.get_or_insert_with(|| {
            Animated::new(
                target,
                Motion::pick(Spring::SPATIAL_FAST, state.reduce.get()),
                EPS,
                now,
            )
        });
        if (widen.target() - target).abs() > f32::EPSILON {
            widen.set_target(target, now);
            shell.request_redraw();
        }
        if frame {
            state.now = Some(now);
            if !widen.is_settled(now) {
                shell.request_redraw();
            }
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
        self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        )
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_ref::<ScrollState>();
        state.reduce.set(theme.reduce_motion);
        let bounds = layout.bounds();
        let Some(content) = layout.children().next() else {
            return;
        };
        let content_h = content.bounds().height;
        let now = state.now.unwrap_or_else(Instant::now);
        let t = state.widen.as_ref().map_or(0.0, |w| w.value(now));
        let offset = metrics(bounds.height, content_h)
            .map_or(0.0, |(max, _, _)| state.offset.clamp(0.0, max));
        let viewport_shift = Rectangle {
            y: bounds.y + offset,
            ..bounds
        };
        iced_core::Renderer::with_layer(renderer, bounds, |renderer| {
            iced_core::Renderer::with_translation(
                renderer,
                Vector::new(0.0, -offset),
                |renderer| {
                    self.content.as_widget().draw(
                        &tree.children[0],
                        renderer,
                        theme,
                        style,
                        content,
                        cursor,
                        &viewport_shift,
                    );
                },
            );
        });
        if let Some(thumb) = Self::thumb(bounds, offset, content_h, t) {
            let w = thumb.width;
            renderer.fill_quad(
                Quad {
                    bounds: thumb,
                    border: Border::default().rounded(w / 2.0),
                    snap: true,
                    ..Quad::default()
                },
                Background::Color(with_alpha(
                    theme.scheme.on_surface,
                    REST + (HOVER - REST) * t,
                )),
            );
        }
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<iced_core::overlay::Element<'b, M, Theme, Renderer>> {
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}

impl<'a, M: Clone + 'a> From<Scroll<'a, M>> for Element<'a, M, Theme, Renderer> {
    fn from(scroll: Scroll<'a, M>) -> Self {
        iced_core::Element::new(scroll)
    }
}

/// What a divider sits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Divider {
    /// outline-variant, 1 px.
    #[default]
    Solid,
    /// On Glass (menus): the veil highlight.
    Glass,
}

impl rule::Catalog for Theme {
    type Class<'a> = Divider;

    fn default<'a>() -> Self::Class<'a> {
        Divider::Solid
    }

    fn style(&self, class: &Divider) -> rule::Style {
        let color: Color = match class {
            Divider::Solid => self.scheme.outline_variant,
            Divider::Glass => veil_hi(self.variant),
        };
        rule::Style {
            color,
            radius: 0.0.into(),
            fill_mode: FillMode::Full,
            snap: true,
        }
    }
}

/// A 1 px horizontal divider.
pub fn divider<'a>(on: Divider) -> iced_widget::Rule<'a, Theme> {
    iced_widget::rule::horizontal(1).class(on)
}
