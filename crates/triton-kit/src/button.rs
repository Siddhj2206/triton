//! Buttons: pill [`button`] and circle/squircle-morphing [`icon_button`].

use triton_design::color::STATE_LAYER;
use triton_design::icon::{Icon, IconStyle};
use triton_design::shape::{CORNER_SMOOTHING, RADII, SIZE};
use triton_design::typography::TypeStyle;
use triton_engine::style::{Rgba, Theme};
use triton_engine::ui::{Element, NodeKey, box_, icon, row, text};

/// Visual emphasis of a Triton [`button`] or [`icon_button`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ButtonTone {
    Filled,
    Tonal,
    Veil,
    Ghost,
    /// `error_container`/`on_error_container` — the only tone for destructive
    /// actions (dialogs, "Move to Trash"), never `Filled` primary.
    Destructive,
}

/// Triton M3 pill button with hover/press state layer and click callback.
pub fn button<M: 'static>(
    key: impl Into<NodeKey>,
    label: impl Into<String>,
    lead_icon: Option<Icon>,
    tone: ButtonTone,
    theme: &Theme,
    on_click: impl Fn() -> M + 'static,
) -> Element<M> {
    let (bg, fg) = tone_colors(tone, theme);
    let hover = bg.over(fg, STATE_LAYER.hover);
    let press = bg.over(fg, STATE_LAYER.press);

    let mut el = row()
        .id(key)
        .h(SIZE.control_m)
        .px(18.0)
        .gap(8.0)
        .items_center()
        .justify_center()
        .rounded(RADII.lg)
        .bg(bg)
        .hover_bg(hover)
        .press_bg(press)
        .on_click(on_click);

    if let Some(ic) = lead_icon {
        el = el.child(icon(ic, IconStyle::Outlined, SIZE.icon_m, fg));
    }
    el.child(text(label, TypeStyle::LABEL_L, fg))
}

/// Circular or squircle-morphing icon button (e.g., launcher button, settings gear, power).
#[allow(clippy::too_many_arguments)]
pub fn icon_button<M: 'static>(
    key: impl Into<NodeKey>,
    glyph: Icon,
    style: IconStyle,
    diameter: f32,
    toggled: bool,
    tone: ButtonTone,
    theme: &Theme,
    on_click: impl Fn() -> M + 'static,
) -> Element<M> {
    let (bg, fg) = if toggled {
        (
            Rgba::from_color(theme.scheme.primary),
            Rgba::from_color(theme.scheme.on_primary),
        )
    } else {
        tone_colors(tone, theme)
    };
    let hover = bg.over(fg, STATE_LAYER.hover);
    let press = bg.over(fg, STATE_LAYER.press);
    let radius = if toggled { RADII.sm } else { diameter * 0.5 };

    box_()
        .id(key)
        .size(diameter, diameter)
        .items_center()
        .justify_center()
        .rounded(radius)
        .smoothing(if toggled { CORNER_SMOOTHING } else { 0.0 })
        .bg(bg)
        .hover_bg(hover)
        .press_bg(press)
        .on_click(on_click)
        .child(icon(glyph, style, (diameter * 0.5).clamp(18.0, 24.0), fg))
}

fn tone_colors(tone: ButtonTone, theme: &Theme) -> (Rgba, Rgba) {
    match tone {
        ButtonTone::Filled => (
            Rgba::from_color(theme.scheme.primary),
            Rgba::from_color(theme.scheme.on_primary),
        ),
        ButtonTone::Tonal => (
            Rgba::from_color(theme.scheme.secondary_container),
            Rgba::from_color(theme.scheme.on_secondary_container),
        ),
        ButtonTone::Veil => (theme.veil(), Rgba::from_color(theme.scheme.on_surface)),
        ButtonTone::Ghost => (
            Rgba::TRANSPARENT,
            Rgba::from_color(theme.scheme.on_surface_variant),
        ),
        ButtonTone::Destructive => (
            Rgba::from_color(theme.scheme.error_container),
            Rgba::from_color(theme.scheme.on_error_container),
        ),
    }
}
