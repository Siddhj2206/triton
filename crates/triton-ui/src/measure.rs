//! [`Measure`]: lays its content out at its natural size on one axis and reports
//! it, so a layer surface can be sized to fit what it shows (a bar island is its
//! own surface, and niri blurs and shadows the whole surface).
//!
//! The other axis stays bounded by the surface; a report is sent only when the
//! measured extent changes.

use std::cell::Cell;

use iced_core::layout::{self, Layout};
use iced_core::widget::{Tree, Widget, tree};
use iced_core::{Length, Rectangle, Size, Vector, mouse, renderer};

use crate::{Element, Renderer, Theme};

/// The axis `Measure` lets run free: width for the islands, height for panels.
#[derive(Debug, Clone, Copy)]
enum Dim {
    Width,
    Height,
}

pub struct Measure<'a, M> {
    content: Element<'a, M>,
    on_extent: Box<dyn Fn(f32) -> M + 'a>,
    dim: Dim,
}

impl<M> std::fmt::Debug for Measure<'_, M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Measure").finish_non_exhaustive()
    }
}

/// Wraps `content`; `on_width` gets its natural width whenever that changes.
pub fn measure<'a, M>(
    content: impl Into<Element<'a, M>>,
    on_width: impl Fn(f32) -> M + 'a,
) -> Measure<'a, M> {
    Measure {
        content: content.into(),
        on_extent: Box::new(on_width),
        dim: Dim::Width,
    }
}

/// Wraps `content`; `on_height` gets its natural height at the surface's width
/// whenever that changes.
pub fn measure_height<'a, M>(
    content: impl Into<Element<'a, M>>,
    on_height: impl Fn(f32) -> M + 'a,
) -> Measure<'a, M> {
    Measure {
        content: content.into(),
        on_extent: Box::new(on_height),
        dim: Dim::Height,
    }
}

#[derive(Debug, Default)]
struct State {
    measured: Cell<f32>,
    reported: Option<f32>,
}

impl<M> Widget<M, Theme, Renderer> for Measure<'_, M> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Shrink, Length::Shrink)
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let free = match self.dim {
            Dim::Width => Size::new(f32::INFINITY, limits.max().height),
            Dim::Height => Size::new(limits.max().width, f32::INFINITY),
        };
        let loose = layout::Limits::new(Size::ZERO, free);
        let node = self
            .content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, &loose);
        let extent = match self.dim {
            Dim::Width => node.size().width,
            Dim::Height => node.size().height,
        };
        tree.state.downcast_ref::<State>().measured.set(extent);
        node
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn iced_core::widget::Operation,
    ) {
        self.content
            .as_widget_mut()
            .operate(&mut tree.children[0], layout, renderer, operation);
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &iced_core::Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn iced_core::Clipboard,
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
        let state = tree.state.downcast_mut::<State>();
        let extent = state.measured.get();
        if state.reported.is_none_or(|r| (r - extent).abs() >= 0.5) {
            state.reported = Some(extent);
            shell.publish((self.on_extent)(extent));
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
        viewport: &Rectangle,
    ) {
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
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

impl<'a, M: 'a> From<Measure<'a, M>> for Element<'a, M> {
    fn from(m: Measure<'a, M>) -> Self {
        iced_core::Element::new(m)
    }
}
