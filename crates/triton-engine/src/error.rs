//! Error types for `triton-engine`.

use thiserror::Error;

/// Errors returned by the Wayland/EGL/GLES3 engine.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum EngineError {
    /// Failed to connect to the Wayland display or initialize globals.
    #[error("wayland initialization failed: {0}")]
    Wayland(String),

    /// Failed to load or initialize `libwayland-egl`.
    #[error("wayland-egl error: {0}")]
    WlEgl(String),

    /// Failed to initialize EGL or create an EGL surface/context.
    #[error("EGL error: {0}")]
    Egl(String),

    /// Failed to compile shaders or initialize GLES3 resources.
    #[error("GLES3 renderer error: {0}")]
    Gpu(String),

    /// Event loop initialization or runtime failure.
    #[error("event loop error: {0}")]
    EventLoop(String),

    /// Headless script parsing or key resolution failure.
    #[error("headless script error: {0}")]
    Script(String),

    /// Filesystem I/O failure (e.g. writing PNG/tree snapshots).
    #[error("I/O error: {0}")]
    Io(String),
}
