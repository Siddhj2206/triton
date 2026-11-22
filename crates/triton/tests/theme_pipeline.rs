//! Integration tests for Triton's in-process `material-colors` live theming
//! pipeline: 28-role `Scheme` extraction with WCAG AA contrast, GTK3/4
//! `gtk.css` + niri `colors.kdl` atomic writers, `gsettings` gating, and live
//! `TritonShell` updates.

use std::path::PathBuf;
use std::sync::{Arc, mpsc};

use triton::{
    BackendMode, BarMode, IpcCommand, IpcResponse, ShellEvent, ShellMessage, SurfaceRole,
    ThemeCommand, ThemeSyncConfig, TritonShell, WallpaperImage, apply_gsettings, extract_scheme,
    extract_source_color, gsettings_color_scheme_args, render_gtk3_css, render_gtk4_css,
    scheme_from_seed, sync_config_files, write_gtk_css,
};
use triton_design::color::{Color, Scheme, Variant, contrast};
use triton_engine::{AppCommand, HeadlessSession, ImageData, OutputInfo, ShellApp};

fn solid_image(w: u32, h: u32, rgba: [u8; 4]) -> ImageData {
    let mut px = Vec::with_capacity((w * h * 4) as usize);
    for _ in 0..w * h {
        px.extend_from_slice(&rgba);
    }
    ImageData::from_rgba(w, h, px).expect("valid RGBA image")
}

fn gradient_image(w: u32, h: u32, left: [u8; 4], right: [u8; 4]) -> ImageData {
    let mut px = Vec::with_capacity((w * h * 4) as usize);
    for _y in 0..h {
        for x in 0..w {
            let t = if w <= 1 {
                0.0
            } else {
                x as f32 / (w - 1) as f32
            };
            let lerp = |a: u8, b: u8| -> u8 {
                (f32::from(a) + (f32::from(b) - f32::from(a)) * t).round() as u8
            };
            px.extend_from_slice(&[
                lerp(left[0], right[0]),
                lerp(left[1], right[1]),
                lerp(left[2], right[2]),
                255,
            ]);
        }
    }
    ImageData::from_rgba(w, h, px).expect("valid gradient image")
}

fn assert_aa_on_roles(label: &str, variant: Variant, s: &Scheme) {
    let pairs = [
        ("on_primary/primary", s.on_primary, s.primary),
        (
            "on_primary_container/primary_container",
            s.on_primary_container,
            s.primary_container,
        ),
        ("on_secondary/secondary", s.on_secondary, s.secondary),
        (
            "on_secondary_container/secondary_container",
            s.on_secondary_container,
            s.secondary_container,
        ),
        ("on_tertiary/tertiary", s.on_tertiary, s.tertiary),
        (
            "on_tertiary_container/tertiary_container",
            s.on_tertiary_container,
            s.tertiary_container,
        ),
        ("on_error/error", s.on_error, s.error),
        (
            "on_error_container/error_container",
            s.on_error_container,
            s.error_container,
        ),
        ("on_surface/surface", s.on_surface, s.surface),
        (
            "on_surface/surface_container_highest",
            s.on_surface,
            s.surface_container_highest,
        ),
        (
            "on_surface_variant/surface",
            s.on_surface_variant,
            s.surface,
        ),
    ];

    for (pair_name, fg, bg) in pairs {
        let ratio = contrast(fg, bg);
        assert!(
            ratio >= 4.5,
            "{label} ({variant:?}) {pair_name} failed WCAG AA: {ratio:.2}:1 (need >= 4.5:1)"
        );
    }
}

#[test]
fn extracts_28_role_schemes_with_aa_contrast_across_fixture_wallpapers() {
    let fixtures: [(&str, ImageData); 7] = [
        ("ocean-blue", solid_image(32, 32, [28, 92, 188, 255])),
        ("sunset-orange", solid_image(32, 32, [218, 96, 38, 255])),
        ("forest-emerald", solid_image(32, 32, [34, 152, 86, 255])),
        ("canary-yellow", solid_image(32, 32, [235, 205, 40, 255])),
        (
            "bright-pastel-gradient",
            gradient_image(64, 64, [245, 220, 235, 255], [220, 240, 255, 255]),
        ),
        ("dark-charcoal", solid_image(32, 32, [18, 20, 24, 255])),
        (
            "high-res-gradient",
            gradient_image(512, 288, [160, 40, 120, 255], [40, 120, 180, 255]),
        ),
    ];

    for (name, img) in &fixtures {
        let seed = extract_source_color(img);
        for variant in [Variant::Dark, Variant::Light] {
            let scheme = extract_scheme(img, variant);
            assert_eq!(scheme, scheme_from_seed(seed, variant));
            assert_aa_on_roles(name, variant, &scheme);

            // Surface containers step monotonically away from the surface.
            let containers = [
                scheme.surface_container_lowest,
                scheme.surface_container_low,
                scheme.surface_container,
                scheme.surface_container_high,
                scheme.surface_container_highest,
            ];
            assert!(
                containers
                    .windows(2)
                    .all(|w| contrast(w[0], scheme.on_surface) > contrast(w[1], scheme.on_surface)),
                "{name} ({variant:?}): surface containers must step away from surface"
            );

            // Semantic signal roles are preserved from the design system.
            let reference = Scheme::reference(variant);
            assert_eq!(scheme.signal_warn, reference.signal_warn);
            assert_eq!(scheme.signal_info, reference.signal_info);
        }
    }

    // Distinct chromatic wallpapers yield distinct primary colors (not the static reference).
    let blue_dark = extract_scheme(&fixtures[0].1, Variant::Dark);
    let orange_dark = extract_scheme(&fixtures[1].1, Variant::Dark);
    let green_light = extract_scheme(&fixtures[2].1, Variant::Light);
    assert_ne!(blue_dark.primary, orange_dark.primary);
    assert_ne!(green_light.primary, Scheme::LIGHT.primary);
    assert_ne!(
        extract_source_color(&fixtures[0].1),
        Color::TRANSPARENT,
        "seed color should be opaque"
    );
}

fn hex6(c: Color) -> String {
    format!(
        "#{:02x}{:02x}{:02x}",
        (c.r.clamp(0.0, 1.0) * 255.0).round() as u8,
        (c.g.clamp(0.0, 1.0) * 255.0).round() as u8,
        (c.b.clamp(0.0, 1.0) * 255.0).round() as u8,
    )
}

#[test]
fn writes_gtk3_gtk4_css_and_niri_colors_atomically_into_explicit_config_root() {
    let dir = tempfile::tempdir().expect("create temp config root");
    let root = dir.path();

    let img = solid_image(32, 32, [218, 96, 38, 255]);
    let dark = extract_scheme(&img, Variant::Dark);
    let light = extract_scheme(&img, Variant::Light);

    // Pure @define-color overrides: no layout/selector rules re-laying out apps.
    for variant in [Variant::Dark, Variant::Light] {
        let scheme = extract_scheme(&img, variant);
        for css in [
            render_gtk3_css(&scheme, variant),
            render_gtk4_css(&scheme, variant),
        ] {
            assert_ne!(css.trim(), "");
            for line in css.lines() {
                let trimmed = line.trim();
                if trimmed.is_empty() || trimmed.starts_with("/*") || trimmed.starts_with('*') {
                    continue;
                }
                assert!(
                    trimmed.starts_with("@define-color ") && trimmed.ends_with(';'),
                    "GTK css must only emit @define-color overrides, got: {trimmed}"
                );
            }
        }
    }

    // Atomic write of GTK3/4 + niri colors.kdl under the explicit config root.
    let paths = sync_config_files(root, &dark, Variant::Dark).expect("sync config files");
    assert_eq!(paths.gtk3, root.join("gtk-3.0").join("gtk.css"));
    assert_eq!(paths.gtk4, root.join("gtk-4.0").join("gtk.css"));
    assert_eq!(
        paths.niri_colors,
        root.join("niri").join("triton").join("colors.kdl")
    );

    let gtk3 = std::fs::read_to_string(&paths.gtk3).expect("read gtk3 css");
    let gtk4 = std::fs::read_to_string(&paths.gtk4).expect("read gtk4 css");
    let kdl = std::fs::read_to_string(&paths.niri_colors).expect("read colors.kdl");

    let primary_hex = hex6(dark.primary);
    let surface_hex = hex6(dark.surface);
    assert!(gtk3.contains(&format!(
        "@define-color theme_selected_bg_color {primary_hex};"
    )));
    assert!(gtk3.contains(&format!("@define-color theme_bg_color {surface_hex};")));
    assert!(gtk4.contains(&format!("@define-color accent_bg_color {primary_hex};")));
    assert!(gtk4.contains(&format!("@define-color window_bg_color {surface_hex};")));
    assert!(kdl.contains(&primary_hex));

    // No temp files left behind in any target directory.
    for sub in [
        root.join("gtk-3.0"),
        root.join("gtk-4.0"),
        root.join("niri").join("triton"),
    ] {
        for entry in std::fs::read_dir(&sub).expect("read dir") {
            let name = entry.expect("entry").file_name();
            let name = name.to_string_lossy();
            assert!(
                !name.contains(".tmp"),
                "stale temp file left in {}: {name}",
                sub.display()
            );
        }
    }

    // Re-writing with Light variant updates files atomically.
    let (gtk3_path, gtk4_path) =
        write_gtk_css(root, &light, Variant::Light).expect("rewrite gtk css");
    let gtk4_light = std::fs::read_to_string(&gtk4_path).expect("read light gtk4");
    assert_eq!(gtk3_path, paths.gtk3);
    assert!(gtk4_light.contains(&format!(
        "@define-color window_bg_color {};",
        hex6(light.surface)
    )));
    assert_ne!(gtk4, gtk4_light);
}

#[test]
fn gsettings_is_strictly_gated_behind_real_services() {
    assert_eq!(
        gsettings_color_scheme_args(Variant::Dark),
        [
            "set",
            "org.gnome.desktop.interface",
            "color-scheme",
            "prefer-dark",
        ]
    );
    assert_eq!(
        gsettings_color_scheme_args(Variant::Light),
        [
            "set",
            "org.gnome.desktop.interface",
            "color-scheme",
            "prefer-light",
        ]
    );

    // Fake dev mode must never invoke gsettings.
    assert!(!apply_gsettings(Variant::Dark, BackendMode::Fake).expect("noop in fake mode"));
    assert!(!apply_gsettings(Variant::Light, BackendMode::Fake).expect("noop in fake mode"));

    let dir = tempfile::tempdir().expect("temp dir");
    let files_only = ThemeSyncConfig::files_only(dir.path().to_path_buf());
    assert!(!files_only.apply_gsettings);
    let real = ThemeSyncConfig::real_services(dir.path().to_path_buf());
    assert!(real.apply_gsettings);
}

#[test]
fn wallpaper_load_and_theme_toggle_update_live_shell_scheme_and_redraw() {
    let mut shell = TritonShell::new(BackendMode::Fake);
    let mut session = HeadlessSession::<ShellMessage>::new((1920.0, 56.0), 1.0);

    // Pin the bar so we can observe live theme fill changes on its rendered quads.
    let _ = shell.handle_event(ShellEvent::SetBarMode(BarMode::Pinned), session.now());
    let (bar_id, bar_sizing) = shell
        .surfaces()
        .into_iter()
        .find_map(|(id, spec)| (id.role == SurfaceRole::Bar).then_some((id, spec.sizing)))
        .expect("pinned bar surface");

    let collect_fills = |session: &HeadlessSession<ShellMessage>| -> Vec<[f32; 4]> {
        session
            .draw_list()
            .batches
            .iter()
            .filter_map(|b| match b {
                triton_engine::gpu::DrawBatch::Quads { instances, .. } => Some(instances),
                _ => None,
            })
            .flat_map(|instances| instances.iter().map(|q| q.fill))
            .collect()
    };

    session.render_surface(&mut shell, bar_id, bar_sizing);
    let initial_quad_fills = collect_fills(&session);

    // Load a warm sunset-orange wallpaper into the shell.
    let orange_img = solid_image(32, 32, [218, 96, 38, 255]);
    let expected_dark = extract_scheme(&orange_img, Variant::Dark);
    let expected_light = extract_scheme(&orange_img, Variant::Light);
    assert_ne!(expected_dark.primary, Scheme::DARK.primary);
    assert_ne!(expected_light.primary, Scheme::LIGHT.primary);

    let path = PathBuf::from("/tmp/sunset-orange.png");
    let _ = shell.handle_event(
        ShellEvent::SetWallpaper {
            path: path.clone(),
            output: None,
        },
        session.now(),
    );
    let cmds = shell.handle_event(
        ShellEvent::WallpaperReady {
            image: Arc::new(WallpaperImage::from_data(path, orange_img)),
        },
        session.now(),
    );
    assert!(
        cmds.contains(&AppCommand::RedrawAll),
        "wallpaper theme extraction must request RedrawAll"
    );
    assert_eq!(shell.state().theme.variant, Variant::Dark);
    assert_eq!(shell.state().theme.scheme, expected_dark);

    // Rendering after theme extraction reflects the new scheme colors.
    session.render_surface(&mut shell, bar_id, bar_sizing);
    let dark_quad_fills = collect_fills(&session);
    assert_ne!(
        initial_quad_fills, dark_quad_fills,
        "bar quads must visually update with extracted scheme"
    );

    // `triton msg theme light` derives a dynamic Light scheme from the active
    // wallpaper instead of falling back to the hand-picked Scheme::LIGHT.
    let (reply_tx, reply_rx) = mpsc::channel();
    let cmds = shell.handle_event(
        ShellEvent::Ipc {
            command: IpcCommand::Theme {
                action: ThemeCommand::Light,
            },
            reply: reply_tx,
        },
        session.now(),
    );
    assert!(matches!(reply_rx.recv().unwrap(), IpcResponse::Ok { .. }));
    assert!(cmds.contains(&AppCommand::RedrawAll));
    assert_eq!(shell.state().theme.variant, Variant::Light);
    assert_eq!(shell.state().theme.scheme, expected_light);

    // `triton msg theme toggle` flips back to Dark using the active wallpaper.
    let (reply_tx, reply_rx) = mpsc::channel();
    let cmds = shell.handle_event(
        ShellEvent::Ipc {
            command: IpcCommand::Theme {
                action: ThemeCommand::ToggleVariant,
            },
            reply: reply_tx,
        },
        session.now(),
    );
    assert!(matches!(reply_rx.recv().unwrap(), IpcResponse::Ok { .. }));
    assert!(cmds.contains(&AppCommand::RedrawAll));
    assert_eq!(shell.state().theme.variant, Variant::Dark);
    assert_eq!(shell.state().theme.scheme, expected_dark);
}

#[test]
fn real_services_syncs_gtk_and_niri_files_while_fake_mode_writes_nothing() {
    let real_dir = tempfile::tempdir().expect("temp dir for real services");
    let mut real_shell = TritonShell::new(BackendMode::RealServices);
    real_shell.set_theme_sync(ThemeSyncConfig::files_only(real_dir.path().to_path_buf()));

    let session = HeadlessSession::<ShellMessage>::new((1920.0, 1080.0), 1.0);
    let _ = real_shell.handle_event(
        ShellEvent::OutputsChanged(vec![OutputInfo::new("DP-1", (1920, 1080))]),
        session.now(),
    );

    let emerald_img = solid_image(32, 32, [34, 152, 86, 255]);
    let expected_dark = extract_scheme(&emerald_img, Variant::Dark);
    let expected_light = extract_scheme(&emerald_img, Variant::Light);

    let path = PathBuf::from("/tmp/emerald.png");
    let _ = real_shell.handle_event(
        ShellEvent::SetWallpaper {
            path: path.clone(),
            output: None,
        },
        session.now(),
    );
    let _ = real_shell.handle_event(
        ShellEvent::WallpaperReady {
            image: Arc::new(WallpaperImage::from_data(path.clone(), emerald_img.clone())),
        },
        session.now(),
    );

    let gtk3_path = real_dir.path().join("gtk-3.0").join("gtk.css");
    let gtk4_path = real_dir.path().join("gtk-4.0").join("gtk.css");
    let kdl_path = real_dir
        .path()
        .join("niri")
        .join("triton")
        .join("colors.kdl");
    assert!(gtk3_path.exists());
    assert!(gtk4_path.exists());
    assert!(kdl_path.exists());

    let gtk4_dark = std::fs::read_to_string(&gtk4_path).expect("read dark gtk4");
    let kdl_dark = std::fs::read_to_string(&kdl_path).expect("read dark colors.kdl");
    assert!(gtk4_dark.contains(&hex6(expected_dark.primary)));
    assert!(kdl_dark.contains(&hex6(expected_dark.primary)));

    // Toggle to Light via IPC and confirm files regenerate with Light scheme.
    let (reply_tx, reply_rx) = mpsc::channel();
    let _ = real_shell.handle_event(
        ShellEvent::Ipc {
            command: IpcCommand::Theme {
                action: ThemeCommand::Light,
            },
            reply: reply_tx,
        },
        session.now(),
    );
    assert!(matches!(reply_rx.recv().unwrap(), IpcResponse::Ok { .. }));

    let gtk4_light = std::fs::read_to_string(&gtk4_path).expect("read light gtk4");
    let kdl_light = std::fs::read_to_string(&kdl_path).expect("read light colors.kdl");
    assert!(gtk4_light.contains(&hex6(expected_light.primary)));
    assert!(gtk4_light.contains(&hex6(expected_light.surface)));
    assert!(kdl_light.contains(&hex6(expected_light.primary)));

    // Fake dev mode must NEVER write desktop config files even if configured.
    let fake_dir = tempfile::tempdir().expect("temp dir for fake mode");
    let mut fake_shell = TritonShell::new(BackendMode::Fake);
    fake_shell.set_theme_sync(ThemeSyncConfig::files_only(fake_dir.path().to_path_buf()));
    let _ = fake_shell.handle_event(
        ShellEvent::SetWallpaper {
            path: path.clone(),
            output: None,
        },
        session.now(),
    );
    let _ = fake_shell.handle_event(
        ShellEvent::WallpaperReady {
            image: Arc::new(WallpaperImage::from_data(path, emerald_img)),
        },
        session.now(),
    );
    let (reply_tx, _reply_rx) = mpsc::channel();
    let _ = fake_shell.handle_event(
        ShellEvent::Ipc {
            command: IpcCommand::Theme {
                action: ThemeCommand::Light,
            },
            reply: reply_tx,
        },
        session.now(),
    );
    assert!(
        std::fs::read_dir(fake_dir.path())
            .expect("read fake dir")
            .next()
            .is_none(),
        "BackendMode::Fake must never write gtk.css or colors.kdl"
    );
}
