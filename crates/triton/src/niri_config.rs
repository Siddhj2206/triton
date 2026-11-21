//! The first slice of ADR-0004: the include files triton owns under
//! `$XDG_CONFIG_HOME/niri/triton/`. For now only the layer rules the shell's own
//! surfaces need (blur behind the bar). The full transaction (validate a staged
//! copy, swap, confirm with `ConfigLoaded`, roll back) and adding the include line
//! to the user's `config.kdl` come with the settings app.

use std::path::{Path, PathBuf};
use std::{fs, io};

use triton_design::SIZE;
use triton_design::material::ShadowToken;

use crate::shell::BAR_NAMESPACE;

/// `#rrggbbaa`, as niri's config takes colors.
fn hex(color: iced_core::Color) -> String {
    color
        .into_rgba8()
        .iter()
        .fold(String::from("#"), |out, byte| format!("{out}{byte:02x}"))
}

const HEADER: &str = "// Written by triton. Changes here are overwritten; use the settings app.\n";

/// Every file triton owns, by name: `index.kdl` includes the rest.
pub fn render() -> Vec<(&'static str, String)> {
    // The islands are pills; the rule's corner radius shapes their Glass
    // shadow (the floating shadow), since iced cannot draw outside the surface.
    // No background-effect here: rule blur plus corner radius makes niri
    // binarize the surface's alpha, so the islands request their own blur
    // through ext-background-effect instead (docs/research/niri-layer-blur-alpha.md).
    let radius = SIZE.bar_height / 2.0;
    let shadow = ShadowToken::FLOATING;
    let (x, y, softness, spread) = (shadow.x, shadow.y, shadow.blur, shadow.spread);
    let color = hex(shadow.color);
    let layer_rules = format!(
        "{HEADER}\
layer-rule {{
    match namespace=\"^{BAR_NAMESPACE}$\"
    geometry-corner-radius {radius}
    shadow {{
        on
        softness {softness}
        spread {spread}
        offset x={x} y={y}
        color \"{color}\"
    }}
}}
"
    );
    let index = format!("{HEADER}include \"layer-rules.kdl\"\n");
    vec![("index.kdl", index), ("layer-rules.kdl", layer_rules)]
}

fn dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join(".config")))?;
    Some(base.join("niri").join("triton"))
}

/// Writes the files that differ from what is on disk. niri watches included
/// files, so it reloads by itself.
pub fn install() -> io::Result<()> {
    let Some(dir) = dir() else {
        return Ok(());
    };
    fs::create_dir_all(&dir)?;
    // Leaf files first, so the index never names a file that is not there yet.
    for (name, text) in render().into_iter().rev() {
        let path = dir.join(name);
        if fs::read_to_string(&path).is_ok_and(|old| old == text) {
            continue;
        }
        let staged = dir.join(format!(".{name}.new"));
        fs::write(&staged, &text)?;
        fs::rename(&staged, &path)?;
        tracing::info!(path = %path.display(), "wrote niri include");
    }
    Ok(())
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
    fn the_bar_is_rounded_and_shadowed_but_never_rule_blurred() {
        // Rule blur + corner radius binarizes the surface's alpha in niri
        // (docs/research/niri-layer-blur-alpha.md); blur is client-requested.
        let rules = &render()[1].1;
        assert!(rules.contains("namespace=\"^triton-bar$\""));
        assert!(rules.contains("geometry-corner-radius"));
        assert!(rules.contains("shadow {"));
        assert!(!rules.contains("background-effect"));
    }
}
