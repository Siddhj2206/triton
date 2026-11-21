//! The Triton shell process (ADR-0003): one `iced_layershell` daemon that owns every
//! shell surface. Starts with no surface; niri's outputs decide what appears. It
//! renders with wgpu on Vulkan, falling back to tiny-skia (ADR-0007).

mod app;
mod host;
mod launch;
mod msg;
mod niri;
mod niri_config;
mod notify;
mod panels;
mod pipeline;
mod polkit;
mod services;
mod shell;
mod wayl;

use iced_layershell::settings::{LayerShellSettings, Settings, StartMode};
use tracing_subscriber::EnvFilter;

use crate::app::Shell;

/// wgpu on Vulkan unless the user picked otherwise (ADR-0007). By default wgpu would
/// also set up its GL backend, and in spike 1 GL used about 20 MB more than Vulkan.
/// If no Vulkan adapter works, iced falls back to tiny-skia.
fn prefer_vulkan() {
    if std::env::var_os("WGPU_BACKEND").is_none() {
        // SAFETY: called first thing in `main`, before any other thread exists, so
        // nothing can read the environment concurrently.
        #[expect(unsafe_code, reason = "set_var is unsafe in edition 2024")]
        unsafe {
            std::env::set_var("WGPU_BACKEND", "vulkan");
        }
    }
}

fn main() -> Result<(), iced_layershell::Error> {
    // `triton msg …`: a command line to the running shell, for niri binds.
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() == Some("msg") {
        let command = args.collect::<Vec<_>>().join(" ");
        return match msg::send(&command) {
            Ok(()) => Ok(()),
            Err(e) => {
                eprintln!("triton msg {command}: {e} (is the shell running?)");
                std::process::exit(1);
            }
        };
    }
    prefer_vulkan();
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| "triton=info,warn".into()),
        )
        .init();
    std::panic::set_hook(Box::new(|info| {
        tracing::error!(%info, "panic");
    }));
    if let Err(e) = niri_config::install(&triton_design::Scheme::DARK) {
        tracing::warn!(%e, "cannot write the niri include files");
    }
    // Dev and preview run `Services::fake`; this opts into the real read-only
    // UPower and NetworkManager watchers.
    let real_services = std::env::args().any(|a| a == "--real-services");
    let base = triton_ui::settings();
    iced_layershell::daemon(
        move || Shell::boot(real_services),
        "triton",
        Shell::update,
        Shell::view,
    )
    .subscription(Shell::subscription)
    .theme(Shell::theme)
    .style(Shell::style)
    .settings(Settings {
        fonts: base.fonts,
        default_font: base.default_font,
        default_text_size: base.default_text_size,
        // wgpu antialiases paths (the wave, smoothed corners) only with MSAA;
        // without it their edges are jagged. Quads and text are smooth either way.
        antialiasing: true,
        layer_settings: LayerShellSettings {
            start_mode: StartMode::Background,
            ..LayerShellSettings::default()
        },
        ..Settings::default()
    })
    .run()
}
