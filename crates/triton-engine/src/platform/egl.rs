//! Dynamic EGL 1.5 / GLES3 setup over `khronos-egl` + `glow`.
//!
//! One `EGLDisplay` and one GLES3 context serve the entire process; each
//! Wayland layer surface binds its own `EglSurface`.

#![allow(unsafe_code)] // FFI to libEGL.so.1 and glow loader

use std::ffi::c_void;
use std::fmt;

use khronos_egl as egl;

use crate::error::EngineError;
use crate::platform::wl_egl::EglWindow;

/// `EGL_PLATFORM_WAYLAND_KHR` (0x31D8).
const PLATFORM_WAYLAND: egl::Enum = 0x31D8;

/// `EGL_PLATFORM_SURFACELESS_MESA` (0x31DD).
const PLATFORM_SURFACELESS_MESA: egl::Enum = 0x31DD;

/// `EGL_BUFFER_AGE_KHR` / `EGL_BUFFER_AGE_EXT` (0x313D).
const BUFFER_AGE_KHR: egl::Int = 0x313D;

type SwapWithDamageFn =
    unsafe extern "system" fn(*mut c_void, *mut c_void, *const egl::Int, egl::Int) -> egl::Boolean;

/// Process-wide EGL display and GLES3 context.
pub struct Egl {
    instance: egl::DynamicInstance<egl::EGL1_5>,
    display: egl::Display,
    context: egl::Context,
    config: egl::Config,
    dummy_pbuffer: Option<egl::Surface>,
    swap_with_damage: Option<SwapWithDamageFn>,
    has_buffer_age: bool,
}

impl fmt::Debug for Egl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Egl")
            .field("display", &self.display)
            .field("swap_with_damage", &self.swap_with_damage.is_some())
            .field("has_buffer_age", &self.has_buffer_age)
            .finish_non_exhaustive()
    }
}

/// One `EGLSurface` backed by a `wl_egl_window`.
#[derive(Clone, Copy)]
pub struct EglSurface {
    pub(crate) raw: egl::Surface,
}

impl fmt::Debug for EglSurface {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EglSurface")
            .field("raw", &self.raw)
            .finish()
    }
}

impl Egl {
    /// Open an EGL 1.5 display from a live Wayland [`wayland_client::Connection`].
    pub fn from_connection(conn: &wayland_client::Connection) -> Result<Self, EngineError> {
        let display_ptr: *mut c_void = conn.backend().display_ptr().cast();
        // SAFETY: `display_ptr` comes directly from `conn.backend().display_ptr()` on the `system` backend.
        unsafe { Self::new(display_ptr) }
    }

    /// Open an EGL 1.5 display on `wl_display` and create an RGBA8 GLES3 context.
    ///
    /// # Safety
    /// `wl_display` must be a live `*mut wl_display` pointer from `wayland-client`'s `system` backend.
    pub unsafe fn new(wl_display: *mut c_void) -> Result<Self, EngineError> {
        // SAFETY: Loads system `libEGL.so.1` via khronos-egl dynamic loader.
        let instance = unsafe {
            egl::DynamicInstance::<egl::EGL1_5>::load_required()
                .map_err(|e| EngineError::Egl(format!("failed to load libEGL: {e}")))?
        };
        // SAFETY: Caller guarantees `wl_display` is a live `*mut wl_display`.
        let display = unsafe {
            instance
                .get_platform_display(PLATFORM_WAYLAND, wl_display, &[egl::ATTRIB_NONE])
                .map_err(|e| EngineError::Egl(format!("eglGetPlatformDisplay: {e}")))?
        };
        let (maj, min) = instance
            .initialize(display)
            .map_err(|e| EngineError::Egl(format!("eglInitialize: {e}")))?;
        let version = instance
            .query_string(Some(display), egl::VERSION)
            .map_or_else(|_| "?".into(), |s| s.to_string_lossy().into_owned());
        tracing::info!(maj, min, %version, "initialized EGL display");

        instance
            .bind_api(egl::OPENGL_ES_API)
            .map_err(|e| EngineError::Egl(format!("eglBindAPI(GLES): {e}")))?;
        let config = choose_config(&instance, display, true)?;
        let ctx_attribs = [egl::CONTEXT_CLIENT_VERSION, 3, egl::NONE];
        let context = instance
            .create_context(display, config, None, &ctx_attribs)
            .map_err(|e| EngineError::Egl(format!("eglCreateContext(ES3): {e}")))?;

        let extensions = instance
            .query_string(Some(display), egl::EXTENSIONS)
            .map_or_else(|_| String::new(), |s| s.to_string_lossy().into_owned());
        let has_buffer_age = extensions
            .split_whitespace()
            .any(|ext| ext == "EGL_EXT_buffer_age" || ext == "EGL_KHR_partial_update");

        let swap_with_damage = instance
            .get_proc_address("eglSwapBuffersWithDamageKHR")
            .map(|f| {
                // SAFETY: `eglSwapBuffersWithDamageKHR` has the `SwapWithDamageFn` C signature.
                unsafe { std::mem::transmute::<extern "system" fn(), SwapWithDamageFn>(f) }
            });
        tracing::debug!(
            swap_with_damage = swap_with_damage.is_some(),
            has_buffer_age,
            "queried EGL extensions"
        );

        Ok(Self {
            instance,
            display,
            context,
            config,
            dummy_pbuffer: None,
            swap_with_damage,
            has_buffer_age,
        })
    }

    /// Open a displayless EGL 1.5 context for offscreen FBO rendering (no Wayland
    /// or X11 display server required).
    ///
    /// Tries `EGL_PLATFORM_SURFACELESS_MESA` first and falls back to
    /// `EGL_DEFAULT_DISPLAY` (creating a 1×1 pbuffer if the driver lacks
    /// `EGL_KHR_surfaceless_context`).
    pub fn headless() -> Result<Self, EngineError> {
        // SAFETY: Loads system `libEGL.so.1` via khronos-egl dynamic loader.
        let instance = unsafe {
            egl::DynamicInstance::<egl::EGL1_5>::load_required()
                .map_err(|e| EngineError::Egl(format!("failed to load libEGL: {e}")))?
        };

        let display = {
            // SAFETY: `EGL_DEFAULT_DISPLAY` is the standard sentinel pointer for surfaceless Mesa.
            let surfaceless = unsafe {
                instance.get_platform_display(
                    PLATFORM_SURFACELESS_MESA,
                    egl::DEFAULT_DISPLAY,
                    &[egl::ATTRIB_NONE],
                )
            };
            match surfaceless {
                Ok(d) if instance.initialize(d).is_ok() => d,
                _ => {
                    // SAFETY: `EGL_DEFAULT_DISPLAY` is a valid sentinel for `eglGetDisplay`.
                    let fallback = unsafe {
                        instance.get_display(egl::DEFAULT_DISPLAY).ok_or_else(|| {
                            EngineError::Egl("no headless EGL display available".into())
                        })?
                    };
                    instance
                        .initialize(fallback)
                        .map_err(|e| EngineError::Egl(format!("eglInitialize(headless): {e}")))?;
                    fallback
                }
            }
        };

        instance
            .bind_api(egl::OPENGL_ES_API)
            .map_err(|e| EngineError::Egl(format!("eglBindAPI(GLES): {e}")))?;
        let config = choose_config(&instance, display, false)?;
        let ctx_attribs = [egl::CONTEXT_CLIENT_VERSION, 3, egl::NONE];
        let context = instance
            .create_context(display, config, None, &ctx_attribs)
            .map_err(|e| EngineError::Egl(format!("eglCreateContext(headless ES3): {e}")))?;

        let dummy_pbuffer = if instance
            .make_current(display, None, None, Some(context))
            .is_ok()
        {
            None
        } else {
            let pb_attribs = [egl::WIDTH, 1, egl::HEIGHT, 1, egl::NONE];
            let pb = instance
                .create_pbuffer_surface(display, config, &pb_attribs)
                .map_err(|e| {
                    let _ = instance.destroy_context(display, context);
                    let _ = instance.terminate(display);
                    EngineError::Egl(format!("eglCreatePbufferSurface(1x1): {e}"))
                })?;
            instance
                .make_current(display, Some(pb), Some(pb), Some(context))
                .map_err(|e| {
                    let _ = instance.destroy_surface(display, pb);
                    let _ = instance.destroy_context(display, context);
                    let _ = instance.terminate(display);
                    EngineError::Egl(format!("eglMakeCurrent(pbuffer): {e}"))
                })?;
            Some(pb)
        };

        Ok(Self {
            instance,
            display,
            context,
            config,
            dummy_pbuffer,
            swap_with_damage: None,
            has_buffer_age: false,
        })
    }

    /// Create a `glow::Context` resolving GLES3 entry points via `eglGetProcAddress`.
    pub fn glow(&self) -> glow::Context {
        // SAFETY: Caller ensures an EGL surface/context is current before calling GL functions.
        unsafe {
            glow::Context::from_loader_function(|name| {
                self.instance
                    .get_proc_address(name)
                    .map_or(std::ptr::null(), |f| f as usize as *const c_void)
            })
        }
    }

    /// Wrap a `wl_egl_window` in an `EGLSurface`.
    pub fn create_surface(&self, window: &EglWindow) -> Result<EglSurface, EngineError> {
        // SAFETY: `window.native_window()` is a valid `wl_egl_window` for `self.display`.
        let raw = unsafe {
            self.instance
                .create_platform_window_surface(
                    self.display,
                    self.config,
                    window.native_window(),
                    &[egl::ATTRIB_NONE],
                )
                .map_err(|e| EngineError::Egl(format!("eglCreatePlatformWindowSurface: {e}")))?
        };
        Ok(EglSurface { raw })
    }

    /// Destroy an `EGLSurface` before its `wl_egl_window` is dropped.
    pub fn destroy_surface(&self, surface: EglSurface) {
        let _ = self.instance.destroy_surface(self.display, surface.raw);
    }

    /// Bind `surface` (or surfaceless / fallback pbuffer) as current on the calling thread.
    pub fn make_current(&self, surface: Option<EglSurface>) -> Result<(), EngineError> {
        let s = surface.map(|s| s.raw).or(self.dummy_pbuffer);
        self.instance
            .make_current(self.display, s, s, Some(self.context))
            .map_err(|e| EngineError::Egl(format!("eglMakeCurrent: {e}")))
    }

    /// Disable swap-interval blocking (`eglSwapInterval(0)`) so Wayland
    /// `wl_surface.frame` callbacks pace presentation without stalling inside Mesa.
    pub fn set_swap_interval(&self, interval: i32) {
        let _ = self.instance.swap_interval(self.display, interval);
    }

    /// Query `EGL_BUFFER_AGE_KHR` for `surface` when the buffer-age extension is supported.
    ///
    /// Returns `None` if the extension is unavailable, the query fails, or EGL reports a negative age.
    pub fn buffer_age(&self, surface: EglSurface) -> Option<u32> {
        if !self.has_buffer_age {
            return None;
        }
        let age = self
            .instance
            .query_surface(self.display, surface.raw, BUFFER_AGE_KHR)
            .ok()?;
        u32::try_from(age).ok()
    }

    /// Present `surface`, reporting buffer-pixel damage rects when supported.
    pub fn swap(&self, surface: EglSurface, damage: &[[i32; 4]]) -> Result<(), EngineError> {
        if let Some(swap_fn) = self.swap_with_damage
            && !damage.is_empty()
        {
            let mut flat = Vec::with_capacity(damage.len() * 4);
            for rect in damage {
                flat.extend_from_slice(rect);
            }
            // SAFETY: `self.display` and `surface.raw` are live EGL handles and
            // `flat` holds `damage.len() * 4` contiguous ints.
            let ok = unsafe {
                swap_fn(
                    self.display.as_ptr(),
                    surface.raw.as_ptr(),
                    flat.as_ptr(),
                    i32::try_from(damage.len()).unwrap_or(0),
                )
            };
            if ok == egl::FALSE {
                let err = self
                    .instance
                    .get_error()
                    .unwrap_or(egl::Error::BadParameter);
                return Err(EngineError::Egl(format!(
                    "eglSwapBuffersWithDamageKHR: {err}"
                )));
            }
            return Ok(());
        }
        self.instance
            .swap_buffers(self.display, surface.raw)
            .map_err(|e| EngineError::Egl(format!("eglSwapBuffers: {e}")))
    }
}

fn choose_config(
    instance: &egl::DynamicInstance<egl::EGL1_5>,
    display: egl::Display,
    require_window: bool,
) -> Result<egl::Config, EngineError> {
    let attribs_window = [
        egl::SURFACE_TYPE,
        egl::WINDOW_BIT,
        egl::RENDERABLE_TYPE,
        egl::OPENGL_ES3_BIT,
        egl::RED_SIZE,
        8,
        egl::GREEN_SIZE,
        8,
        egl::BLUE_SIZE,
        8,
        egl::ALPHA_SIZE,
        8,
        egl::NONE,
    ];
    let attribs_pbuffer = [
        egl::SURFACE_TYPE,
        egl::PBUFFER_BIT,
        egl::RENDERABLE_TYPE,
        egl::OPENGL_ES3_BIT,
        egl::RED_SIZE,
        8,
        egl::GREEN_SIZE,
        8,
        egl::BLUE_SIZE,
        8,
        egl::ALPHA_SIZE,
        8,
        egl::NONE,
    ];
    let attribs_any = [
        egl::SURFACE_TYPE,
        egl::DONT_CARE,
        egl::RENDERABLE_TYPE,
        egl::OPENGL_ES3_BIT,
        egl::RED_SIZE,
        8,
        egl::GREEN_SIZE,
        8,
        egl::BLUE_SIZE,
        8,
        egl::ALPHA_SIZE,
        8,
        egl::NONE,
    ];
    let mut configs = Vec::with_capacity(64);
    if require_window {
        instance
            .choose_config(display, &attribs_window, &mut configs)
            .map_err(|e| EngineError::Egl(format!("eglChooseConfig: {e}")))?;
    } else {
        let _ = instance.choose_config(display, &attribs_pbuffer, &mut configs);
        if configs.is_empty() {
            instance
                .choose_config(display, &attribs_any, &mut configs)
                .map_err(|e| EngineError::Egl(format!("eglChooseConfig(headless): {e}")))?;
        }
    }
    // Prefer an 8-8-8-8 RGBA config (avoid 10-10-10-2 or non-alpha configs).
    let chosen = configs
        .iter()
        .copied()
        .find(|&c| {
            instance.get_config_attrib(display, c, egl::RED_SIZE) == Ok(8)
                && instance.get_config_attrib(display, c, egl::ALPHA_SIZE) == Ok(8)
        })
        .or_else(|| configs.first().copied())
        .ok_or_else(|| EngineError::Egl("no matching RGBA8 GLES3 EGLConfig".into()))?;
    Ok(chosen)
}

impl Drop for Egl {
    fn drop(&mut self) {
        let _ = self.instance.make_current(self.display, None, None, None);
        if let Some(pb) = self.dummy_pbuffer.take() {
            let _ = self.instance.destroy_surface(self.display, pb);
        }
        let _ = self.instance.destroy_context(self.display, self.context);
        let _ = self.instance.terminate(self.display);
    }
}
