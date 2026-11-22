//! Small filesystem helpers shared by the config writers (`niri_config`,
//! `pipeline`): the XDG config root and a staged same-dir write that reports
//! whether anything changed.

use std::path::{Path, PathBuf};
use std::{fs, io};

/// `$XDG_CONFIG_HOME`, falling back to `~/.config`.
pub fn config_root() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join(".config")))
}

/// Writes `text` to `dir/name` only if it differs, through a same-dir rename.
/// `Ok(true)` means the file was written this call.
pub fn write_if_changed(dir: &Path, name: &str, text: &str) -> io::Result<bool> {
    let path = dir.join(name);
    if fs::read_to_string(&path).is_ok_and(|old| old == text) {
        return Ok(false);
    }
    fs::create_dir_all(dir)?;
    let staged = dir.join(format!(".{name}.new"));
    fs::write(&staged, text)?;
    fs::rename(&staged, &path)?;
    tracing::info!(path = %path.display(), "wrote {name}");
    Ok(true)
}
