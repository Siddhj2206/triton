//! Bar-chrome pieces: the spring-morphing [`pill`] and the `.t-ic` [`icon_disc`].

use triton_design::color::STATE_LAYER;
use triton_design::icon::{Icon, IconStyle};
use triton_design::shape::{CORNER_SMOOTHING, SIZE};
use triton_engine::style::{FontSpec, Rgba, Theme};
use triton_engine::ui::{Element, NodeKey, TextAlign, box_, icon, text};

/// `.t-ic` 40px icon disc: `secondary_container` circle with a 22px
/// `on_secondary_container` glyph — the standard list-row leading icon.
pub fn icon_disc<M>(glyph: Icon, theme: &Theme) -> Element<M> {
    box_()
        .size(SIZE.control_m, SIZE.control_m)
        .items_center()
        .justify_center()
        .rounded(SIZE.control_m * 0.5)
        .smoothing(0.0)
        .bg(Rgba::from_color(theme.scheme.secondary_container))
        .child(icon(
            glyph,
            IconStyle::Outlined,
            22.0,
            Rgba::from_color(theme.scheme.on_secondary_container),
        ))
}

/// Spring-morphing workspace/status pill (width, corner radius, and color smoothly
/// interpolate with `morph` in `0.0..=1.0`).
#[allow(clippy::too_many_arguments)]
pub fn pill<M: 'static>(
    key: impl Into<NodeKey>,
    label: impl Into<String>,
    morph: f32,
    idle_w: f32,
    active_w: f32,
    height: f32,
    theme: &Theme,
    on_click: impl Fn() -> M + 'static,
) -> Element<M> {
    let m = morph.clamp(0.0, 1.0);
    let w = idle_w + (active_w - idle_w) * m;
    let idle_bg = theme.veil();
    let active_bg = Rgba::from_color(theme.scheme.primary);
    let bg = idle_bg.lerp(active_bg, m);
    let fg = Rgba::from_color(theme.scheme.on_surface_variant)
        .lerp(Rgba::from_color(theme.scheme.on_primary), m);
    let hover = bg.over(fg, STATE_LAYER.hover);
    let press = bg.over(fg, STATE_LAYER.press);
    let radius = height * 0.5 - (height * 0.5 - 12.0) * m;

    let mut font = FontSpec::display(16.0);
    font.weight = if m > 0.5 { 600 } else { 500 };

    box_()
        .id(key)
        .size(w, height)
        .items_center()
        .justify_center()
        .rounded(radius)
        .smoothing(CORNER_SMOOTHING * m)
        .bg(bg)
        .hover_bg(hover)
        .press_bg(press)
        .on_click(on_click)
        .child(text(label, font, fg).text_align(TextAlign::Center))
}
