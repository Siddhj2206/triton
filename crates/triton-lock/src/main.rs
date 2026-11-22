//! The Triton lock screen (roadmap §3): a separate process because a lock
//! client that crashes must leave the session locked, not take the shell
//! down with it (spike 3 proved niri keeps the lock up).
//!
//! `ext-session-lock-v1` over `iced_sessionlock`, PAM auth via dlopened
//! libpam (`pam`), the Lock board on every output.

mod app;
mod pam;
mod user;

use iced_sessionlock::Settings;
use tracing_subscriber::EnvFilter;

const HELP: &str = "\
triton-lock — the Triton lock screen (ext-session-lock-v1 + PAM)

USAGE:
    triton-lock [OPTIONS]

OPTIONS:
    --demo   Skip PAM entirely: any non-empty password unlocks. For the
             headless preview, where the system PAM stack cannot
             authenticate a container user. NEVER in a real session.
    --help   Print this text.
";

/// wgpu on Vulkan unless the user picked otherwise (ADR-0007). When the
/// workspace build gives this binary iced's wgpu renderer, that avoids
/// setting up the GL backend; a tiny-skia-only build ignores it.
fn prefer_vulkan() {
    if std::env::var_os("WGPU_BACKEND").is_none() {
        // SAFETY: called first thing in `main`, before any other thread
        // exists, so nothing can read the environment concurrently.
        #[expect(unsafe_code, reason = "set_var is unsafe in edition 2024")]
        unsafe {
            std::env::set_var("WGPU_BACKEND", "vulkan");
        }
    }
}

fn main() -> iced_sessionlock::Result {
    let mut demo = false;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--demo" => demo = true,
            "--help" | "-h" => {
                print!("{HELP}");
                return Ok(());
            }
            other => {
                eprintln!("triton-lock: unknown argument '{other}'\n\n{HELP}");
                std::process::exit(2);
            }
        }
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
    if demo {
        tracing::warn!("--demo: PAM is skipped; any non-empty password unlocks");
    }
    let base = triton_ui::settings();
    iced_sessionlock::application(
        move || app::Lock::new(demo),
        app::Lock::update,
        app::Lock::view,
    )
    .subscription(app::Lock::subscription)
    .theme(app::Lock::theme)
    .style(app::Lock::style)
    .settings(Settings {
        id: Some("triton-lock".to_owned()),
        fonts: base.fonts,
        default_font: base.default_font,
        default_text_size: base.default_text_size,
        // wgpu antialiases paths (the avatar wave) only with MSAA.
        antialiasing: true,
        ..Settings::default()
    })
    .run()
    .inspect_err(|e| {
        // Protocol missing or the session already locked: log and exit
        // nonzero; niri/systemd sees the failure.
        tracing::error!("cannot start the session lock: {e}");
    })
    .inspect(|()| {
        // Clean exit (unlocked): the shell's lock-secure check reads this.
        if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR") {
            let _ =
                std::fs::remove_file(std::path::PathBuf::from(dir).join("triton").join("locked"));
        }
    })
}
