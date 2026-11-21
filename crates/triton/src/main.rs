//! The Triton shell process (ADR-0003): one `iced_layershell` daemon that owns every
//! shell surface. Starts with no surface; niri's outputs decide what appears.

mod app;
mod host;
mod niri;
mod niri_config;
mod shell;

use iced_layershell::settings::{LayerShellSettings, Settings, StartMode};
use tracing_subscriber::EnvFilter;

use crate::app::Shell;

fn main() -> Result<(), iced_layershell::Error> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    if let Err(e) = niri_config::install() {
        tracing::warn!(%e, "cannot write the niri include files");
    }
    let base = triton_ui::settings();
    iced_layershell::daemon(Shell::boot, "triton", Shell::update, Shell::view)
        .subscription(Shell::subscription)
        .theme(Shell::theme)
        .style(Shell::style)
        .settings(Settings {
            fonts: base.fonts,
            default_font: base.default_font,
            default_text_size: base.default_text_size,
            layer_settings: LayerShellSettings {
                start_mode: StartMode::Background,
                ..LayerShellSettings::default()
            },
            ..Settings::default()
        })
        .run()
}
