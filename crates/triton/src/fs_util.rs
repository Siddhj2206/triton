//! Shared filesystem helpers for owned config writers (`niri_config` and
//! `theme_pipeline`).

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Resolve the XDG config root (`$XDG_CONFIG_HOME`, falling back to
/// `~/.config`). Returns `None` when neither variable is set.
///
/// Tests and isolated sessions should pass an explicit config directory
/// directly rather than calling this.
pub(crate) fn xdg_config_home() -> Option<PathBuf> {
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(xdg));
    }
    std::env::var_os("HOME")
        .filter(|v| !v.is_empty())
        .map(|home| PathBuf::from(home).join(".config"))
}

/// Write `content` to `dest` atomically: write to a unique sibling temp file
/// (`.{file_name}.{pid}.{nonce}.tmp`), `fsync`, and `rename` over `dest` so
/// watchers never observe a half-written file.
///
/// On failure, removes the temp file if created and returns `(op, path, error)`.
pub(crate) fn atomic_write(
    dest: &Path,
    content: &str,
    fallback_name: &str,
) -> Result<(), (&'static str, PathBuf, std::io::Error)> {
    static NEXT_NONCE: AtomicU64 = AtomicU64::new(0);
    let nonce = NEXT_NONCE.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let file_name = dest.file_name().map_or_else(
        || fallback_name.to_owned(),
        |n| n.to_string_lossy().into_owned(),
    );
    let tmp = dest.with_file_name(format!(".{file_name}.{pid}.{nonce}.tmp"));

    let mut file =
        fs::File::create(&tmp).map_err(|source| ("create temp file", tmp.clone(), source))?;
    if let Err(source) = file
        .write_all(content.as_bytes())
        .and_then(|()| file.sync_all())
    {
        let _ = fs::remove_file(&tmp);
        return Err(("write", tmp, source));
    }
    drop(file);

    if let Err(source) = fs::rename(&tmp, dest) {
        let _ = fs::remove_file(&tmp);
        return Err(("rename", dest.to_path_buf(), source));
    }
    Ok(())
}
