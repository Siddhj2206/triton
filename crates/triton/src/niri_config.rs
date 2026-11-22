//! Triton's owned niri config writer (ADR-0004).
//!
//! Triton configures niri exclusively through generated files it owns under
//! `<niri config dir>/triton/`: `index.kdl` (the entry point the user's
//! `config.kdl` includes), `binds.kdl`, `colors.kdl`, and `layer-rules.kdl`.
//! The user's hand-written `config.kdl` is touched once at most — `install`
//! appends a single `include optional=true "triton/index.kdl"` line — and every
//! later mutation only regenerates owned files, which niri watches and
//! live-reloads.
//!
//! Generated files are deterministic renders written atomically (temp file in
//! the same directory, `fsync`, `rename`), so no validate/swap/rollback
//! transaction is needed; `niri validate -c` is the dev-time oracle.
//!
//! Every function takes an explicit target directory so tests and the isolated
//! nested session run against scratch roots and never touch the host's
//! `~/.config/niri`.

use std::fmt::Write as _;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use thiserror::Error;
use triton_design::color::{Color, Scheme, Variant, with_alpha};
use triton_design::material::Material;
use triton_design::shape::RADII;

use crate::state::{BarMode, SurfaceRole};

/// Directory inside the niri config dir that Triton owns outright.
pub const OWNED_DIR: &str = "triton";

/// The only line Triton ever appends to the user's `config.kdl`.
///
/// `optional=true` requires niri 26.04+; the include survives a missing
/// `triton/` directory so a broken generation can never wedge the session.
pub const INCLUDE_LINE: &str = "include optional=true \"triton/index.kdl\"";

const GENERATED_HEADER: &str =
    "// Written by triton. Changes here are overwritten; use the settings app.\n";

/// Errors produced while writing owned files or installing the include line.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum NiriConfigError {
    /// A filesystem operation failed; `path` is the file or directory involved.
    #[error("{op} failed for {}: {source}", path.display())]
    Io {
        /// What was attempted (`"create directory"`, `"read"`, `"write"`, `"rename"`).
        op: &'static str,
        /// The file or directory the operation targeted.
        path: PathBuf,
        /// The underlying IO error.
        source: std::io::Error,
    },
}

impl NiriConfigError {
    fn io(op: &'static str, path: impl Into<PathBuf>) -> impl FnOnce(std::io::Error) -> Self {
        let path = path.into();
        move |source| Self::Io { op, path, source }
    }
}

/// Resolve the niri config directory: `$XDG_CONFIG_HOME/niri`, falling back to
/// `~/.config/niri`. `None` when neither variable is set.
///
/// Callers that need an explicit dir (tests, the nested dev session) should
/// pass their own path to the writers instead of calling this.
pub fn niri_config_dir() -> Option<PathBuf> {
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(xdg).join("niri"));
    }
    std::env::var_os("HOME")
        .filter(|v| !v.is_empty())
        .map(|home| PathBuf::from(home).join(".config").join("niri"))
}

/// Render `triton/index.kdl`: the entry point included by `config.kdl`.
///
/// Nested includes resolve relative to `index.kdl`'s own directory and are
/// `optional` so one bad file can never block the rest.
pub fn render_index() -> String {
    format!(
        "{GENERATED_HEADER}\
include optional=true \"layer-rules.kdl\"\n\
include optional=true \"binds.kdl\"\n\
include optional=true \"colors.kdl\"\n"
    )
}

/// Render `triton/binds.kdl`: the resolved board shortcuts.
///
/// `Super+B` toggles the bar and `Super+V` opens clipboard history through
/// `triton msg`; `Super+Shift+V` toggles niri's floating window mode natively.
pub fn render_binds() -> String {
    let mut out = String::from(GENERATED_HEADER);
    out.push_str("binds {\n");
    for bind in BINDS {
        out.push_str("    ");
        out.push_str(bind.key);
        if let Some(title) = bind.overlay_title {
            let _ = write!(out, " hotkey-overlay-title=\"{title}\"");
        }
        match bind.action {
            Action::Spawn(argv) => {
                out.push_str(" { spawn");
                for arg in argv {
                    let _ = write!(out, " \"{arg}\"");
                }
                out.push_str("; }\n");
            }
            Action::Niri(action) => {
                let _ = writeln!(out, " {{ {action}; }}");
            }
        }
    }
    out.push_str("}\n");
    out
}

/// Render `triton/colors.kdl` from a [`Scheme`]: niri's accent, shadow, and
/// backdrop colors harmonized with the live Material 3 theme.
///
/// Focus ring, border, and tab indicators take `primary`/`outline`/`error`,
/// window shadows take the floating-material shadow color, the insert hint is
/// a translucent `primary`, and the overview backdrop takes `surface`.
pub fn render_colors(scheme: &Scheme) -> String {
    let active = hex8(scheme.primary);
    let inactive = hex8(scheme.outline);
    let urgent = hex8(scheme.error);
    let hint = hex8(with_alpha(scheme.primary, 0.5));
    let shadow = Material::Glass
        .shadow()
        .map_or_else(|| hex8(Color::BLACK), |token| hex8(token.color));
    let backdrop = hex8(scheme.surface);
    let highlight = hex8(scheme.primary_container);

    format!(
        "{GENERATED_HEADER}\
layout {{
    focus-ring {{
        active-color \"{active}\"
        inactive-color \"{inactive}\"
        urgent-color \"{urgent}\"
    }}
    border {{
        active-color \"{active}\"
        inactive-color \"{inactive}\"
        urgent-color \"{urgent}\"
    }}
    tab-indicator {{
        active-color \"{active}\"
        inactive-color \"{inactive}\"
        urgent-color \"{urgent}\"
    }}
    shadow {{
        color \"{shadow}\"
    }}
    insert-hint {{
        color \"{hint}\"
    }}
}}
overview {{
    backdrop-color \"{backdrop}\"
}}
recent-windows {{
    highlight {{
        active-color \"{highlight}\"
        urgent-color \"{urgent}\"
    }}
}}
"
    )
}

/// Render `triton/layer-rules.kdl` from [`triton_design::MaterialSpec`] values.
///
/// Each Triton layer-shell namespace gets `xray` from its material's spec
/// (Mica `true`, Glass/Thick `false`), and floating materials also get a
/// `geometry-corner-radius` + `shadow` matching their [`ShadowToken`]. Rule-side
/// `blur` is never emitted: blur regions are submitted per-surface via
/// `ext-background-effect-v1`, and rule blur would smear across transparent
/// shadow gutters and quantize interior alpha (docs/research/niri-layer-blur-alpha.md).
///
/// `bar_mode` selects the bar's material: `Pinned` is a Mica strip while
/// `Autohide` is floating Glass islands; `Hidden` leaves the (unmounted) Glass
/// rule in place.
pub fn render_layer_rules(variant: Variant, bar_mode: BarMode) -> String {
    let bar_material = match bar_mode {
        BarMode::Pinned => Material::Mica,
        BarMode::Autohide | BarMode::Hidden => Material::Glass,
    };
    let surfaces = [
        (SurfaceRole::Bar.namespace(), bar_material),
        (SurfaceRole::Osd.namespace(), Material::Glass),
    ];

    let mut out = String::from(GENERATED_HEADER);
    for (namespace, material) in surfaces {
        render_layer_rule(&mut out, namespace, material, variant);
    }
    out
}

/// Atomically write all four owned files under `<config_dir>/triton/`.
///
/// Returns the owned directory. Creates `config_dir` (and parents) if missing.
pub fn write_owned_files(
    config_dir: &Path,
    scheme: &Scheme,
    variant: Variant,
    bar_mode: BarMode,
) -> Result<PathBuf, NiriConfigError> {
    let owned = ensure_owned_dir(config_dir)?;
    atomic_write(&owned.join("index.kdl"), &render_index())?;
    atomic_write(&owned.join("binds.kdl"), &render_binds())?;
    atomic_write(&owned.join("colors.kdl"), &render_colors(scheme))?;
    atomic_write(
        &owned.join("layer-rules.kdl"),
        &render_layer_rules(variant, bar_mode),
    )?;
    Ok(owned)
}

/// Regenerate only `triton/colors.kdl` — the live-theming seam used when a new
/// [`Scheme`] is extracted from the wallpaper.
pub fn write_colors(config_dir: &Path, scheme: &Scheme) -> Result<PathBuf, NiriConfigError> {
    let owned = ensure_owned_dir(config_dir)?;
    let path = owned.join("colors.kdl");
    atomic_write(&path, &render_colors(scheme))?;
    Ok(path)
}

/// Regenerate only `triton/layer-rules.kdl` — e.g. when the bar mode changes
/// between Mica (`Pinned`) and Glass (`Autohide`) materials.
pub fn write_layer_rules(
    config_dir: &Path,
    variant: Variant,
    bar_mode: BarMode,
) -> Result<PathBuf, NiriConfigError> {
    let owned = ensure_owned_dir(config_dir)?;
    let path = owned.join("layer-rules.kdl");
    atomic_write(&path, &render_layer_rules(variant, bar_mode))?;
    Ok(path)
}

/// One-time startup install (ADR-0004): write all owned files and append
/// [`INCLUDE_LINE`] to `config.kdl` at most once.
///
/// Idempotent: if any line already references `triton/index.kdl` — including a
/// commented-out one the user disabled on purpose — nothing is appended and
/// every other byte of the user's file is preserved verbatim.
pub fn install(
    config_dir: &Path,
    scheme: &Scheme,
    variant: Variant,
    bar_mode: BarMode,
) -> Result<(), NiriConfigError> {
    write_owned_files(config_dir, scheme, variant, bar_mode)?;
    ensure_include_line(&config_dir.join("config.kdl"))
}

/// Resolved board shortcuts rendered into `triton/binds.kdl`.
///
/// `Super+V` targets the Phase 2 clipboard history; the bind ships now so the
/// muscle memory is stable — `triton msg clipboard` resolves once that command
/// lands and fails harmlessly until then.
const BINDS: &[Bind] = &[
    Bind {
        key: "Super+B",
        overlay_title: Some("Triton: Toggle Bar"),
        action: Action::Spawn(&["triton", "msg", "bar", "toggle"]),
    },
    Bind {
        key: "Super+V",
        overlay_title: Some("Triton: Clipboard History"),
        action: Action::Spawn(&["triton", "msg", "clipboard"]),
    },
    Bind {
        key: "Super+Shift+V",
        overlay_title: None,
        action: Action::Niri("toggle-window-floating"),
    },
];

/// One generated keybind line inside `binds { ... }`.
#[derive(Debug)]
struct Bind {
    /// niri key syntax, e.g. `Super+Shift+V`.
    key: &'static str,
    /// Shown in niri's hotkey overlay (`Super+/`).
    overlay_title: Option<&'static str>,
    action: Action,
}

#[derive(Debug)]
enum Action {
    /// `spawn` with an argv, each element quoted.
    Spawn(&'static [&'static str]),
    /// A native niri action such as `toggle-window-floating`.
    Niri(&'static str),
}

/// Render one `layer-rule` block for a namespace styled with `material`.
fn render_layer_rule(out: &mut String, namespace: &str, material: Material, variant: Variant) {
    out.push_str("layer-rule {\n");
    let _ = writeln!(out, "    match namespace=\"^{namespace}$\"");
    if material.floats() {
        let _ = writeln!(out, "    geometry-corner-radius {}", RADII.floating_window);
    }
    if let Some(spec) = material.spec(variant)
        && let Some(xray) = spec.xray
    {
        out.push_str("    background-effect {\n");
        let _ = writeln!(out, "        xray {xray}");
        out.push_str("    }\n");
    }
    if let Some(shadow) = material.shadow() {
        out.push_str("    shadow {\n");
        out.push_str("        on\n");
        let _ = writeln!(out, "        softness {}", shadow.blur);
        let _ = writeln!(out, "        spread {}", shadow.spread);
        let _ = writeln!(out, "        offset x={} y={}", shadow.x, shadow.y);
        let _ = writeln!(out, "        color \"{}\"", hex8(shadow.color));
        out.push_str("    }\n");
    }
    out.push_str("}\n");
}

/// Create `<config_dir>/triton/` (and parents) if missing.
fn ensure_owned_dir(config_dir: &Path) -> Result<PathBuf, NiriConfigError> {
    let owned = config_dir.join(OWNED_DIR);
    fs::create_dir_all(&owned).map_err(NiriConfigError::io("create directory", owned.clone()))?;
    Ok(owned)
}

/// Append [`INCLUDE_LINE`] to `config.kdl` unless a line already references
/// `triton/index.kdl`. Rewrites atomically so a crash can never leave a
/// half-appended line; all other bytes are preserved verbatim.
fn ensure_include_line(config_path: &Path) -> Result<(), NiriConfigError> {
    let existing = match fs::read_to_string(config_path) {
        Ok(contents) => contents,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(source) => {
            return Err(NiriConfigError::Io {
                op: "read",
                path: config_path.to_path_buf(),
                source,
            });
        }
    };

    // Any mention of our index on an include-ish line counts — plain
    // `include "triton/index.kdl"`, our `optional=true` spelling, or a line
    // the user commented out to disable Triton on purpose.
    let already_installed = existing
        .lines()
        .any(|line| line.contains("include") && line.contains("triton/index.kdl"));
    if already_installed {
        return Ok(());
    }

    let mut updated = existing;
    if !updated.is_empty() && !updated.ends_with('\n') {
        updated.push('\n');
    }
    updated.push_str(INCLUDE_LINE);
    updated.push('\n');
    atomic_write(config_path, &updated)
}

/// Write `contents` to `path` atomically: temp file in the same directory,
/// `fsync`, rename. niri watches the directory so the rename is the reload
/// trigger — a half-written file is never observed.
fn atomic_write(path: &Path, contents: &str) -> Result<(), NiriConfigError> {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("triton.kdl");
    let tmp_path = path.with_file_name(format!(".{file_name}.tmp"));

    let write_result = (|| {
        let mut file = fs::File::create(&tmp_path)?;
        file.write_all(contents.as_bytes())?;
        file.sync_all()
    })();
    if let Err(source) = write_result {
        let _ = fs::remove_file(&tmp_path);
        return Err(NiriConfigError::Io {
            op: "write",
            path: tmp_path,
            source,
        });
    }

    fs::rename(&tmp_path, path).map_err(|source| NiriConfigError::Io {
        op: "rename",
        path: path.to_path_buf(),
        source,
    })
}

/// Format a [`Color`] as an 8-digit RGBA hex literal (`#rrggbbaa`), matching
/// the form niri accepts in `*-color` properties.
fn hex8(color: Color) -> String {
    fn channel(v: f32) -> u8 {
        (v.clamp(0.0, 1.0) * 255.0).round() as u8
    }
    format!(
        "#{:02x}{:02x}{:02x}{:02x}",
        channel(color.r),
        channel(color.g),
        channel(color.b),
        channel(color.a)
    )
}
