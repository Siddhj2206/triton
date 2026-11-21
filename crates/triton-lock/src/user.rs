//! Who the session belongs to: the passwd entry for the process's real uid.

use std::ffi::CStr;

/// The user the lock screen greets and authenticates.
#[derive(Debug)]
pub struct User {
    /// Login name — `pam_authenticate`'s user argument.
    pub name: String,
    /// Shown under the avatar: the gecos's first field, or `name`.
    pub display: String,
    /// Up to two letters inside the avatar wave.
    pub initials: String,
}

/// The current user, from `getpwuid_r`; the env name as a last resort.
pub fn current() -> User {
    if let Some(user) = lookup() {
        return user;
    }
    let name = std::env::var("USER").unwrap_or_else(|_| "user".to_owned());
    User {
        display: name.clone(),
        initials: initials(&name),
        name,
    }
}

/// The reentrant passwd lookup; the buffer grows on `ERANGE`.
#[expect(
    unsafe_code,
    reason = "getpwuid_r's out-params are the stable POSIX ABI"
)]
fn lookup() -> Option<User> {
    let size = libc::sysconf(libc::_SC_GETPW_R_SIZE_MAX);
    let mut buf = vec![0_u8; usize::try_from(size.max(1024)).unwrap_or(16384)];
    unsafe {
        loop {
            let mut pwd: libc::passwd = std::mem::zeroed();
            let mut result: *mut libc::passwd = std::ptr::null_mut();
            let status = libc::getpwuid_r(
                libc::getuid(),
                &mut pwd,
                buf.as_mut_ptr().cast(),
                buf.len(),
                &mut result,
            );
            if status == libc::ERANGE && buf.len() < (1 << 20) {
                buf.resize(buf.len() * 4, 0);
                continue;
            }
            if status != 0 || result.is_null() {
                return None;
            }
            let name = CStr::from_ptr(pwd.pw_name).to_string_lossy().into_owned();
            let gecos = if pwd.pw_gecos.is_null() {
                String::new()
            } else {
                CStr::from_ptr(pwd.pw_gecos).to_string_lossy().into_owned()
            };
            let display = display_name(&gecos, &name);
            return Some(User {
                initials: initials(&display),
                display,
                name,
            });
        }
    }
}

/// The display name: gecos's first comma field when non-empty, else `name`.
fn display_name(gecos: &str, name: &str) -> String {
    let first = gecos.split(',').next().unwrap_or_default().trim();
    if first.is_empty() {
        name.to_owned()
    } else {
        first.to_owned()
    }
}

/// The avatar's initials: first letters of the display name's words, max 2,
/// uppercase.
fn initials(display: &str) -> String {
    display
        .split_whitespace()
        .filter_map(|word| word.chars().next())
        .take(2)
        .flat_map(char::to_uppercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_name_prefers_gecos() {
        assert_eq!(display_name("Sid Kumar,Other,x", "sid"), "Sid Kumar");
        assert_eq!(display_name("", "sid"), "sid");
        assert_eq!(display_name(",office", "sid"), "sid");
        assert_eq!(display_name("  ", "sid"), "sid");
    }

    #[test]
    fn initials_take_word_letters() {
        assert_eq!(initials("Sid Kumar"), "SK");
        assert_eq!(initials("Sid"), "S");
        assert_eq!(initials("madonna"), "M");
        assert_eq!(initials("a b c"), "AB");
        assert_eq!(initials(""), "");
        assert_eq!(initials("José Silva"), "JS");
    }
}
