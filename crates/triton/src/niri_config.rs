//! The first slice of ADR-0004: the include files triton owns under
//! `$XDG_CONFIG_HOME/niri/triton/`. For now only the layer rules the shell's own
//! surfaces need (rounding and shadows on floating surfaces). On startup the
//! files are (re)written and the user's `config.kdl` gets its one triton line if
//! it is not already there; the settings app's full transaction (validate a
//! staged copy, swap, confirm with `ConfigLoaded`, roll back) comes later.

use std::path::{Path, PathBuf};
use std::{fs, io};

use triton_design::SIZE;
use triton_design::material::ShadowToken;

use crate::shell::{BAR_NAMESPACE, LAUNCHER_NAMESPACE, NOTIFICATIONS_NAMESPACE};

/// `#rrggbbaa`, as niri's config takes colors.
fn hex(color: iced_core::Color) -> String {
    color
        .into_rgba8()
        .iter()
        .fold(String::from("#"), |out, byte| format!("{out}{byte:02x}"))
}

const HEADER: &str = "// Written by triton. Changes here are overwritten; use the settings app.\n";

/// The only line triton ever puts in the user's `config.kdl` (ADR-0004).
/// `optional` keeps niri starting if the triton directory is ever removed.
const INCLUDE_LINE: &str = "include optional=true \"triton/index.kdl\"";

/// One layer-rule per namespace: corner radius for the shadow shape, shadow
/// from the tokens, and `xray false` — niri blurs client-requested regions as
/// xray (wallpaper only) by default, but Glass and Thick see the windows
/// below (`MaterialSpec::xray` in triton-design). The Mica strip has no rule:
/// wallpaper-only is exactly what it wants.
fn rule(namespace: &str, radius: f32, shadow: ShadowToken) -> String {
    let (x, y, softness, spread) = (shadow.x, shadow.y, shadow.blur, shadow.spread);
    let color = hex(shadow.color);
    format!(
        "layer-rule {{
    match namespace=\"^{namespace}$\"
    geometry-corner-radius {radius}
    background-effect {{
        xray false
    }}
    shadow {{
        on
        softness {softness}
        spread {spread}
        offset x={x} y={y}
        color \"{color}\"
    }}
}}
"
    )
}

/// Every file triton owns, by name: `index.kdl` includes the rest.
pub fn render() -> Vec<(&'static str, String)> {
    // Floating surfaces take their corner radius and shadow from a rule, since
    // iced cannot draw outside a surface's bounds. No rule `blur` here: rule
    // blur plus corner radius makes niri binarize the surface's alpha, so
    // surfaces request their own blur through ext-background-effect instead
    // (docs/research/niri-layer-blur-alpha.md). `xray` alone is safe.
    let layer_rules = format!(
        "{HEADER}{}{}",
        rule(BAR_NAMESPACE, SIZE.bar_height / 2.0, ShadowToken::FLOATING),
        rule(
            LAUNCHER_NAMESPACE,
            triton_design::shape::RADII.floating_window,
            ShadowToken::FLOATING_LARGE,
        ) + &rule(
            NOTIFICATIONS_NAMESPACE,
            triton_design::shape::RADII.floating_window,
            ShadowToken::FLOATING_LARGE,
        ),
    );
    let index = format!("{HEADER}include \"layer-rules.kdl\"\n");
    vec![("index.kdl", index), ("layer-rules.kdl", layer_rules)]
}

fn niri_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join(".config")))?;
    Some(base.join("niri"))
}

/// Writes `text` to `dir/name` only if it differs, through a same-dir rename.
fn write_if_changed(dir: &Path, name: &str, text: &str) -> io::Result<()> {
    let path = dir.join(name);
    if fs::read_to_string(&path).is_ok_and(|old| old == text) {
        return Ok(());
    }
    let staged = dir.join(format!(".{name}.new"));
    fs::write(&staged, text)?;
    fs::rename(&staged, &path)?;
    tracing::info!(path = %path.display(), "wrote niri config");
    Ok(())
}

/// `config.kdl` already hands off to triton when some `include` line names our
/// index (any spelling). Comments never count.
fn has_include(config: &str) -> bool {
    config.lines().any(|line| {
        let line = line.trim();
        line.starts_with("include") && line.contains("triton/index.kdl")
    })
}

/// Appends the include line to `dir`'s `config.kdl` once. The file may not
/// exist yet or may not end in a newline.
fn ensure_include_line(dir: &Path) -> io::Result<()> {
    let old = fs::read_to_string(dir.join("config.kdl")).unwrap_or_default();
    if has_include(&old) {
        return Ok(());
    }
    let sep = if old.ends_with('\n') || old.is_empty() {
        ""
    } else {
        "\n"
    };
    write_if_changed(dir, "config.kdl", &format!("{old}{sep}{INCLUDE_LINE}\n"))
}

/// Writes the files that differ from what is on disk, then makes sure the
/// user's config includes them. niri watches included files and reloads by
/// itself.
pub fn install() -> io::Result<()> {
    let Some(niri) = niri_dir() else {
        return Ok(());
    };
    let dir = niri.join("triton");
    fs::create_dir_all(&dir)?;
    // Leaf files first, so the index never names a file that is not there yet.
    for (name, text) in render().into_iter().rev() {
        write_if_changed(&dir, name, &text)?;
    }
    ensure_include_line(&niri)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_includes_every_other_file() {
        let files = render();
        let (_, index) = files
            .iter()
            .find(|(n, _)| *n == "index.kdl")
            .expect("index");
        for (name, _) in files.iter().filter(|(n, _)| *n != "index.kdl") {
            assert!(index.contains(&format!("include \"{name}\"")), "{name}");
        }
    }

    #[test]
    fn floating_surfaces_are_rounded_shadowed_and_see_through() {
        // Rule blur + corner radius binarizes the surface's alpha in niri
        // (docs/research/niri-layer-blur-alpha.md); blur is client-requested.
        // `xray false` keeps that blur seeing windows, not just wallpaper.
        let rules = &render()[1].1;
        for namespace in ["triton-bar", "triton-launcher", "triton-notifications"] {
            assert!(
                rules.contains(&format!("namespace=\"^{namespace}$\"")),
                "{namespace}"
            );
        }
        assert_eq!(rules.matches("geometry-corner-radius").count(), 3);
        assert_eq!(rules.matches("xray false").count(), 3);
        assert!(!rules.contains("blur"));
    }

    #[test]
    fn include_line_is_added_once() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config.kdl");

        ensure_include_line(dir.path()).unwrap();
        let text = fs::read_to_string(&config).unwrap();
        assert_eq!(text, format!("{INCLUDE_LINE}\n"));

        // Second run is a no-op: one line, file unchanged.
        ensure_include_line(dir.path()).unwrap();
        assert_eq!(fs::read_to_string(&config).unwrap(), text);
    }

    #[test]
    fn include_line_respects_existing_spelling_and_file_end() {
        for existing in [
            // Already included, either spelling — nothing appended.
            "include \"triton/index.kdl\"\n",
            &format!("{INCLUDE_LINE}\n"),
            "// include \"triton/index.kdl\"\ninclude optional=true \"triton/index.kdl\"\n",
        ] {
            assert!(has_include(existing), "{existing}");
        }
        // A commented-out line does not count.
        assert!(!has_include("// include \"triton/index.kdl\"\n"));

        // No trailing newline: the line still lands on its own line.
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("config.kdl"), "prefer-no-csd").unwrap();
        ensure_include_line(dir.path()).unwrap();
        let text = fs::read_to_string(dir.path().join("config.kdl")).unwrap();
        assert_eq!(text, format!("prefer-no-csd\n{INCLUDE_LINE}\n"));
    }
}
