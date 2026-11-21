//! PAM authentication through the system's libpam, loaded at runtime with
//! `libloading` — never linked, so the binary runs against whatever stack the
//! distro ships (and never against a nix build, which has no linkable `.pc`).
//!
//! [`authenticate`] is blocking: `pam_authenticate` can take seconds, and PAM
//! modules add failure delays on purpose. Callers run it on a worker thread,
//! never the UI thread.

use std::path::Path;

/// The PAM service to authenticate against: Triton's own stack file when a
/// distro or user installed one, else a stock file every system carries.
pub fn service() -> &'static str {
    pick_service(|p| Path::new(p).is_file())
}

/// The first service that has a config file on disk, else "login" (PAM itself
/// falls back to `/etc/pam.d/other` for a service with no file).
fn pick_service(exists: impl Fn(&str) -> bool) -> &'static str {
    for (files, name) in [
        (
            ["/etc/pam.d/triton-lock", "/usr/lib/pam.d/triton-lock"].as_slice(),
            "triton-lock",
        ),
        (["/etc/pam.d/login", "/usr/lib/pam.d/login"].as_slice(), "login"),
        (["/etc/pam.d/su"].as_slice(), "su"),
    ] {
        if files.iter().any(|file| exists(file)) {
            return name;
        }
    }
    "login"
}

/// Checks `password` against the PAM stack for `user`. Only a clean run
/// through `pam_authenticate` + `pam_acct_mgmt` unlocks; every failure (bad
/// password, missing service, expired account) returns `false` with the
/// reason in the log.
pub fn authenticate(service: &str, user: &str, password: &str) -> bool {
    match ffi::authenticate(service, user, password) {
        Ok(()) => true,
        Err(e) => {
            tracing::info!("pam: authentication did not succeed: {e}");
            false
        }
    }
}

#[expect(
    unsafe_code,
    reason = "libpam's C ABI: the pam_message/pam_response/pam_conv layouts \
    and the pam_* signatures are stable (Linux-PAM, LSB)"
)]
mod ffi {
    use std::ffi::{CStr, CString, c_char, c_int, c_void};
    use std::ptr;
    use std::sync::OnceLock;

    use anyhow::{Context, bail};
    use libloading::{Library, Symbol};

    /// `pam_handle_t`, opaque.
    pub enum PamHandle {}

    #[repr(C)]
    struct PamMessage {
        msg_style: c_int,
        msg: *const c_char,
    }

    #[repr(C)]
    struct PamResponse {
        resp: *mut c_char,
        resp_retcode: c_int,
    }

    type Converse = extern "C" fn(
        num_msg: c_int,
        msg: *const *const PamMessage,
        resp: *mut *mut PamResponse,
        appdata_ptr: *mut c_void,
    ) -> c_int;

    #[repr(C)]
    struct PamConv {
        conv: Converse,
        appdata_ptr: *mut c_void,
    }

    type PamStart = unsafe extern "C" fn(
        service: *const c_char,
        user: *const c_char,
        conv: *const PamConv,
        handle: *mut *mut PamHandle,
    ) -> c_int;
    type PamAuthenticate = unsafe extern "C" fn(handle: *mut PamHandle, flags: c_int) -> c_int;
    type PamAcctMgmt = unsafe extern "C" fn(handle: *mut PamHandle, flags: c_int) -> c_int;
    type PamEnd = unsafe extern "C" fn(handle: *mut PamHandle, status: c_int) -> c_int;

    const PAM_SUCCESS: c_int = 0;
    const PAM_PROMPT_ECHO_OFF: c_int = 1;
    const PAM_PROMPT_ECHO_ON: c_int = 2;
    const PAM_ERROR_MSG: c_int = 3;
    const PAM_TEXT_INFO: c_int = 4;
    const PAM_BUF_ERR: c_int = 5;
    /// The account is verified but its password expired. A lock screen cannot
    /// change passwords, so it still unlocks (swaylock does the same).
    const PAM_NEW_AUTHTOK_REQD: c_int = 12;
    const PAM_CONV_ERR: c_int = 19;

    /// The system libpam, dlopened once for the process's life.
    fn libpam() -> anyhow::Result<&'static Library> {
        static LIBPAM: OnceLock<anyhow::Result<Library>> = OnceLock::new();
        match LIBPAM.get_or_init(|| unsafe {
            Library::new("libpam.so.0")
                .or_else(|_| Library::new("libpam.so"))
                .map_err(anyhow::Error::new)
        }) {
            Ok(lib) => Ok(lib),
            Err(e) => bail!("cannot load the system libpam: {e}"),
        }
    }

    /// The `pam_conv` callback: answers password prompts from `appdata_ptr`
    /// (a `CString` the caller keeps alive), and forwards error/info lines to
    /// the log. PAM frees what we allocate, so responses come from libc's
    /// allocator (`calloc`/`strdup`), matching `free(3)`.
    extern "C" fn converse(
        num_msg: c_int,
        msg: *const *const PamMessage,
        resp: *mut *mut PamResponse,
        appdata_ptr: *mut c_void,
    ) -> c_int {
        unsafe {
            if num_msg <= 0 || msg.is_null() || resp.is_null() {
                return PAM_CONV_ERR;
            }
            let responses = libc::calloc(num_msg as usize, std::mem::size_of::<PamResponse>())
                .cast::<PamResponse>();
            if responses.is_null() {
                return PAM_BUF_ERR;
            }
            let password = appdata_ptr.cast::<c_char>();
            let mut failed = false;
            for i in 0..num_msg {
                let message = *msg.add(i as usize);
                if message.is_null() {
                    continue;
                }
                let text = || {
                    if (*message).msg.is_null() {
                        String::new()
                    } else {
                        CStr::from_ptr((*message).msg)
                            .to_string_lossy()
                            .into_owned()
                    }
                };
                match (*message).msg_style {
                    PAM_PROMPT_ECHO_OFF | PAM_PROMPT_ECHO_ON => {
                        if password.is_null() {
                            failed = true;
                            break;
                        }
                        let dup = libc::strdup(password);
                        if dup.is_null() {
                            failed = true;
                            break;
                        }
                        (*responses.add(i as usize)).resp = dup;
                    }
                    PAM_ERROR_MSG => tracing::warn!("pam error message: {}", text()),
                    PAM_TEXT_INFO => tracing::info!("pam: {}", text()),
                    other => {
                        tracing::debug!(style = other, "pam: unhandled conversation message");
                    }
                }
            }
            if failed {
                for i in 0..num_msg {
                    let response = responses.add(i as usize);
                    if !(*response).resp.is_null() {
                        libc::free((*response).resp.cast());
                    }
                }
                libc::free(responses.cast());
                return PAM_CONV_ERR;
            }
            *resp = responses;
            PAM_SUCCESS
        }
    }

    /// One run of the PAM stack. Always `pam_end`s the handle it starts.
    pub fn authenticate(service: &str, user: &str, password: &str) -> anyhow::Result<()> {
        let lib = libpam()?;
        let pam_start: Symbol<PamStart> = unsafe { lib.get(b"pam_start") }.context("pam_start")?;
        let pam_authenticate: Symbol<PamAuthenticate> =
            unsafe { lib.get(b"pam_authenticate") }.context("pam_authenticate")?;
        let pam_end: Symbol<PamEnd> = unsafe { lib.get(b"pam_end") }.context("pam_end")?;
        // Not every build needs account management; its absence is not fatal.
        let pam_acct_mgmt: Option<Symbol<PamAcctMgmt>> =
            unsafe { lib.get(b"pam_acct_mgmt") }.ok();

        let service = CString::new(service).context("service name contains NUL")?;
        let user = CString::new(user).context("user name contains NUL")?;
        let password = CString::new(password).context("password contains NUL")?;
        let conv = PamConv {
            conv: converse,
            appdata_ptr: password.as_ptr() as *mut c_void,
        };
        let mut handle: *mut PamHandle = ptr::null_mut();
        let mut status = unsafe { pam_start(service.as_ptr(), user.as_ptr(), &conv, &mut handle) };
        if status != PAM_SUCCESS {
            bail!("pam_start failed (status {status})");
        }
        if handle.is_null() {
            bail!("pam_start returned a null handle");
        }
        status = unsafe { pam_authenticate(handle, 0) };
        if status == PAM_SUCCESS
            && let Some(acct) = &pam_acct_mgmt
        {
            let acct_status = unsafe { acct(handle, 0) };
            status = if acct_status == PAM_NEW_AUTHTOK_REQD {
                tracing::info!("pam: password expired; unlocking anyway");
                PAM_SUCCESS
            } else {
                acct_status
            };
        }
        let end_status = unsafe { pam_end(handle, status) };
        if status != PAM_SUCCESS {
            bail!("authentication failed (pam status {status})");
        }
        if end_status != PAM_SUCCESS {
            bail!("pam_end failed (status {end_status})");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn only(paths: &[&str]) -> impl Fn(&str) -> bool + '_ {
        move |p| paths.contains(&p)
    }

    #[test]
    fn service_prefers_triton_lock() {
        assert_eq!(pick_service(|_| false), "login");
        assert_eq!(pick_service(only(&["/usr/lib/pam.d/triton-lock"])), "triton-lock");
        assert_eq!(pick_service(only(&["/etc/pam.d/triton-lock", "/etc/pam.d/login"])), "triton-lock");
        assert_eq!(pick_service(only(&["/usr/lib/pam.d/login"])), "login");
        assert_eq!(pick_service(only(&["/etc/pam.d/su"])), "su");
    }
}
