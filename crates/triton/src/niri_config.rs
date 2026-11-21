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

use crate::shell::{
    BAR_NAMESPACE, LAUNCHER_NAMESPACE, NOTIFICATIONS_NAMESPACE, OSD_NAMESPACE, POLKIT_NAMESPACE,
    POWER_NAMESPACE, QUICK_SETTINGS_NAMESPACE, TOASTS_NAMESPACE,
};

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

/// Every file triton owns, by name: `index.kdl` includes the rest. `scheme`
/// feeds `colors.kdl` — the pipeline's wallpaper scheme, or the reference.
pub fn render(scheme: &triton_design::Scheme) -> Vec<(&'static str, String)> {
    // Floating surfaces take their corner radius and shadow from a rule, since
    // iced cannot draw outside a surface's bounds. No rule `blur` here: rule
    // blur plus corner radius makes niri binarize the surface's alpha, so
    // surfaces request their own blur through ext-background-effect instead
    // (docs/research/niri-layer-blur-alpha.md). `xray` alone is safe.
    let floating = triton_design::shape::RADII.floating_window;
    let layer_rules = format!(
        "{HEADER}{}{}",
        rule(BAR_NAMESPACE, SIZE.bar_height / 2.0, ShadowToken::FLOATING),
        [
            LAUNCHER_NAMESPACE,
            NOTIFICATIONS_NAMESPACE,
            OSD_NAMESPACE,
            TOASTS_NAMESPACE,
            QUICK_SETTINGS_NAMESPACE,
            POWER_NAMESPACE,
            POLKIT_NAMESPACE,
        ]
        .iter()
        .map(|ns| rule(ns, floating, ShadowToken::FLOATING_LARGE))
        .collect::<String>(),
    );
    let index = format!(
        "{HEADER}include \"layer-rules.kdl\"\ninclude \"binds.kdl\"\ninclude \"colors.kdl\"\n"
    );
    vec![
        ("index.kdl", index),
        ("layer-rules.kdl", layer_rules),
        ("binds.kdl", binds()),
        ("colors.kdl", colors(scheme)),
    ]
}

/// The binds niri calls straight into the shell (roadmap §3: binds stay plain
/// `spawn` lines). Every one is a `triton msg`; the socket maps them to actions.
/// Media keys land on the OSD; Mod keys on the panels. If a bind duplicates one
/// in the user's own `config.kdl`, delete it here — this file is triton's.
fn binds() -> String {
    format!(
        "{HEADER}binds {{
    Mod+Space {{ spawn \"triton\" \"msg\" \"launcher toggle\"; }}
    Mod+N {{ spawn \"triton\" \"msg\" \"notifications toggle\"; }}
    Mod+Alt+Q {{ spawn \"triton\" \"msg\" \"quicksettings toggle\"; }}
    Mod+B {{ spawn \"triton\" \"msg\" \"bar toggle\"; }}
    Mod+Alt+L {{ spawn \"triton\" \"msg\" \"lock\"; }}
    XF86PowerOff {{ spawn \"triton\" \"msg\" \"power toggle\"; }}
    XF86AudioRaiseVolume {{ spawn \"triton\" \"msg\" \"volume up\"; }}
    XF86AudioLowerVolume {{ spawn \"triton\" \"msg\" \"volume down\"; }}
    XF86AudioMute {{ spawn \"triton\" \"msg\" \"volume mute\"; }}
    XF86AudioMicMute {{ spawn \"triton\" \"msg\" \"mic mute\"; }}
    XF86MonBrightnessUp {{ spawn \"triton\" \"msg\" \"brightness up\"; }}
    XF86MonBrightnessDown {{ spawn \"triton\" \"msg\" \"brightness down\"; }}
}}
"
    )
}

/// niri's own colors, from the active scheme: the focus ring takes primary,
/// the backdrop a darkened surface (color pipeline → niri, roadmap §4).
fn colors(scheme: &triton_design::Scheme) -> String {
    let hex8 = |c: iced_core::Color| hex(c);
    format!(
        "{HEADER}layout {{
    focus-ring {{
        active-color \"{active}\"
        inactive-color \"{inactive}\"
    }}
    backdrop-color \"{backdrop}\"
}}
",
        active = hex8(scheme.primary),
        inactive = hex8(scheme.outline),
        backdrop = hex8(scheme.surface_container_lowest),
    )
}

/// The systemd user unit: niri-session runs `graphical-session.target`, so a
/// `WantedBy` on it starts the shell with the session (roadmap §2).
const UNIT: &str = "# Written by triton. Changes here are overwritten.
[Unit]
Description=Triton shell (bar, panels, OSD, notifications)
PartOf=graphical-session.target
After=graphical-session.target

[Service]
ExecStart=triton
Restart=on-failure
Slice=session.slice

[Install]
WantedBy=graphical-session.target
";

/// `~/.config/systemd/user`: the unit plus the wants link that enables it —
/// the same "one owned file" pattern as the niri includes.
fn install_unit() -> io::Result<()> {
    let Some(base) = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join(".config")))
    else {
        return Ok(());
    };
    let dir = base.join("systemd/user");
    fs::create_dir_all(&dir)?;
    write_if_changed(&dir, "triton.service", UNIT)?;
    let wants = dir.join("graphical-session.target.wants");
    fs::create_dir_all(&wants)?;
    let link = wants.join("triton.service");
    if !link.exists() {
        std::os::unix::fs::symlink("../triton.service", &link)?;
        tracing::info!(path = %link.display(), "enabled triton.service");
    }
    Ok(())
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
/// itself. Also installs the systemd unit; its failure is logged, not fatal.
pub fn install(scheme: &triton_design::Scheme) -> io::Result<()> {
    if let Err(e) = install_unit() {
        tracing::warn!(%e, "cannot install the systemd unit");
    }
    let Some(niri) = niri_dir() else {
        return Ok(());
    };
    let dir = niri.join("triton");
    fs::create_dir_all(&dir)?;
    // Leaf files first, so the index never names a file that is not there yet.
    for (name, text) in render(scheme).into_iter().rev() {
        write_if_changed(&dir, name, &text)?;
    }
    ensure_include_line(&niri)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_includes_every_other_file() {
        let files = render(&triton_design::Scheme::DARK);
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
        let rules = &render(&triton_design::Scheme::DARK)[1].1;
        for namespace in [
            "triton-bar",
            "triton-launcher",
            "triton-notifications",
            "triton-osd",
        ] {
            assert!(
                rules.contains(&format!("namespace=\"^{namespace}$\"")),
                "{namespace}"
            );
        }
        assert_eq!(rules.matches("geometry-corner-radius").count(), 8);
        assert_eq!(rules.matches("xray false").count(), 8);
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
