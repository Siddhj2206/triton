//! wallpaper -> scheme -> themes (roadmap §4 Appearance: "wallpaper -> scheme ->
//! shell theme + GTK4/libadwaita + GTK3 + gsettings + niri, applied live").
//!
//! [`current_wallpaper`] finds the image (the config file's one path, else the
//! newest image in ~/Pictures), [`load`] decodes it once — the background
//! surface gets the handle, Mica gets the mean luminance for its adaptive tint
//! (`material.mica_adaptive` in tokens.json) — [`schemes`] runs material-colors'
//! tonal-spot (matugen's engine and default variant) to fill the 28 `Scheme`
//! roles, and [`apply`] pushes the active variant to the outside world and
//! reports the targets that finished, for the toast. [`run`] is the whole chain
//! for the worker thread `app::set_wallpaper` kicks off.

use std::fmt::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::{fs, io};

use anyhow::Context;
use iced_core::Color;
use iced_widget::image::Handle;
use image::imageops::{self, FilterType};
use image::{ImageReader, RgbaImage};
use material_colors::color::Rgb;
use material_colors::dynamic_color::DynamicScheme;
use material_colors::hct::Hct;
use material_colors::image::extract_color;
use material_colors::scheme::variant::SchemeTonalSpot;
use triton_design::color::{Scheme, Variant, luminance};

/// What the pipeline worker hands back to the app: the wallpaper image and
/// both schemes.
#[derive(Debug)]
pub struct PipelineDone {
    pub wallpaper: Option<iced_widget::image::Handle>,
    /// Mean wallpaper luminance → Mica's adaptive tint.
    pub luminance: f32,
    pub dark: triton_design::Scheme,
    pub light: triton_design::Scheme,
    /// Targets the last `apply` finished (for its toast).
    pub applied: Vec<&'static str>,
}

/// Long side of the copy that goes to the quantizer; the engine's own docs warn
/// that quantize time grows with pixel count and suggest ~128 px.
const SCHEME_SIDE: u32 = 128;

/// Long side of the copy the luminance is averaged over.
const LUMINANCE_SIDE: u32 = 64;

/// The configured wallpaper: contents of `$XDG_CONFIG_HOME/triton/wallpaper`
/// (one path), else the most recently modified image in `~/Pictures`, else
/// `None`. Anything unreadable or missing falls through to the next source
/// with a warning.
pub fn current_wallpaper() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    let config = crate::util::config_root()?;
    wallpaper_in(&config, &home)
}

/// The two-step lookup behind [`current_wallpaper`], with the roots injected so
/// tests never touch the real home.
fn wallpaper_in(config: &Path, home: &Path) -> Option<PathBuf> {
    let file = config.join("triton").join("wallpaper");
    match fs::read_to_string(&file) {
        Ok(text) => {
            let path = expand_home(text.trim(), home);
            if path.as_os_str().is_empty() {
                // An empty file means "no choice"; fall through to the scan.
            } else if path.is_file() && is_image(&path) {
                return Some(path);
            } else {
                tracing::warn!(
                    path = %path.display(),
                    "configured wallpaper is not a readable image"
                );
            }
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => {
            tracing::warn!(%e, path = %file.display(), "cannot read the wallpaper config");
        }
    }
    newest_image(&home.join("Pictures"))
}

/// `~/…` in the config file means the real home; anything else stays as written.
fn expand_home(text: &str, home: &Path) -> PathBuf {
    match text.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None if text == "~" => home.to_path_buf(),
        None => PathBuf::from(text),
    }
}

/// The newest jpg/png/webp directly in `dir` — not recursive: a Pictures folder
/// can be arbitrarily deep, and this runs on shell start.
fn newest_image(dir: &Path) -> Option<PathBuf> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return None,
        Err(e) => {
            tracing::warn!(%e, dir = %dir.display(), "cannot scan for wallpapers");
            return None;
        }
    };
    entries
        .filter_map(Result::ok)
        .filter(|entry| is_image(&entry.path()))
        .filter_map(|entry| {
            let meta = entry.metadata().ok()?;
            if !meta.is_file() {
                return None;
            }
            Some((entry.path(), meta.modified().ok()?))
        })
        .max_by_key(|(_, modified)| *modified)
        .map(|(path, _)| path)
}

/// `path` names an image we decode: jpg, png or webp by extension.
fn is_image(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            matches!(
                ext.to_ascii_lowercase().as_str(),
                "jpg" | "jpeg" | "png" | "webp"
            )
        })
}

/// Persist a wallpaper choice: `$XDG_CONFIG_HOME/triton/wallpaper` gets one path.
pub fn set_wallpaper(path: &Path) -> io::Result<()> {
    let root = crate::util::config_root()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no HOME or XDG_CONFIG_HOME"))?;
    set_wallpaper_in(&root, path)
}

/// [`set_wallpaper`] with the config root injected, for tests.
fn set_wallpaper_in(config: &Path, path: &Path) -> io::Result<()> {
    let text = path.to_str().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "wallpaper path is not UTF-8")
    })?;
    crate::util::write_if_changed(&config.join("triton"), "wallpaper", &format!("{text}\n"))?;
    Ok(())
}

/// A decoded wallpaper: the pixels the background surface draws, plus the number
/// Mica needs.
#[derive(Debug, Clone)]
pub struct Wallpaper {
    /// Full-res handle for the background surface to draw (`ContentFit::Cover`).
    #[expect(dead_code, reason = "the Phase 2 wallpaper picker previews with it")]
    pub handle: Handle,
    /// Mean luminance of a downscaled copy; feeds Mica's adaptive tint.
    // Read only by tests today, like `load` — `expect` fails unfulfilled there.
    #[allow(dead_code)]
    pub luminance: f32,
}

/// Decode `path` once: the handle owns the full-res pixels, a downscaled copy
/// gives the mean luminance.
// Dead in the binary but exercised by tests — `expect` would fail unfulfilled
// in the test build, so this keeps a targeted `allow`.
#[allow(dead_code)]
pub fn load(path: &Path) -> anyhow::Result<Wallpaper> {
    let rgba = decode(path)?;
    let thumb = downscale(&rgba, LUMINANCE_SIDE);
    Ok(Wallpaper {
        handle: Handle::from_rgba(rgba.width(), rgba.height(), rgba.into_raw()),
        luminance: mean_luminance(&thumb),
    })
}

/// Pixels out of `path`. The extension is checked first so a mistyped path
/// fails with a clear error; the reader then sniffs the real format, so a file
/// named wrong but decodable still loads.
fn decode(path: &Path) -> anyhow::Result<RgbaImage> {
    anyhow::ensure!(is_image(path), "not an image: {}", path.display());
    Ok(ImageReader::open(path)
        .with_context(|| format!("cannot open {}", path.display()))?
        .with_guessed_format()
        .with_context(|| format!("cannot sniff the format of {}", path.display()))?
        .decode()
        .with_context(|| format!("cannot decode {}", path.display()))?
        .to_rgba8())
}

/// Dark and light schemes from the image via material-colors tonal-spot —
/// matugen's engine and default variant. Quantizing a full-res wallpaper is
/// slow, so the copy going in is cut to ~128 px on the long side.
pub fn schemes(image: &RgbaImage) -> (Scheme, Scheme) {
    let thumb = downscale(image, SCHEME_SIDE);
    let pixels: Vec<Rgb> = thumb
        .pixels()
        .map(|p| Rgb::new(p.0[0], p.0[1], p.0[2]))
        .collect();
    let source = if pixels.is_empty() {
        Hct::new(Rgb::new(0, 0, 0))
    } else {
        Hct::new(extract_color(&pixels))
    };
    (
        scheme_of(
            &SchemeTonalSpot::new(source, true, None).scheme,
            Variant::Dark,
        ),
        scheme_of(
            &SchemeTonalSpot::new(source, false, None).scheme,
            Variant::Light,
        ),
    )
}

/// The engine's role getters map onto `Scheme`'s fields by name.
fn scheme_of(s: &DynamicScheme, variant: Variant) -> Scheme {
    let c = |rgb: Rgb| Color::from_rgb8(rgb.red, rgb.green, rgb.blue);
    let reference = Scheme::reference(variant);
    Scheme {
        primary: c(s.primary()),
        on_primary: c(s.on_primary()),
        primary_container: c(s.primary_container()),
        on_primary_container: c(s.on_primary_container()),
        secondary: c(s.secondary()),
        on_secondary: c(s.on_secondary()),
        secondary_container: c(s.secondary_container()),
        on_secondary_container: c(s.on_secondary_container()),
        tertiary: c(s.tertiary()),
        on_tertiary: c(s.on_tertiary()),
        tertiary_container: c(s.tertiary_container()),
        on_tertiary_container: c(s.on_tertiary_container()),
        error: c(s.error()),
        on_error: c(s.on_error()),
        error_container: c(s.error_container()),
        on_error_container: c(s.on_error_container()),
        surface: c(s.surface()),
        surface_container_lowest: c(s.surface_container_lowest()),
        surface_container_low: c(s.surface_container_low()),
        surface_container: c(s.surface_container()),
        surface_container_high: c(s.surface_container_high()),
        surface_container_highest: c(s.surface_container_highest()),
        on_surface: c(s.on_surface()),
        on_surface_variant: c(s.on_surface_variant()),
        outline: c(s.outline()),
        outline_variant: c(s.outline_variant()),
        // signal_warn/signal_info are triton-only roles — the M3 engine never
        // emits them — so both variants keep the canvas reference values.
        signal_warn: reference.signal_warn,
        signal_info: reference.signal_info,
    }
}

/// `image` shrunk so its long side is at most `side`; already-small inputs are
/// cloned untouched.
fn downscale(image: &RgbaImage, side: u32) -> RgbaImage {
    let (w, h) = image.dimensions();
    let long = w.max(h);
    if long <= side {
        return image.clone();
    }
    let scale = f64::from(side) / f64::from(long);
    let nw = ((f64::from(w) * scale).round() as u32).max(1);
    let nh = ((f64::from(h) * scale).round() as u32).max(1);
    imageops::resize(image, nw, nh, FilterType::Triangle)
}

/// WCAG luminance of the average pixel, alpha composited over black — niri
/// draws the wallpaper over black, so a transparent edge should count as dark.
fn mean_luminance(image: &RgbaImage) -> f32 {
    let n = u64::from(image.width()) * u64::from(image.height());
    if n == 0 {
        return 0.0;
    }
    let (mut r, mut g, mut b) = (0u64, 0u64, 0u64);
    for p in image.pixels() {
        let [pr, pg, pb, pa] = p.0;
        r += u64::from(pr) * u64::from(pa);
        g += u64::from(pg) * u64::from(pa);
        b += u64::from(pb) * u64::from(pa);
    }
    let channel = |sum: u64| (sum as f64 / (f64::from(u8::MAX) * n as f64)).round() as u8;
    luminance(Color::from_rgb8(channel(r), channel(g), channel(b)))
}

/// Apply a scheme to the outside world; returns the names of the targets that
/// finished, for the "scheme applied to: …" toast the roadmap describes. The
/// GTK3 and GTK4 `gtk.css` files get the libadwaita color variables for the
/// active variant, and `gsettings` flips `color-scheme`. Per-target failures
/// warn and drop the target from the list. (niri's colors come from
/// `niri_config::install`, which owns its own files; the shell theme is the
/// `Scheme` itself.)
pub fn apply(scheme: &Scheme, variant: Variant) -> Vec<&'static str> {
    let Some(config) = crate::util::config_root() else {
        tracing::warn!("no HOME or XDG_CONFIG_HOME; nothing applied");
        return Vec::new();
    };
    apply_to(scheme, variant, &config, true)
}

/// The whole pipeline for `app::set_wallpaper`'s worker thread: decode once,
/// derive both schemes, apply the active one to the theme targets, and hand the
/// app the wallpaper handle, the luminance and the finished-target list. A
/// decode failure degrades to the reference schemes and no wallpaper — the
/// shell keeps running on tokens.
pub fn run(path: &Path, variant: Variant) -> PipelineDone {
    let rgba = match decode(path) {
        Ok(rgba) => rgba,
        Err(e) => {
            tracing::warn!(%e, path = %path.display(), "cannot load the wallpaper");
            return PipelineDone {
                wallpaper: None,
                // Well under Mica's 0.6 adaptive-tint trigger: assume a dark
                // backdrop when there is no image to measure.
                luminance: 0.2,
                dark: Scheme::DARK,
                light: Scheme::LIGHT,
                applied: Vec::new(),
            };
        }
    };
    let (dark, light) = schemes(&rgba);
    let luminance = mean_luminance(&downscale(&rgba, LUMINANCE_SIDE));
    let handle = Handle::from_rgba(rgba.width(), rgba.height(), rgba.into_raw());
    let scheme = match variant {
        Variant::Dark => dark,
        Variant::Light => light,
    };
    let applied = apply(&scheme, variant);
    PipelineDone {
        wallpaper: Some(handle),
        luminance,
        dark,
        light,
        applied,
    }
}

/// The targets behind [`apply`], with the config root injected and the
/// `gsettings` spawn behind a flag — tests pass a tempdir and `false`, and
/// never touch D-Bus.
fn apply_to(
    scheme: &Scheme,
    variant: Variant,
    config: &Path,
    gsettings: bool,
) -> Vec<&'static str> {
    let mut done = Vec::new();
    let css = gtk_css(scheme, variant);
    for (name, dir) in [
        ("gtk-3.0", config.join("gtk-3.0")),
        ("gtk-4.0", config.join("gtk-4.0")),
    ] {
        match crate::util::write_if_changed(&dir, "gtk.css", &css) {
            Ok(_) => done.push(name),
            Err(e) => tracing::warn!(%e, dir = %dir.display(), "cannot write gtk.css"),
        }
    }
    if gsettings {
        // `spawn` only: Command::output/status are clippy-disallowed, and this
        // runs on a user action anyway. A missing binary means a non-GNOME
        // session — a warn, not an error.
        let value = match variant {
            Variant::Dark => "prefer-dark",
            Variant::Light => "prefer-light",
        };
        match Command::new("gsettings")
            .args(["set", "org.gnome.desktop.interface", "color-scheme", value])
            .spawn()
        {
            Ok(_) => done.push("gsettings"),
            Err(e) => tracing::warn!(%e, "cannot run gsettings; color-scheme unchanged"),
        }
    }
    done
}

/// A matugen-style libadwaita theme for one variant: `@define-color` for the
/// public variables, M3 roles mapped by intent. Chrome surfaces (headerbar,
/// sidebar) take the Mica fill `surface_container_low`; floating surfaces
/// (popover, dialog) the Glass/Thick fill `surface_container_high` — the same
/// materials the shell uses (`material.*` in tokens.json).
fn gtk_css(s: &Scheme, variant: Variant) -> String {
    let dark = matches!(variant, Variant::Dark);
    // Shade alphas approximate libadwaita's own defaults; they don't track the
    // seed color.
    let (shade, darker_shade) = if dark {
        ("rgba(0, 0, 0, 0.36)", "rgba(0, 0, 0, 0.45)")
    } else {
        ("rgba(0, 0, 0, 0.07)", "rgba(0, 0, 0, 0.12)")
    };
    // success_* has no M3 or triton role, so it is a fixed GNOME-green hue at
    // tones that stay legible in the variant. warning_* maps to signal_warn;
    // the on-window text tone darkens in the light variant (amber on white is
    // unreadable), like libadwaita's own light warning color.
    let success_bg = hct_color(150.0, 80.0, 75.0);
    let success_fg = hct_color(150.0, 45.0, 18.0);
    let success_text = hct_color(150.0, 80.0, if dark { 78.0 } else { 45.0 });
    let warning_fg = hct_color(80.0, 45.0, 18.0);
    let warning_text = if dark {
        s.signal_warn
    } else {
        hct_color(70.0, 90.0, 45.0)
    };
    let colors = [
        ("window_bg_color", s.surface),
        ("window_fg_color", s.on_surface),
        ("view_bg_color", s.surface_container_lowest),
        ("view_fg_color", s.on_surface),
        ("accent_bg_color", s.primary),
        ("accent_fg_color", s.on_primary),
        ("accent_color", s.primary),
        ("destructive_bg_color", s.error),
        ("destructive_fg_color", s.on_error),
        ("destructive_color", s.error),
        ("success_bg_color", success_bg),
        ("success_fg_color", success_fg),
        ("success_color", success_text),
        ("warning_bg_color", s.signal_warn),
        ("warning_fg_color", warning_fg),
        ("warning_color", warning_text),
        ("error_bg_color", s.error),
        ("error_fg_color", s.on_error),
        ("error_color", s.error),
        ("headerbar_bg_color", s.surface_container_low),
        ("headerbar_fg_color", s.on_surface),
        ("headerbar_border_color", s.outline_variant),
        ("headerbar_backdrop_color", s.surface),
        ("sidebar_bg_color", s.surface_container_low),
        ("sidebar_fg_color", s.on_surface),
        ("sidebar_backdrop_color", s.surface),
        ("sidebar_border_color", s.outline_variant),
        ("popover_bg_color", s.surface_container_high),
        ("popover_fg_color", s.on_surface),
        ("card_bg_color", s.surface_container),
        ("card_fg_color", s.on_surface),
        ("dialog_bg_color", s.surface_container_high),
        ("dialog_fg_color", s.on_surface),
        ("scrollbar_outline_color", s.outline_variant),
    ];
    let literals = [
        ("headerbar_shade_color", shade),
        ("headerbar_darker_shade_color", darker_shade),
        ("sidebar_shade_color", shade),
        ("popover_shade_color", shade),
        ("card_shade_color", shade),
        ("shade_color", shade),
        // Opaque stops GTK themes reference in shadows and borders.
        ("light_1", "#ffffff"),
        ("dark_1", "#000000"),
        ("dark_2", "#141414"),
        ("dark_3", "#1e1e1e"),
        ("dark_4", "#282828"),
        ("dark_5", "#323232"),
    ];
    let mut css = String::from(
        "/* Written by triton. Changes here are overwritten; pick a new wallpaper\n   or scheme in the settings app instead. */\n\n",
    );
    for (name, color) in colors {
        let _ = writeln!(css, "@define-color {name} {};", hex(color));
    }
    for (name, value) in literals {
        let _ = writeln!(css, "@define-color {name} {value};");
    }
    css
}

/// An HCT triple as an iced `Color` — for the signal colors the scheme doesn't
/// carry (success) and the on-colors of the ones it does (warning).
fn hct_color(hue: f64, chroma: f64, tone: f64) -> Color {
    let rgb = Rgb::from(Hct::from(hue, chroma, tone));
    Color::from_rgb8(rgb.red, rgb.green, rgb.blue)
}

/// `#rrggbb`, as `gtk.css` takes colors (`niri_config::hex` does `#rrggbbaa`).
fn hex(color: Color) -> String {
    let [r, g, b, _] = color.into_rgba8();
    format!("#{r:02x}{g:02x}{b:02x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small two-tone image — enough pixels for the quantizer to find a
    /// chromatic source color.
    fn test_image() -> RgbaImage {
        RgbaImage::from_fn(64, 64, |x, _| {
            if x < 32 {
                image::Rgba([50, 90, 200, 255])
            } else {
                image::Rgba([60, 150, 90, 255])
            }
        })
    }

    #[test]
    fn schemes_produce_distinct_containers_and_variants() {
        let (dark, light) = schemes(&test_image());
        for scheme in [dark, light] {
            let containers = [
                scheme.surface_container_lowest,
                scheme.surface_container_low,
                scheme.surface_container,
                scheme.surface_container_high,
                scheme.surface_container_highest,
            ];
            for (i, a) in containers.iter().enumerate() {
                for b in &containers[i + 1..] {
                    assert_ne!(a, b, "container tones repeat");
                }
            }
        }
        assert_ne!(dark, light);
        // signal_* aren't M3 roles: the reference values carry over.
        assert_eq!(dark.signal_warn, Scheme::DARK.signal_warn);
        assert_eq!(dark.signal_info, Scheme::DARK.signal_info);
        assert_eq!(light.signal_warn, Scheme::LIGHT.signal_warn);
        assert_eq!(light.signal_info, Scheme::LIGHT.signal_info);
    }

    #[test]
    fn a_dark_wallpaper_gives_a_dark_surface() {
        let img = RgbaImage::from_pixel(48, 48, image::Rgba([10, 12, 18, 255]));
        let (dark, _) = schemes(&img);
        assert!(
            luminance(dark.surface) < 0.05,
            "surface luminance {}",
            luminance(dark.surface)
        );
    }

    #[test]
    fn load_decodes_and_rejects() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("waves.png");
        image::DynamicImage::ImageRgba8(test_image())
            .save(&path)
            .unwrap();
        let wallpaper = load(&path).unwrap();
        assert!(wallpaper.luminance > 0.0 && wallpaper.luminance < 1.0);

        // Rejected by extension, and by decode failure.
        let text = dir.path().join("notes.txt");
        fs::write(&text, b"pretends to be an image").unwrap();
        assert!(load(&text).is_err());
        let broken = dir.path().join("broken.png");
        fs::write(&broken, b"not a png").unwrap();
        assert!(load(&broken).is_err());
    }

    #[test]
    fn wallpaper_lookup_prefers_config_then_pictures() {
        let dir = tempfile::tempdir().unwrap();
        let (config, home) = (dir.path().join("config"), dir.path().join("home"));
        fs::create_dir_all(home.join("Pictures")).unwrap();

        // Nothing configured: the image in ~/Pictures; other extensions skip.
        let pic = home.join("Pictures").join("shot.png");
        fs::write(&pic, b"not-really-png").unwrap();
        fs::write(home.join("Pictures").join("notes.txt"), b"x").unwrap();
        assert_eq!(wallpaper_in(&config, &home), Some(pic.clone()));

        // A configured path wins.
        let chosen = home.join("waves.jpg");
        fs::write(&chosen, b"").unwrap();
        set_wallpaper_in(&config, &chosen).unwrap();
        assert_eq!(wallpaper_in(&config, &home), Some(chosen.clone()));

        // A configured path that vanished falls back to the scan.
        fs::remove_file(&chosen).unwrap();
        assert_eq!(wallpaper_in(&config, &home), Some(pic));
    }

    #[test]
    fn apply_writes_gtk_css_for_the_variant() {
        let dir = tempfile::tempdir().unwrap();
        let done = apply_to(&Scheme::DARK, Variant::Dark, dir.path(), false);
        assert_eq!(done, ["gtk-3.0", "gtk-4.0"]);
        for sub in ["gtk-3.0", "gtk-4.0"] {
            let css = fs::read_to_string(dir.path().join(sub).join("gtk.css")).unwrap();
            assert!(css.contains("Written by triton"));
            assert!(css.contains(&format!(
                "@define-color window_bg_color {}",
                hex(Scheme::DARK.surface)
            )));
        }

        // The other variant rewrites the files.
        let done = apply_to(&Scheme::LIGHT, Variant::Light, dir.path(), false);
        assert_eq!(done, ["gtk-3.0", "gtk-4.0"]);
        let css = fs::read_to_string(dir.path().join("gtk-4.0").join("gtk.css")).unwrap();
        assert!(css.contains(&format!(
            "@define-color window_bg_color {}",
            hex(Scheme::LIGHT.surface)
        )));
    }
}
