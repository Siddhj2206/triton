//! [`Measure`]: lays its content out at its natural width and reports that width,
//! so a layer surface can be sized to fit what it shows (a bar island is its own
//! surface, and niri blurs and shadows the whole surface).
//!
//! The content is laid out with no width limit, whatever size the surface has at
//! the moment; a report is sent only when the width changes.

use std::cell::Cell;

use iced_core::layout::{self, Layout};
use iced_core::widget::{Tree, Widget, tree};
use iced_core::{Length, Rectangle, Size, Vector, mouse, renderer};

use crate::{Element, Renderer, Theme};

pub struct Measure<'a, M> {
    content: Element<'a, M>,
    on_width: Box<dyn Fn(f32) -> M + 'a>,
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
        on_width: Box::new(on_width),
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
        let loose = layout::Limits::new(Size::ZERO, Size::new(f32::INFINITY, limits.max().height));
        let node = self
            .content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, &loose);
        tree.state
            .downcast_ref::<State>()
            .measured
            .set(node.size().width);
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
        let width = state.measured.get();
        if state.reported.is_none_or(|r| (r - width).abs() >= 0.5) {
            state.reported = Some(width);
            shell.publish((self.on_width)(width));
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
