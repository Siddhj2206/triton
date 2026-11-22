//! Dynamic `libwayland-egl.so.1` loader (zero `-devel` packages required).
//!
//! `wl_egl_window` is shared between the Wayland client and Mesa's EGL driver,
//! so its layout is owned by `libwayland-egl` and never reimplemented here.

#![allow(unsafe_code)] // FFI to libwayland-egl.so.1 via libloading

use std::ffi::c_void;
use std::fmt;
use std::os::raw::c_int;

use wayland_client::Proxy;
use wayland_client::protocol::wl_surface::WlSurface;

use crate::error::EngineError;

type CreateFn = unsafe extern "C" fn(*mut c_void, c_int, c_int) -> *mut c_void;
type ResizeFn = unsafe extern "C" fn(*mut c_void, c_int, c_int, c_int, c_int);
type DestroyFn = unsafe extern "C" fn(*mut c_void);

/// Dynamically loaded `libwayland-egl` entry points.
pub struct WlEgl {
    _lib: libloading::Library,
    create: CreateFn,
    resize: ResizeFn,
    destroy: DestroyFn,
}

impl fmt::Debug for WlEgl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad("WlEgl { .. }")
    }
}

/// A `wl_egl_window` bound to one `wl_surface`, sized in physical buffer px.
pub struct EglWindow {
    raw: *mut c_void,
    resize: ResizeFn,
    destroy: DestroyFn,
}

impl fmt::Debug for EglWindow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EglWindow")
            .field("raw", &self.raw)
            .finish_non_exhaustive()
    }
}

impl WlEgl {
    /// Load `libwayland-egl.so.1` (falling back to the unversioned name).
    pub fn load() -> Result<Self, EngineError> {
        // SAFETY: Loading a standard system shared library with C ABI symbols.
        let lib = unsafe {
            libloading::Library::new("libwayland-egl.so.1")
                .or_else(|_| libloading::Library::new("libwayland-egl.so"))
                .map_err(|e| EngineError::WlEgl(format!("failed to dlopen libwayland-egl: {e}")))?
        };
        // SAFETY: Symbol names and C signatures match the official libwayland-egl ABI.
        unsafe {
            let create = *lib
                .get::<CreateFn>(b"wl_egl_window_create\0")
                .map_err(|e| EngineError::WlEgl(format!("wl_egl_window_create: {e}")))?;
            let resize = *lib
                .get::<ResizeFn>(b"wl_egl_window_resize\0")
                .map_err(|e| EngineError::WlEgl(format!("wl_egl_window_resize: {e}")))?;
            let destroy = *lib
                .get::<DestroyFn>(b"wl_egl_window_destroy\0")
                .map_err(|e| EngineError::WlEgl(format!("wl_egl_window_destroy: {e}")))?;
            Ok(Self {
                _lib: lib,
                create,
                resize,
                destroy,
            })
        }
    }

    /// Create a `wl_egl_window` of `w`x`h` physical buffer pixels on `surface`.
    pub fn create_window(
        &self,
        surface: &WlSurface,
        w: u32,
        h: u32,
    ) -> Result<EglWindow, EngineError> {
        let proxy = surface.id().as_ptr();
        if proxy.is_null() {
            return Err(EngineError::WlEgl(
                "wl_surface proxy pointer is null".into(),
            ));
        }
        let w = i32::try_from(w.max(1)).unwrap_or(1) as c_int;
        let h = i32::try_from(h.max(1)).unwrap_or(1) as c_int;
        // SAFETY: `proxy` is a live `*mut wl_surface` from wayland-client's `system` backend.
        let raw = unsafe { (self.create)(proxy.cast(), w, h) };
        if raw.is_null() {
            return Err(EngineError::WlEgl(
                "wl_egl_window_create returned null".into(),
            ));
        }
        Ok(EglWindow {
            raw,
            resize: self.resize,
            destroy: self.destroy,
        })
    }
}

impl EglWindow {
    /// Raw `EGLNativeWindowType` pointer for `eglCreatePlatformWindowSurface`.
    pub fn native_window(&self) -> *mut c_void {
        self.raw
    }

    /// Resize the EGL backing buffer to `w`x`h` physical pixels.
    pub fn resize(&self, w: u32, h: u32) {
        let w = i32::try_from(w.max(1)).unwrap_or(1) as c_int;
        let h = i32::try_from(h.max(1)).unwrap_or(1) as c_int;
        // SAFETY: `self.raw` is a live `wl_egl_window` owned by `self`.
        unsafe { (self.resize)(self.raw, w, h, 0, 0) }
    }
}

impl Drop for EglWindow {
    fn drop(&mut self) {
        // SAFETY: `self.raw` was created by `wl_egl_window_create` and is destroyed once here.
        unsafe { (self.destroy)(self.raw) }
    }
}

// SAFETY: `EglWindow` is owned exclusively and driven from the engine's event-loop thread.
unsafe impl Send for EglWindow {}
