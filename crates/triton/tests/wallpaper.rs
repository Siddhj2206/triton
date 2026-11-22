//! Integration tests for the per-output wallpaper pipeline: off-thread decode,
//! GPU upload via `sync_images`, 200 ms crossfade, and mean-luminance detection.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use triton::{
    BRIGHT_BACKDROP_THRESHOLD, BackendMode, ShellEvent, ShellMessage, SurfaceRole, TritonShell,
    WallpaperImage, mean_relative_luminance,
};
use triton_engine::{
    AppCommand, HeadlessSession, ImageData, ImageId, OutputInfo, ShellApp, SurfaceLayer,
    event_channel, write_png,
};

fn solid_rgba(w: u32, h: u32, rgba: [u8; 4]) -> ImageData {
    let mut px = Vec::with_capacity((w * h * 4) as usize);
    for _ in 0..w * h {
        px.extend_from_slice(&rgba);
    }
    ImageData::from_rgba(w, h, px).expect("valid image data")
}

fn wallpaper(path: &str, rgba: [u8; 4]) -> WallpaperImage {
    WallpaperImage::from_data(PathBuf::from(path), solid_rgba(8, 8, rgba))
}

fn key_of(shell: &TritonShell, output: &str) -> triton::OutputKey {
    shell
        .surfaces()
        .iter()
        .filter(|(id, _)| id.role == SurfaceRole::Background)
        .find_map(|(id, _)| {
            let (_, name) = shell.surface_descriptor(*id)?;
            (name == output).then_some(id.output)
        })
        .expect("output has a background surface")
}

#[test]
fn mean_luminance_distinguishes_dark_and_bright_images() {
    let dark = wallpaper("/tmp/dark.png", [0, 0, 0, 255]);
    let bright = wallpaper("/tmp/bright.png", [255, 255, 255, 255]);

    assert!(dark.mean_luminance < 0.05);
    assert!(bright.mean_luminance > 0.95);

    // Per-pixel luminance equals luminance of the mean color for uniform images.
    let mid = solid_rgba(4, 4, [128, 128, 128, 255]);
    let lum = mean_relative_luminance(&mid);
    assert!(
        lum > 0.2 && lum < 0.3,
        "mid-gray sRGB ~0.216 linear, got {lum}"
    );

    // Fully transparent pixels contribute nothing (alpha-weighted over black).
    let transparent = solid_rgba(2, 2, [255, 255, 255, 0]);
    assert!(mean_relative_luminance(&transparent) < 0.01);
}

/// Deliver a `WallpaperReady` event and let `sync_images` upload the pixels.
fn deliver_ready(
    shell: &mut TritonShell,
    session: &mut HeadlessSession<ShellMessage>,
    image: WallpaperImage,
) -> Vec<AppCommand<triton::SurfaceId>> {
    let cmds = shell.handle_event(
        ShellEvent::WallpaperReady {
            image: Arc::new(image),
        },
        session.now(),
    );
    session.sync_app_images(shell);
    cmds
}

fn background_id(shell: &TritonShell, output: &str) -> triton::SurfaceId {
    shell
        .surfaces()
        .iter()
        .find_map(|(id, spec)| {
            let (role, name) = shell.surface_descriptor(*id)?;
            (role == SurfaceRole::Background && name == output).then_some((*id, spec.sizing))
        })
        .expect("background surface exists")
        .0
}

fn image_batches(session: &HeadlessSession<ShellMessage>) -> Vec<(ImageId, Option<ImageId>, f32)> {
    session
        .draw_list()
        .batches
        .iter()
        .filter_map(|b| match b {
            triton_engine::gpu::DrawBatch::Images {
                image_id,
                crossfade_to,
                instances,
                ..
            } => Some((*image_id, *crossfade_to, instances.first()?.fx[0])),
            _ => None,
        })
        .collect()
}

#[test]
fn wallpaper_decodes_uploads_and_displays_per_output() {
    let mut shell = TritonShell::new(BackendMode::Fake);
    let mut session = HeadlessSession::new((1920.0, 1080.0), 1.0);
    let (tx, rx) = event_channel::<ShellEvent>();
    shell.set_event_sender(tx.clone());

    // Request a wallpaper via the ShellEvent seam (same as `triton msg wallpaper`).
    tx.send(ShellEvent::SetWallpaper {
        path: PathBuf::from("/tmp/wp-a.png"),
        output: None,
    })
    .unwrap();
    let _ = session.drain_events(&mut shell, &rx, TritonShell::handle_event);

    // Background surfaces mount on every connected output on Layer::Background.
    let bg = shell
        .surfaces()
        .into_iter()
        .find(|(id, _)| id.role == SurfaceRole::Background)
        .expect("background mounted");
    assert_eq!(bg.1.layer, SurfaceLayer::Background);
    assert!(matches!(bg.1.sizing, triton_engine::SurfaceSize::Stretch));

    // Deliver the decoded image as the off-thread worker would.
    let cmds = deliver_ready(
        &mut shell,
        &mut session,
        wallpaper("/tmp/wp-a.png", [10, 20, 30, 255]),
    );
    assert!(cmds.contains(&AppCommand::RedrawAll));

    // Uploaded to the ImageStore and displayed on the output.
    let key = key_of(&shell, "eDP-1");
    let current = shell
        .wallpapers()
        .current_for(key)
        .expect("image displayed");
    assert_eq!(session.images().dimensions(current), Some((8, 8)));

    // First wallpaper shows immediately (no crossfade without a predecessor).
    assert!(!shell.wallpapers().is_animating(key, session.now()));
    let (bg_id, bg_sizing) = (
        background_id(&shell, "eDP-1"),
        triton_engine::SurfaceSize::Stretch,
    );
    session.render_surface(&mut shell, bg_id, bg_sizing);
    let batches = image_batches(&session);
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].0, current);
    assert_eq!(batches[0].1, None);

    // The decoded image is exposed for #19 via shell state.
    let active = shell
        .state()
        .wallpaper
        .active_image
        .as_ref()
        .expect("active image");
    assert_eq!(active.data.rgba.len(), 8 * 8 * 4);
}

#[test]
fn wallpaper_crossfades_200ms_and_releases_old_image_on_settle() {
    let mut shell = TritonShell::new(BackendMode::Fake);
    let mut session = HeadlessSession::new((1920.0, 1080.0), 1.0);
    let (tx, rx) = event_channel::<ShellEvent>();
    shell.set_event_sender(tx.clone());
    let key = key_of(&shell, "eDP-1");

    // Display wallpaper A.
    tx.send(ShellEvent::SetWallpaper {
        path: PathBuf::from("/tmp/wp-a.png"),
        output: None,
    })
    .unwrap();
    let _ = session.drain_events(&mut shell, &rx, TritonShell::handle_event);
    let _ = deliver_ready(
        &mut shell,
        &mut session,
        wallpaper("/tmp/wp-a.png", [0, 0, 0, 255]),
    );
    let old_id = shell.wallpapers().current_for(key).unwrap();

    // Switch to wallpaper B mid-stream.
    tx.send(ShellEvent::SetWallpaper {
        path: PathBuf::from("/tmp/wp-b.png"),
        output: None,
    })
    .unwrap();
    let _ = session.drain_events(&mut shell, &rx, TritonShell::handle_event);
    let cmds = deliver_ready(
        &mut shell,
        &mut session,
        wallpaper("/tmp/wp-b.png", [255, 255, 255, 255]),
    );

    // The settle timer is armed for the 200 ms crossfade.
    assert!(cmds.contains(&AppCommand::ScheduleTimer {
        tag: triton::TIMER_WALLPAPER_SETTLE,
        delay: Duration::from_millis(200),
    }));

    // Fade starts at t=0 blending old -> new.
    let fade = shell.wallpapers().fade_for(key).expect("crossfade active");
    assert_eq!(fade.from, old_id);
    assert!(shell.is_animating(background_id(&shell, "eDP-1"), session.now()));

    let bg_id = background_id(&shell, "eDP-1");
    session.render_surface(&mut shell, bg_id, triton_engine::SurfaceSize::Stretch);
    let batches = image_batches(&session);
    assert_eq!(batches[0].0, old_id);
    assert_eq!(batches[0].1, shell.wallpapers().current_for(key));
    assert!(batches[0].2 < 0.05);

    // Mid-fade: ~50% blend, still animating.
    session.advance(Duration::from_millis(100));
    assert!(shell.is_animating(bg_id, session.now()));
    session.render_surface(&mut shell, bg_id, triton_engine::SurfaceSize::Stretch);
    let t = image_batches(&session)[0].2;
    assert!(t > 0.4 && t < 0.6, "mid-fade t ≈ 0.5, got {t}");

    // Past the deadline the fade settles visually and animation stops (0% idle).
    session.advance(Duration::from_millis(150));
    assert!(!shell.is_animating(bg_id, session.now()));

    // The settle timer releases the outgoing texture once the fade completes.
    let cmds = shell.on_timer(triton::TIMER_WALLPAPER_SETTLE, session.now());
    assert!(cmds.contains(&AppCommand::RedrawAll));
    assert!(shell.wallpapers().fade_for(key).is_none());
    session.sync_app_images(&mut shell);
    assert_eq!(
        session.images().dimensions(old_id),
        None,
        "outgoing texture released after settle"
    );
    let new_id = shell.wallpapers().current_for(key).unwrap();
    assert_eq!(session.images().dimensions(new_id), Some((8, 8)));

    // Settled view renders the new image alone.
    session.render_surface(&mut shell, bg_id, triton_engine::SurfaceSize::Stretch);
    let batches = image_batches(&session);
    assert_eq!(batches[0].0, new_id);
    assert_eq!(batches[0].1, None);
}

#[test]
fn wallpaper_crossfade_uses_120ms_under_reduce_motion() {
    let mut shell = TritonShell::new(BackendMode::Fake);
    let mut session = HeadlessSession::new((1920.0, 1080.0), 1.0);
    let (tx, rx) = event_channel::<ShellEvent>();
    shell.set_event_sender(tx.clone());
    let key = key_of(&shell, "eDP-1");

    tx.send(ShellEvent::SetReduceMotion(true)).unwrap();
    tx.send(ShellEvent::SetWallpaper {
        path: PathBuf::from("/tmp/wp-a.png"),
        output: None,
    })
    .unwrap();
    let _ = session.drain_events(&mut shell, &rx, TritonShell::handle_event);
    let _ = deliver_ready(
        &mut shell,
        &mut session,
        wallpaper("/tmp/wp-a.png", [0, 0, 0, 255]),
    );

    tx.send(ShellEvent::SetWallpaper {
        path: PathBuf::from("/tmp/wp-b.png"),
        output: None,
    })
    .unwrap();
    let _ = session.drain_events(&mut shell, &rx, TritonShell::handle_event);
    let cmds = deliver_ready(
        &mut shell,
        &mut session,
        wallpaper("/tmp/wp-b.png", [255, 255, 255, 255]),
    );

    assert!(cmds.contains(&AppCommand::ScheduleTimer {
        tag: triton::TIMER_WALLPAPER_SETTLE,
        delay: Duration::from_millis(120),
    }));

    // 120 ms linear fade settles earlier than the normal 200 ms window.
    session.advance(Duration::from_millis(125));
    let bg_id = background_id(&shell, "eDP-1");
    assert!(!shell.is_animating(bg_id, session.now()));
    let _ = shell.on_timer(triton::TIMER_WALLPAPER_SETTLE, session.now());
    assert!(shell.wallpapers().fade_for(key).is_none());
}

#[test]
fn wallpaper_covers_new_outputs_on_hotplug_and_drops_removed_ones() {
    let mut shell = TritonShell::new(BackendMode::Fake);
    let mut session = HeadlessSession::new((1920.0, 1080.0), 1.0);
    let (tx, rx) = event_channel::<ShellEvent>();
    shell.set_event_sender(tx.clone());
    let _ = session.set_outputs(&mut shell, vec![OutputInfo::new("eDP-1", (1920, 1080))]);

    tx.send(ShellEvent::SetWallpaper {
        path: PathBuf::from("/tmp/wp.png"),
        output: None,
    })
    .unwrap();
    let _ = session.drain_events(&mut shell, &rx, TritonShell::handle_event);
    let _ = deliver_ready(
        &mut shell,
        &mut session,
        wallpaper("/tmp/wp.png", [40, 50, 60, 255]),
    );

    // Hotplug a second output: it immediately shows the default wallpaper
    // (same decoded image shared, no re-decode).
    let _ = session.add_output(&mut shell, OutputInfo::new("DP-1", (2560, 1440)));
    let dp_key = key_of(&shell, "DP-1");
    let edp_key = key_of(&shell, "eDP-1");
    let shared = shell.wallpapers().current_for(edp_key).unwrap();
    assert_eq!(shell.wallpapers().current_for(dp_key), Some(shared));

    // Per-output override retargets only that output.
    tx.send(ShellEvent::SetWallpaper {
        path: PathBuf::from("/tmp/wp-dp.png"),
        output: Some("DP-1".to_owned()),
    })
    .unwrap();
    let _ = session.drain_events(&mut shell, &rx, TritonShell::handle_event);
    let _ = deliver_ready(
        &mut shell,
        &mut session,
        wallpaper("/tmp/wp-dp.png", [200, 200, 200, 255]),
    );
    let dp_new = shell.wallpapers().current_for(dp_key).unwrap();
    assert_ne!(dp_new, shared);
    assert_eq!(shell.wallpapers().current_for(edp_key), Some(shared));

    // Let the DP-1 fade settle; the shared texture is still held by eDP-1.
    session.advance(Duration::from_millis(250));
    let _ = shell.on_timer(triton::TIMER_WALLPAPER_SETTLE, session.now());
    session.sync_app_images(&mut shell);
    assert_eq!(session.images().dimensions(shared), Some((8, 8)));

    // Removing eDP-1 unmounts its surfaces and releases its texture reference.
    let _ = session.remove_output(&mut shell, "eDP-1");
    session.sync_app_images(&mut shell);
    assert!(
        shell
            .surfaces()
            .iter()
            .all(|(id, _)| id.role != SurfaceRole::Background
                || shell.surface_descriptor(*id).map(|(_, n)| n) != Some("eDP-1"))
    );
    assert_eq!(
        session.images().dimensions(shared),
        None,
        "unplugging eDP-1 must release its wallpaper texture ref"
    );
    assert_eq!(
        session.images().dimensions(dp_new),
        Some((8, 8)),
        "remaining output DP-1 must keep its wallpaper texture"
    );
}

#[test]
fn bright_backdrop_toggles_adaptive_mica_alpha_on_pinned_bar() {
    let mut shell = TritonShell::new(BackendMode::Fake);
    let mut session = HeadlessSession::new((1920.0, 1080.0), 1.0);
    let (tx, rx) = event_channel::<ShellEvent>();
    shell.set_event_sender(tx.clone());

    tx.send(ShellEvent::SetBarMode(triton::BarMode::Pinned))
        .unwrap();
    tx.send(ShellEvent::SetWallpaper {
        path: PathBuf::from("/tmp/dark.png"),
        output: None,
    })
    .unwrap();
    let _ = session.drain_events(&mut shell, &rx, TritonShell::handle_event);

    // Dark wallpaper: below threshold, no adaptive boost.
    let _ = deliver_ready(
        &mut shell,
        &mut session,
        wallpaper("/tmp/dark.png", [12, 12, 14, 255]),
    );
    assert!(!shell.state().wallpaper.is_bright_backdrop());
    let dark_alpha = pinned_bar_fill_alpha(&mut shell, &mut session);

    // Bright wallpaper: above the 0.6 threshold.
    tx.send(ShellEvent::SetWallpaper {
        path: PathBuf::from("/tmp/bright.png"),
        output: None,
    })
    .unwrap();
    let _ = session.drain_events(&mut shell, &rx, TritonShell::handle_event);
    let _ = deliver_ready(
        &mut shell,
        &mut session,
        wallpaper("/tmp/bright.png", [245, 245, 240, 255]),
    );

    assert!(shell.state().wallpaper.is_bright_backdrop());
    assert!(shell.state().wallpaper.mean_luminance > BRIGHT_BACKDROP_THRESHOLD);
    let bright_alpha = pinned_bar_fill_alpha(&mut shell, &mut session);
    assert!(
        bright_alpha > dark_alpha + 0.05,
        "adaptive Mica raises fill alpha ({dark_alpha} -> {bright_alpha})"
    );

    // Status snapshot exposes the luminance state to `triton msg status`.
    let snapshot = shell.status_snapshot();
    assert!(snapshot.bright_backdrop);
    assert!(snapshot.mean_luminance > BRIGHT_BACKDROP_THRESHOLD);
}

/// Render the pinned bar surface and return its root Mica panel fill alpha.
fn pinned_bar_fill_alpha(
    shell: &mut TritonShell,
    session: &mut HeadlessSession<ShellMessage>,
) -> f32 {
    let (bar_id, sizing) = shell
        .surfaces()
        .iter()
        .find_map(|(id, spec)| (id.role == SurfaceRole::Bar).then_some((*id, spec.sizing)))
        .expect("pinned bar mounted");
    session.render_surface(shell, bar_id, sizing);
    session
        .draw_list()
        .batches
        .iter()
        .find_map(|b| match b {
            triton_engine::gpu::DrawBatch::Quads { instances, .. } => {
                instances.first().map(|q| q.fill[3])
            }
            _ => None,
        })
        .expect("bar root panel quad")
}

#[test]
fn off_thread_decode_delivers_image_and_failures_through_event_channel() {
    let dir = std::env::temp_dir().join(format!("triton-wp-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let png_path = dir.join("fixture.png");
    let (w, h) = (4u32, 4u32);
    let white_px = [255, 255, 255, 255].repeat((w * h) as usize);
    write_png(&png_path, w, h, &white_px).unwrap();

    let mut shell = TritonShell::new(BackendMode::Fake);
    let mut session = HeadlessSession::new((1920.0, 1080.0), 1.0);
    let (tx, rx) = event_channel::<ShellEvent>();
    shell.set_event_sender(tx.clone());

    // The daemon path: SetWallpaper spawns the decode worker itself.
    tx.send(ShellEvent::SetWallpaper {
        path: png_path.clone(),
        output: None,
    })
    .unwrap();
    let _ = session.drain_events(&mut shell, &rx, TritonShell::handle_event);

    // Wait for the worker by draining until the event lands (deadline-bound).
    let decoded = drain_until(&mut shell, &mut session, &rx, |shell| {
        shell.state().wallpaper.active_image.is_some()
    });
    assert!(decoded, "off-thread decode delivered WallpaperReady");
    let key = key_of(&shell, "eDP-1");
    assert!(shell.wallpapers().current_for(key).is_some());
    assert!(shell.state().wallpaper.is_bright_backdrop());

    // Missing file reports WallpaperFailed without panicking or blanking.
    tx.send(ShellEvent::SetWallpaper {
        path: dir.join("does-not-exist.png"),
        output: None,
    })
    .unwrap();
    let _ = session.drain_events(&mut shell, &rx, TritonShell::handle_event);
    // While the decode is in flight the path is claimed; once `WallpaperFailed`
    // lands it returns to `undecoded_desires` with the old image still shown.
    let failed = drain_until(&mut shell, &mut session, &rx, |shell| {
        shell
            .wallpapers()
            .undecoded_desires()
            .iter()
            .any(|p| p.ends_with("does-not-exist.png"))
    });
    assert!(failed, "decode failure reported off-thread");
    // The previously displayed wallpaper still shows.
    assert!(shell.wallpapers().current_for(key).is_some());

    let _ = std::fs::remove_dir_all(&dir);
}

/// Drain events (feeding each into `handle_event` + `sync_images`) until
/// `done` holds or a generous deadline passes. No sleeping — the worker
/// thread runs concurrently and `yield_now` keeps the wait CPU-light.
fn drain_until(
    shell: &mut TritonShell,
    session: &mut HeadlessSession<ShellMessage>,
    rx: &triton_engine::EventReceiver<ShellEvent>,
    done: impl Fn(&TritonShell) -> bool,
) -> bool {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        while let Some(ev) = rx.try_recv() {
            let _ = shell.handle_event(ev, session.now());
        }
        session.sync_app_images(shell);
        if done(shell) {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::yield_now();
    }
}
