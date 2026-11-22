//! In-process Material 3 live theming pipeline powered by `material-colors`.
//!
//! Extracts a 28-role [`triton_design::Scheme`] from the active wallpaper for
//! both [`Variant::Dark`] and [`Variant::Light`] (replacing the hand-picked
//! reference light scheme at runtime), writes GTK3/4 `gtk.css` and niri
//! `triton/colors.kdl` atomically under an explicit config root, and gates
//! `gsettings` strictly behind [`BackendMode::RealServices`].

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use material_colors::color::Rgb;
use material_colors::hct::Hct;
use material_colors::image::extract_color;
use material_colors::scheme::variant::SchemeTonalSpot;
use thiserror::Error;
use tracing::warn;
use triton_design::color::{Color, Scheme, Variant, with_alpha};
use triton_engine::ImageData;

use crate::niri_config::{self, NiriConfigError};
use crate::state::BackendMode;

const GENERATED_CSS_HEADER: &str =
    "/* Written by triton. Changes here are overwritten on theme/wallpaper updates. */\n";

/// Errors produced by the live theming pipeline when writing config files.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ThemePipelineError {
    /// A filesystem operation failed; `path` is the file or directory involved.
    #[error("{op} failed for {}: {source}", path.display())]
    Io {
        /// What was attempted (`"create directory"`, `"write"`, `"rename"`).
        op: &'static str,
        /// The file or directory targeted.
        path: PathBuf,
        /// The underlying IO error.
        source: std::io::Error,
    },
    /// Updating niri's owned `triton/colors.kdl` failed.
    #[error("update niri colors.kdl: {0}")]
    NiriConfig(#[from] NiriConfigError),
}

impl ThemePipelineError {
    fn io(op: &'static str, path: impl Into<PathBuf>) -> impl FnOnce(std::io::Error) -> Self {
        let path = path.into();
        move |source| Self::Io { op, path, source }
    }
}

/// File paths written by [`sync_config_files`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeSyncPaths {
    pub gtk3: PathBuf,
    pub gtk4: PathBuf,
    pub niri_colors: PathBuf,
}

/// Target configuration for syncing the live [`Scheme`] and [`Variant`] to
/// external desktop files (GTK3/4 `gtk.css`, niri `triton/colors.kdl`) and
/// optionally `gsettings`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeSyncConfig {
    /// Root XDG config directory (e.g. `~/.config` or an ephemeral `TempDir`).
    pub config_root: PathBuf,
    /// Whether to invoke `gsettings` when syncing under
    /// [`BackendMode::RealServices`].
    pub apply_gsettings: bool,
}

impl ThemeSyncConfig {
    /// Production `--real-services` target: writes GTK3/4 `gtk.css`,
    /// `niri/triton/colors.kdl`, and updates GNOME `color-scheme` via
    /// `gsettings`.
    pub fn real_services(config_root: PathBuf) -> Self {
        Self {
            config_root,
            apply_gsettings: true,
        }
    }

    /// Filesystem-only target (e.g. integration tests): writes GTK3/4
    /// `gtk.css` and `niri/triton/colors.kdl` into `config_root` without ever
    /// invoking `gsettings`.
    pub fn files_only(config_root: PathBuf) -> Self {
        Self {
            config_root,
            apply_gsettings: false,
        }
    }
}

/// Arguments passed to `gsettings` to switch GNOME/GTK dark/light preference.
pub const fn gsettings_color_scheme_args(variant: Variant) -> [&'static str; 4] {
    let value = match variant {
        Variant::Dark => "prefer-dark",
        Variant::Light => "prefer-light",
    };
    ["set", "org.gnome.desktop.interface", "color-scheme", value]
}

/// Apply the GNOME/GTK desktop `color-scheme` via `gsettings`.
///
/// Strictly gated behind [`BackendMode::RealServices`]: returns `Ok(false)`
/// without spawning any subprocess when `backend == BackendMode::Fake`.
/// When active, spawns `gsettings` on a named background thread so the Wayland
/// event loop is never blocked and no zombie child is left behind.
pub fn apply_gsettings(variant: Variant, backend: BackendMode) -> Result<bool, ThemePipelineError> {
    if backend != BackendMode::RealServices {
        return Ok(false);
    }
    let args = gsettings_color_scheme_args(variant);
    std::thread::Builder::new()
        .name("triton-gsettings".to_owned())
        .spawn(move || {
            match Command::new("gsettings")
                .args(args)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            {
                Ok(mut child) => {
                    if let Err(err) = child.wait() {
                        warn!(error = %err, "waiting on gsettings child failed");
                    }
                }
                Err(err) => {
                    warn!(error = %err, "failed to spawn gsettings for color-scheme");
                }
            }
        })
        .map_err(ThemePipelineError::io("spawn thread", "triton-gsettings"))?;
    Ok(true)
}

/// Resolve the XDG config root (`$XDG_CONFIG_HOME`, falling back to
/// `~/.config`). Returns `None` when neither variable is set.
///
/// Tests and isolated sessions should pass an explicit `config_root` directly
/// rather than calling this.
pub fn xdg_config_home() -> Option<PathBuf> {
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(xdg));
    }
    std::env::var_os("HOME")
        .filter(|v| !v.is_empty())
        .map(|home| PathBuf::from(home).join(".config"))
}

/// Maximum sample dimension along each axis before quantizing wallpaper pixels.
/// 128x128 = 16,384 pixels keeps Celebi quantization under ~1-2 ms even on 4K
/// wallpapers while preserving dominant chromatic clusters.
const MAX_QUANTIZE_DIM: u32 = 128;

/// Fallback seed color (Material 3 default blue) when an image has no opaque
/// pixels to quantize.
const FALLBACK_SEED: Rgb = Rgb {
    red: 66,
    green: 133,
    blue: 244,
};

/// Extract the dominant Material 3 source (seed) color from an RGBA8 wallpaper
/// image in-process via `material-colors` (Celebi quantization + HCT scoring).
pub fn extract_source_color(image: &ImageData) -> Color {
    let pixels = sample_opaque_rgb(image);
    if pixels.is_empty() {
        return rgb_to_color(FALLBACK_SEED);
    }
    rgb_to_color(extract_color(&pixels))
}

/// Derive a full 28-role Material 3 [`Scheme`] (tonal-spot) from a seed color
/// for the requested [`Variant`].
///
/// The 26 Material 3 accent, error, surface, and outline roles are generated
/// by `SchemeTonalSpot`; `signal_warn` and `signal_info` are preserved from
/// `triton-design`'s semantic signal tokens.
pub fn scheme_from_seed(seed: Color, variant: Variant) -> Scheme {
    let is_dark = matches!(variant, Variant::Dark);
    let hct = Hct::new(color_to_rgb(seed));
    let dynamic = SchemeTonalSpot::new(hct, is_dark, None).scheme;
    let reference = Scheme::reference(variant);

    Scheme {
        primary: rgb_to_color(dynamic.primary()),
        on_primary: rgb_to_color(dynamic.on_primary()),
        primary_container: rgb_to_color(dynamic.primary_container()),
        on_primary_container: rgb_to_color(dynamic.on_primary_container()),
        secondary: rgb_to_color(dynamic.secondary()),
        on_secondary: rgb_to_color(dynamic.on_secondary()),
        secondary_container: rgb_to_color(dynamic.secondary_container()),
        on_secondary_container: rgb_to_color(dynamic.on_secondary_container()),
        tertiary: rgb_to_color(dynamic.tertiary()),
        on_tertiary: rgb_to_color(dynamic.on_tertiary()),
        tertiary_container: rgb_to_color(dynamic.tertiary_container()),
        on_tertiary_container: rgb_to_color(dynamic.on_tertiary_container()),
        error: rgb_to_color(dynamic.error()),
        on_error: rgb_to_color(dynamic.on_error()),
        error_container: rgb_to_color(dynamic.error_container()),
        on_error_container: rgb_to_color(dynamic.on_error_container()),
        surface: rgb_to_color(dynamic.surface()),
        surface_container_lowest: rgb_to_color(dynamic.surface_container_lowest()),
        surface_container_low: rgb_to_color(dynamic.surface_container_low()),
        surface_container: rgb_to_color(dynamic.surface_container()),
        surface_container_high: rgb_to_color(dynamic.surface_container_high()),
        surface_container_highest: rgb_to_color(dynamic.surface_container_highest()),
        on_surface: rgb_to_color(dynamic.on_surface()),
        on_surface_variant: rgb_to_color(dynamic.on_surface_variant()),
        outline: rgb_to_color(dynamic.outline()),
        outline_variant: rgb_to_color(dynamic.outline_variant()),
        signal_warn: reference.signal_warn,
        signal_info: reference.signal_info,
    }
}

/// Extract a full 28-role Material 3 [`Scheme`] directly from an RGBA8
/// wallpaper image for the given [`Variant`].
pub fn extract_scheme(image: &ImageData, variant: Variant) -> Scheme {
    let seed = extract_source_color(image);
    scheme_from_seed(seed, variant)
}

/// Render pure `@define-color` GTK4 / libadwaita overrides from a [`Scheme`].
///
/// Emits only color variable definitions (never layout or spacing rules) so
/// stock GNOME / libadwaita apps harmonize with Triton's live palette without
/// re-laying them out.
pub fn render_gtk4_css(scheme: &Scheme, variant: Variant) -> String {
    let accent_bg = hex6(scheme.primary);
    let accent_fg = hex6(scheme.on_primary);
    let accent_text = match variant {
        Variant::Dark => hex6(scheme.primary),
        Variant::Light => hex6(scheme.on_primary_container),
    };
    let destructive_bg = hex6(scheme.error);
    let destructive_fg = hex6(scheme.on_error);
    let warning_bg = hex6(scheme.signal_warn);
    let warning_fg = hex6(Color::from_rgb8(0x19, 0x1B, 0x20));
    let success_bg = hex6(scheme.tertiary);
    let success_fg = hex6(scheme.on_tertiary);
    let window_bg = hex6(scheme.surface);
    let window_fg = hex6(scheme.on_surface);
    let view_bg = hex6(scheme.surface_container_lowest);
    let view_fg = hex6(scheme.on_surface);
    let headerbar_bg = hex6(scheme.surface_container_low);
    let headerbar_fg = hex6(scheme.on_surface);
    let headerbar_border = hex6(scheme.outline_variant);
    let headerbar_backdrop = hex6(scheme.surface);
    let card_bg = hex6(scheme.surface_container);
    let card_fg = hex6(scheme.on_surface);
    let dialog_bg = hex6(scheme.surface_container_high);
    let dialog_fg = hex6(scheme.on_surface);
    let popover_bg = hex6(scheme.surface_container_high);
    let popover_fg = hex6(scheme.on_surface);
    let sidebar_bg = hex6(scheme.surface_container_low);
    let sidebar_fg = hex6(scheme.on_surface);
    let sidebar_backdrop = hex6(scheme.surface);
    let secondary_sidebar_bg = hex6(scheme.surface_container);
    let secondary_sidebar_fg = hex6(scheme.on_surface);
    let shade = rgba_css(with_alpha(
        Color::BLACK,
        match variant {
            Variant::Dark => 0.36,
            Variant::Light => 0.12,
        },
    ));
    let scrollbar_outline = rgba_css(with_alpha(scheme.outline_variant, 0.5));

    format!(
        "{GENERATED_CSS_HEADER}\
@define-color accent_bg_color {accent_bg};\n\
@define-color accent_fg_color {accent_fg};\n\
@define-color accent_color {accent_text};\n\
@define-color destructive_bg_color {destructive_bg};\n\
@define-color destructive_fg_color {destructive_fg};\n\
@define-color destructive_color {destructive_bg};\n\
@define-color warning_bg_color {warning_bg};\n\
@define-color warning_fg_color {warning_fg};\n\
@define-color warning_color {warning_bg};\n\
@define-color success_bg_color {success_bg};\n\
@define-color success_fg_color {success_fg};\n\
@define-color success_color {success_bg};\n\
@define-color window_bg_color {window_bg};\n\
@define-color window_fg_color {window_fg};\n\
@define-color view_bg_color {view_bg};\n\
@define-color view_fg_color {view_fg};\n\
@define-color headerbar_bg_color {headerbar_bg};\n\
@define-color headerbar_fg_color {headerbar_fg};\n\
@define-color headerbar_border_color {headerbar_border};\n\
@define-color headerbar_backdrop_color {headerbar_backdrop};\n\
@define-color headerbar_shade_color {shade};\n\
@define-color card_bg_color {card_bg};\n\
@define-color card_fg_color {card_fg};\n\
@define-color dialog_bg_color {dialog_bg};\n\
@define-color dialog_fg_color {dialog_fg};\n\
@define-color popover_bg_color {popover_bg};\n\
@define-color popover_fg_color {popover_fg};\n\
@define-color sidebar_bg_color {sidebar_bg};\n\
@define-color sidebar_fg_color {sidebar_fg};\n\
@define-color sidebar_backdrop_color {sidebar_backdrop};\n\
@define-color secondary_sidebar_bg_color {secondary_sidebar_bg};\n\
@define-color secondary_sidebar_fg_color {secondary_sidebar_fg};\n\
@define-color shade_color {shade};\n\
@define-color scrollbar_outline_color {scrollbar_outline};\n"
    )
}

/// Render pure `@define-color` GTK3 overrides from a [`Scheme`].
///
/// Covers both standard Adwaita GTK3 tokens (`theme_*`, `borders`,
/// `insensitive_*`) and the shared libadwaita/adw-gtk3 named colors.
pub fn render_gtk3_css(scheme: &Scheme, variant: Variant) -> String {
    let accent_bg = hex6(scheme.primary);
    let accent_fg = hex6(scheme.on_primary);
    let accent_text = match variant {
        Variant::Dark => hex6(scheme.primary),
        Variant::Light => hex6(scheme.on_primary_container),
    };
    let bg = hex6(scheme.surface);
    let fg = hex6(scheme.on_surface);
    let base = hex6(scheme.surface_container_lowest);
    let insensitive_bg = hex6(scheme.surface_container_low);
    let insensitive_fg = hex6(scheme.on_surface_variant);
    let border = hex6(scheme.outline_variant);
    let headerbar_bg = hex6(scheme.surface_container_low);
    let card_bg = hex6(scheme.surface_container);
    let popover_bg = hex6(scheme.surface_container_high);
    let warning = hex6(scheme.signal_warn);
    let error = hex6(scheme.error);
    let success = hex6(scheme.tertiary);

    format!(
        "{GENERATED_CSS_HEADER}\
@define-color theme_fg_color {fg};\n\
@define-color theme_bg_color {bg};\n\
@define-color theme_base_color {base};\n\
@define-color theme_text_color {fg};\n\
@define-color theme_selected_bg_color {accent_bg};\n\
@define-color theme_selected_fg_color {accent_fg};\n\
@define-color insensitive_bg_color {insensitive_bg};\n\
@define-color insensitive_fg_color {insensitive_fg};\n\
@define-color insensitive_base_color {base};\n\
@define-color theme_unfocused_fg_color {insensitive_fg};\n\
@define-color theme_unfocused_bg_color {bg};\n\
@define-color theme_unfocused_base_color {base};\n\
@define-color theme_unfocused_text_color {insensitive_fg};\n\
@define-color theme_unfocused_selected_bg_color {accent_bg};\n\
@define-color theme_unfocused_selected_fg_color {accent_fg};\n\
@define-color borders {border};\n\
@define-color unfocused_borders {border};\n\
@define-color warning_color {warning};\n\
@define-color error_color {error};\n\
@define-color success_color {success};\n\
@define-color accent_bg_color {accent_bg};\n\
@define-color accent_fg_color {accent_fg};\n\
@define-color accent_color {accent_text};\n\
@define-color window_bg_color {bg};\n\
@define-color window_fg_color {fg};\n\
@define-color view_bg_color {base};\n\
@define-color view_fg_color {fg};\n\
@define-color headerbar_bg_color {headerbar_bg};\n\
@define-color headerbar_fg_color {fg};\n\
@define-color card_bg_color {card_bg};\n\
@define-color card_fg_color {fg};\n\
@define-color popover_bg_color {popover_bg};\n\
@define-color popover_fg_color {fg};\n"
    )
}

/// Atomically write `<config_root>/gtk-3.0/gtk.css` and
/// `<config_root>/gtk-4.0/gtk.css`.
pub fn write_gtk_css(
    config_root: &Path,
    scheme: &Scheme,
    variant: Variant,
) -> Result<(PathBuf, PathBuf), ThemePipelineError> {
    let gtk3_dir = config_root.join("gtk-3.0");
    let gtk4_dir = config_root.join("gtk-4.0");
    fs::create_dir_all(&gtk3_dir).map_err(ThemePipelineError::io("create directory", &gtk3_dir))?;
    fs::create_dir_all(&gtk4_dir).map_err(ThemePipelineError::io("create directory", &gtk4_dir))?;

    let gtk3_path = gtk3_dir.join("gtk.css");
    let gtk4_path = gtk4_dir.join("gtk.css");
    atomic_write(&gtk3_path, &render_gtk3_css(scheme, variant))?;
    atomic_write(&gtk4_path, &render_gtk4_css(scheme, variant))?;
    Ok((gtk3_path, gtk4_path))
}

/// Atomically regenerate GTK3/4 `gtk.css` and `<config_root>/niri/triton/colors.kdl`.
pub fn sync_config_files(
    config_root: &Path,
    scheme: &Scheme,
    variant: Variant,
) -> Result<ThemeSyncPaths, ThemePipelineError> {
    let (gtk3, gtk4) = write_gtk_css(config_root, scheme, variant)?;
    let niri_colors = niri_config::write_colors(&config_root.join("niri"), scheme)?;
    Ok(ThemeSyncPaths {
        gtk3,
        gtk4,
        niri_colors,
    })
}

/// Write `content` to a sibling temp file, `fsync` it, and rename it over
/// `dest` so watchers never observe a half-written CSS file.
fn atomic_write(dest: &Path, content: &str) -> Result<(), ThemePipelineError> {
    static NEXT_NONCE: AtomicU64 = AtomicU64::new(0);
    let nonce = NEXT_NONCE.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let file_name = dest.file_name().map_or_else(
        || "gtk.css".to_owned(),
        |n| n.to_string_lossy().into_owned(),
    );
    let tmp = dest.with_file_name(format!(".{file_name}.{pid}.{nonce}.tmp"));

    let mut file =
        fs::File::create(&tmp).map_err(ThemePipelineError::io("create temp file", &tmp))?;
    if let Err(err) = file
        .write_all(content.as_bytes())
        .and_then(|()| file.sync_all())
    {
        let _ = fs::remove_file(&tmp);
        return Err(ThemePipelineError::Io {
            op: "write",
            path: tmp,
            source: err,
        });
    }
    drop(file);

    if let Err(err) = fs::rename(&tmp, dest) {
        let _ = fs::remove_file(&tmp);
        return Err(ThemePipelineError::Io {
            op: "rename",
            path: dest.to_path_buf(),
            source: err,
        });
    }
    Ok(())
}

fn hex6(c: Color) -> String {
    format!(
        "#{:02x}{:02x}{:02x}",
        channel_u8(c.r),
        channel_u8(c.g),
        channel_u8(c.b),
    )
}

fn rgba_css(c: Color) -> String {
    format!(
        "rgba({}, {}, {}, {:.2})",
        channel_u8(c.r),
        channel_u8(c.g),
        channel_u8(c.b),
        c.a.clamp(0.0, 1.0),
    )
}

fn sample_opaque_rgb(image: &ImageData) -> Vec<Rgb> {
    let w = image.width;
    let h = image.height;
    if w == 0 || h == 0 || image.rgba.is_empty() {
        return Vec::new();
    }

    let step_x = (w / MAX_QUANTIZE_DIM).max(1) as usize;
    let step_y = (h / MAX_QUANTIZE_DIM).max(1) as usize;
    let width = w as usize;
    let height = h as usize;

    let est_x = width.div_ceil(step_x);
    let est_y = height.div_ceil(step_y);
    let mut out = Vec::with_capacity(est_x.saturating_mul(est_y));

    for y in (0..height).step_by(step_y) {
        let row_start = y * width * 4;
        for x in (0..width).step_by(step_x) {
            let i = row_start + x * 4;
            let Some(px) = image.rgba.get(i..i + 4) else {
                continue;
            };
            if px[3] >= 128 {
                out.push(Rgb::new(px[0], px[1], px[2]));
            }
        }
    }
    out
}

fn rgb_to_color(rgb: Rgb) -> Color {
    Color::from_rgb8(rgb.red, rgb.green, rgb.blue)
}

fn color_to_rgb(c: Color) -> Rgb {
    Rgb::new(channel_u8(c.r), channel_u8(c.g), channel_u8(c.b))
}

fn channel_u8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}
