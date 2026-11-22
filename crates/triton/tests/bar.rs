//! Behavior tests for Triton's Bar (`Autohide`, `Pinned`, `Hidden`,
//! workspaces, edge-faded focused title, minute-aligned clock, status pill,
//! notification bell, and `reduce_motion`) driven through `HeadlessSession`.

use std::time::Duration;

use triton::bar::{
    BAR_HIDE_DELAY, BAR_REVEAL_DELAY_DEFAULT, BAR_REVEAL_DELAY_FULLSCREEN, BarClock,
    TIMER_BAR_CLOCK, bar_hide_timer_tag, bar_reveal_timer_tag,
};
use triton::{
    BackendMode, BarMode, FocusedWindow, ShellEvent, ShellMessage, StatusSummary, SurfaceRole,
    TritonShell, WorkspaceInfo,
};
use triton_design::color::{Scheme, Variant};
use triton_design::material::Material;
use triton_engine::gpu::DrawBatch;
use triton_engine::{
    AppCommand, HeadlessSession, OutputInfo, ShellApp, SurfaceSize, event_channel,
};

#[test]
fn autohide_enforces_reveal_fullscreen_and_hide_delays_with_2px_edge() {
    let mut shell = TritonShell::new(BackendMode::Fake);
    let mut session = HeadlessSession::<ShellMessage>::new((1920.0, 1080.0), 1.0);

    let (bar_id, bar_spec) = shell
        .surfaces()
        .into_iter()
        .find(|(id, _)| id.role == SurfaceRole::Bar)
        .expect("bar mounted in Autohide");

    // At rest (retracted): zero blur rects, and input_rects contains only the 2px top reveal edge.
    session.render_surface(&mut shell, bar_id, bar_spec.sizing);
    assert!(
        session.frame().blur_rects.is_empty(),
        "retracted autohide bar must not emit blur rects"
    );
    assert_eq!(
        session.frame().input_rects.len(),
        1,
        "retracted autohide bar should expose only the 2px reveal edge"
    );
    let edge = session.frame().input_rects[0].rect;
    assert!((edge.y - 0.0).abs() < 0.5);
    assert!((edge.h - 2.0).abs() < 0.5);
    assert!((edge.w - 1920.0).abs() < 1.0);
    assert!(!shell.is_animating(bar_id, session.now()));

    let reveal_tag = bar_reveal_timer_tag(bar_id.output);
    let hide_tag = bar_hide_timer_tag(bar_id.output);

    // Entering the reveal edge schedules the 120ms normal reveal timer.
    let cmds = session.pointer_enter(&mut shell, bar_id, (960.0, 1.0));
    assert!(cmds.contains(&AppCommand::ScheduleTimer {
        tag: reveal_tag,
        delay: BAR_REVEAL_DELAY_DEFAULT,
    }));
    assert_eq!(BAR_REVEAL_DELAY_DEFAULT, Duration::from_millis(120));

    // Leaving before 120ms cancels the reveal timer and keeps the bar retracted.
    session.advance(Duration::from_millis(60));
    let cmds = session.pointer_leave(&mut shell, bar_id);
    assert!(cmds.contains(&AppCommand::CancelTimer(reveal_tag)));
    session.render_surface(&mut shell, bar_id, bar_spec.sizing);
    assert_eq!(session.frame().blur_rects.len(), 0);

    // Re-entering and letting 120ms elapse reveals the floating 56px Glass islands at y = 8px.
    let cmds = session.pointer_enter(&mut shell, bar_id, (960.0, 1.0));
    assert!(cmds.contains(&AppCommand::ScheduleTimer {
        tag: reveal_tag,
        delay: Duration::from_millis(120),
    }));
    session.advance(Duration::from_millis(120));
    let cmds = shell.on_timer(reveal_tag, session.now());
    assert!(cmds.contains(&AppCommand::Redraw(bar_id)));
    assert!(shell.is_animating(bar_id, session.now()));

    // Let the spatial_default spring settle.
    session.advance(Duration::from_millis(900));
    assert!(!shell.is_animating(bar_id, session.now()));
    session.render_surface(&mut shell, bar_id, bar_spec.sizing);
    assert_eq!(
        session.frame().blur_rects.len(),
        2,
        "revealed autohide bar must mount exactly two floating Glass islands"
    );
    assert!(session.hit_rect("bar-island-left-eDP-1").is_some());
    assert!(session.hit_rect("bar-island-right-eDP-1").is_some());
    assert!(session.hit_rect("bar-island-center-eDP-1").is_none());
    assert!(
        session.hit_rect("bar-title-eDP-1").is_some(),
        "focused window title should live inside the left Glass island"
    );
    for br in &session.frame().blur_rects {
        assert!(
            (br.rect.y - 8.0).abs() < 1.0,
            "floating island should be offset 8px from top edge, got y={}",
            br.rect.y
        );
        assert!(
            (br.rect.h - 56.0).abs() < 1.0,
            "floating island should be 56px tall, got h={}",
            br.rect.h
        );
    }

    // Leaving schedules the 600ms hide timer; re-entering before 600ms cancels it.
    let cmds = session.pointer_leave(&mut shell, bar_id);
    assert!(cmds.contains(&AppCommand::ScheduleTimer {
        tag: hide_tag,
        delay: BAR_HIDE_DELAY,
    }));
    assert_eq!(BAR_HIDE_DELAY, Duration::from_millis(600));

    session.advance(Duration::from_millis(300));
    let cmds = session.pointer_enter(&mut shell, bar_id, (200.0, 30.0));
    assert!(cmds.contains(&AppCommand::CancelTimer(hide_tag)));

    // Leaving and firing the 600ms hide timer retracts the islands cleanly.
    let _ = session.pointer_leave(&mut shell, bar_id);
    session.advance(Duration::from_millis(600));
    let cmds = shell.on_timer(hide_tag, session.now());
    assert!(cmds.contains(&AppCommand::Redraw(bar_id)));
    assert!(shell.is_animating(bar_id, session.now()));

    session.advance(Duration::from_millis(900));
    assert!(!shell.is_animating(bar_id, session.now()));
    session.render_surface(&mut shell, bar_id, bar_spec.sizing);
    assert_eq!(session.frame().blur_rects.len(), 0);
    assert_eq!(session.frame().input_rects.len(), 1);
}

#[test]
fn autohide_uses_300ms_delay_for_fullscreen_and_honors_configurable_normal_delay() {
    let mut shell = TritonShell::new(BackendMode::Fake);
    let mut session = HeadlessSession::<ShellMessage>::new((1920.0, 1080.0), 1.0);
    let (tx, rx) = event_channel::<ShellEvent>();

    let (bar_id, _) = shell
        .surfaces()
        .into_iter()
        .find(|(id, _)| id.role == SurfaceRole::Bar)
        .expect("bar mounted in Autohide");
    let reveal_tag = bar_reveal_timer_tag(bar_id.output);

    // Fullscreen window on this output extends reveal delay to 300ms.
    tx.send(ShellEvent::FocusedWindowChanged(Some(FocusedWindow {
        id: 42,
        title: "Full Screen Game".to_owned(),
        app_id: Some("game".to_owned()),
        is_fullscreen: true,
        output: Some("eDP-1".to_owned()),
    })))
    .unwrap();
    let _ = session.drain_events(&mut shell, &rx, TritonShell::handle_event);

    let cmds = session.pointer_enter(&mut shell, bar_id, (960.0, 1.0));
    assert!(cmds.contains(&AppCommand::ScheduleTimer {
        tag: reveal_tag,
        delay: BAR_REVEAL_DELAY_FULLSCREEN,
    }));
    assert_eq!(BAR_REVEAL_DELAY_FULLSCREEN, Duration::from_millis(300));

    // Configurable normal reveal delay applies when not fullscreen.
    shell.set_bar_reveal_delay(Duration::from_millis(200));
    tx.send(ShellEvent::FocusedWindowChanged(None)).unwrap();
    let _ = session.drain_events(&mut shell, &rx, TritonShell::handle_event);
    let _ = session.pointer_leave(&mut shell, bar_id);
    let cmds = session.pointer_enter(&mut shell, bar_id, (960.0, 1.0));
    assert!(cmds.contains(&AppCommand::ScheduleTimer {
        tag: reveal_tag,
        delay: Duration::from_millis(200),
    }));
}

#[test]
fn pinned_mode_renders_56px_mica_strip_with_adaptive_bright_backdrop_and_hidden_unmounts() {
    let mut shell = TritonShell::new(BackendMode::Fake);
    let mut session = HeadlessSession::<ShellMessage>::new((1920.0, 1080.0), 1.0);
    let (tx, rx) = event_channel::<ShellEvent>();

    tx.send(ShellEvent::SetBarMode(BarMode::Pinned)).unwrap();
    let _ = session.drain_events(&mut shell, &rx, TritonShell::handle_event);

    let (bar_id, bar_spec) = shell
        .surfaces()
        .into_iter()
        .find(|(id, _)| id.role == SurfaceRole::Bar)
        .expect("bar mounted in Pinned");
    assert_eq!(bar_spec.exclusive_zone, 56);
    assert_eq!(bar_spec.sizing, SurfaceSize::Fixed(1920, 56));
    assert!(!bar_spec.shaped_input);

    // Base Dark Mica fill (luminance <= 0.6).
    session.render_surface(&mut shell, bar_id, bar_spec.sizing);
    assert_eq!(session.frame().blur_rects.len(), 1);
    let blur = session.frame().blur_rects[0].rect;
    assert!((blur.w - 1920.0).abs() < 1.0);
    assert!((blur.h - 56.0).abs() < 1.0);

    let base_mica = Material::Mica.fill_adaptive(&Scheme::DARK, Variant::Dark, false);
    let first_quad_alpha = first_bar_strip_fill_alpha(&session);
    assert!(
        (first_quad_alpha - base_mica.a).abs() < 0.01,
        "expected base dark mica alpha {}, got {first_quad_alpha}",
        base_mica.a
    );

    // Bright wallpaper (mean_luminance > 0.6) applies adaptive +0.08 alpha in Dark.
    tx.send(ShellEvent::ApplyTheme {
        scheme: None,
        variant: None,
        mean_luminance: Some(0.75),
    })
    .unwrap();
    let _ = session.drain_events(&mut shell, &rx, TritonShell::handle_event);
    session.render_surface(&mut shell, bar_id, bar_spec.sizing);
    let bright_dark_mica = Material::Mica.fill_adaptive(&Scheme::DARK, Variant::Dark, true);
    let bright_dark_alpha = first_bar_strip_fill_alpha(&session);
    assert!(
        (bright_dark_alpha - bright_dark_mica.a).abs() < 0.01,
        "expected bright dark mica alpha {}, got {bright_dark_alpha}",
        bright_dark_mica.a
    );
    assert!(bright_dark_alpha > first_quad_alpha + 0.05);

    // Light variant with bright wallpaper applies adaptive +0.06 alpha.
    tx.send(ShellEvent::ApplyTheme {
        scheme: Some(Box::new(Scheme::LIGHT)),
        variant: Some(Variant::Light),
        mean_luminance: Some(0.8),
    })
    .unwrap();
    let _ = session.drain_events(&mut shell, &rx, TritonShell::handle_event);
    session.render_surface(&mut shell, bar_id, bar_spec.sizing);
    let bright_light_mica = Material::Mica.fill_adaptive(&Scheme::LIGHT, Variant::Light, true);
    let bright_light_alpha = first_bar_strip_fill_alpha(&session);
    assert!(
        (bright_light_alpha - bright_light_mica.a).abs() < 0.01,
        "expected bright light mica alpha {}, got {bright_light_alpha}",
        bright_light_mica.a
    );

    // Hidden mode cleanly unmounts the Bar surface; ToggleBarHidden restores Pinned.
    tx.send(ShellEvent::SetBarMode(BarMode::Hidden)).unwrap();
    let _ = session.drain_events(&mut shell, &rx, TritonShell::handle_event);
    assert!(
        !shell
            .surfaces()
            .iter()
            .any(|(id, _)| id.role == SurfaceRole::Bar)
    );

    tx.send(ShellEvent::ToggleBarHidden).unwrap();
    let _ = session.drain_events(&mut shell, &rx, TritonShell::handle_event);
    assert_eq!(shell.state().bar_mode, BarMode::Pinned);
    assert!(
        shell
            .surfaces()
            .iter()
            .any(|(id, _)| id.role == SurfaceRole::Bar)
    );
}

fn sample_workspaces() -> Vec<WorkspaceInfo> {
    vec![
        WorkspaceInfo {
            id: 1,
            idx: 1,
            name: None,
            output: "eDP-1".to_owned(),
            is_active: true,
            is_focused: true,
            is_urgent: false,
            active_window_id: Some(10),
        },
        WorkspaceInfo {
            id: 2,
            idx: 2,
            name: None,
            output: "eDP-1".to_owned(),
            is_active: false,
            is_focused: false,
            is_urgent: false,
            active_window_id: Some(20),
        },
        WorkspaceInfo {
            id: 3,
            idx: 3,
            name: None,
            output: "eDP-1".to_owned(),
            is_active: false,
            is_focused: false,
            is_urgent: true,
            active_window_id: Some(30),
        },
        WorkspaceInfo {
            id: 4,
            idx: 4,
            name: None,
            output: "eDP-1".to_owned(),
            is_active: false,
            is_focused: false,
            is_urgent: false,
            active_window_id: None,
        },
        WorkspaceInfo {
            id: 99,
            idx: 1,
            name: Some("other".to_owned()),
            output: "DP-1".to_owned(),
            is_active: true,
            is_focused: false,
            is_urgent: false,
            active_window_id: None,
        },
    ]
}

#[test]
fn workspaces_and_edge_faded_title_compose_kit_and_update_live() {
    let mut shell = TritonShell::new(BackendMode::Fake);
    let mut session = HeadlessSession::<ShellMessage>::new((1920.0, 1080.0), 1.0);
    let (tx, rx) = event_channel::<ShellEvent>();

    tx.send(ShellEvent::SetBarMode(BarMode::Pinned)).unwrap();
    tx.send(ShellEvent::WorkspacesUpdated(sample_workspaces()))
        .unwrap();
    tx.send(ShellEvent::FocusedWindowChanged(Some(FocusedWindow {
        id: 10,
        title: "A Very Long Focused Window Title That Exceeds The Bar Width Budget And Edge Fades"
            .to_owned(),
        app_id: Some("editor".to_owned()),
        is_fullscreen: false,
        output: Some("eDP-1".to_owned()),
    })))
    .unwrap();
    let _ = session.drain_events(&mut shell, &rx, TritonShell::handle_event);

    let (bar_id, bar_spec) = shell
        .surfaces()
        .into_iter()
        .find(|(id, _)| id.role == SurfaceRole::Bar)
        .expect("pinned bar mounted");

    session.render_surface(&mut shell, bar_id, bar_spec.sizing);

    // Per-output workspaces: eDP-1 pills present, DP-1 workspace excluded.
    assert!(session.hit_rect("bar-ws-eDP-1-1").is_some());
    assert!(session.hit_rect("bar-ws-eDP-1-2").is_some());
    assert!(session.hit_rect("bar-ws-eDP-1-3").is_some());
    assert!(session.hit_rect("bar-ws-eDP-1-4").is_some());
    assert!(session.hit_rect("bar-ws-eDP-1-99").is_none());

    // Active workspace pill is 56px wide; inactive is 40px wide.
    let ws1 = session.hit_rect("bar-ws-eDP-1-1").unwrap();
    let ws2 = session.hit_rect("bar-ws-eDP-1-2").unwrap();
    assert!(
        (ws1.w - 56.0).abs() < 1.0,
        "active pill should be 56px, got {}",
        ws1.w
    );
    assert!(
        (ws2.w - 40.0).abs() < 1.0,
        "inactive pill should be 40px, got {}",
        ws2.w
    );

    // Focused title is present and emits an edge-faded glyph batch (`[left, right, top, bottom]`).
    assert!(session.hit_rect("bar-title-eDP-1").is_some());
    let has_faded_glyphs = session.draw_list().batches.iter().any(|b| {
        matches!(
            b,
            DrawBatch::Glyphs {
                edge_fade: Some(ef),
                ..
            } if ef.widths[0] > 0.0 && ef.widths[1] > 0.0
        )
    });
    assert!(
        has_faded_glyphs,
        "focused window title should emit edge-faded glyphs"
    );

    // Clicking inactive workspace 2 emits FocusWorkspace(2) and updates active state.
    let msgs = session.tap_key("bar-ws-eDP-1-2").expect("click ws 2");
    assert_eq!(msgs, vec![ShellMessage::FocusWorkspace(2)]);
    for msg in msgs {
        let _ = shell.update(bar_id, msg, session.now());
    }
    assert!(
        shell
            .state()
            .workspaces
            .iter()
            .any(|w| w.id == 2 && w.is_active)
    );
    assert!(
        shell
            .state()
            .workspaces
            .iter()
            .any(|w| w.id == 1 && !w.is_active)
    );
}

#[test]
fn clock_status_pill_and_notification_bell_align_to_minutes_and_update_live() {
    let mut shell = TritonShell::new(BackendMode::Fake);
    let mut session = HeadlessSession::<ShellMessage>::new((1920.0, 1080.0), 1.0);
    let (tx, rx) = event_channel::<ShellEvent>();

    // Anchor clock to 2026-01-15 14:32:17.250 UTC so delay to next minute is exact 42.750s.
    let anchor_unix = 1_768_487_537_u64;
    shell.set_bar_clock(BarClock::fixed(anchor_unix, 250_000_000, 0, session.now()));

    let start_cmds = shell.on_start(session.now());
    assert!(start_cmds.contains(&AppCommand::ScheduleTimer {
        tag: TIMER_BAR_CLOCK,
        delay: Duration::from_millis(42_750),
    }));

    tx.send(ShellEvent::SetBarMode(BarMode::Pinned)).unwrap();
    tx.send(ShellEvent::StatusUpdated(StatusSummary {
        network_connected: true,
        volume_percent: 72,
        muted: false,
        battery_percent: Some(88),
        charging: false,
        unread_notifications: 5,
    }))
    .unwrap();
    let _ = session.drain_events(&mut shell, &rx, TritonShell::handle_event);

    let (bar_id, bar_spec) = shell
        .surfaces()
        .into_iter()
        .find(|(id, _)| id.role == SurfaceRole::Bar)
        .expect("pinned bar mounted");

    session.render_surface(&mut shell, bar_id, bar_spec.sizing);
    let tree = session.tree_json();

    // Clock renders "14:32" and is idle (0% CPU) between minute ticks.
    assert!(tree.contains("14:32"), "expected clock 14:32 in tree_json");
    assert!(!shell.is_animating(bar_id, session.now()));

    // Unread notification badge renders "5" on the bell.
    assert!(session.hit_rect("bar-bell-eDP-1").is_some());
    assert!(session.hit_rect("bar-status-eDP-1").is_some());
    assert!(
        tree.contains("\"5\""),
        "expected unread count 5 in tree_json"
    );

    // Clicking status pill and notification bell emits their typed messages.
    assert_eq!(
        session.tap_key("bar-status-eDP-1").expect("click status"),
        vec![ShellMessage::StatusPillClicked]
    );
    assert_eq!(
        session.tap_key("bar-bell-eDP-1").expect("click bell"),
        vec![ShellMessage::NotificationBellClicked]
    );

    // Advance virtual clock to the minute boundary and fire TIMER_BAR_CLOCK.
    session.advance(Duration::from_millis(42_750));
    let timer_cmds = shell.on_timer(TIMER_BAR_CLOCK, session.now());
    assert!(timer_cmds.contains(&AppCommand::ScheduleTimer {
        tag: TIMER_BAR_CLOCK,
        delay: Duration::from_secs(60),
    }));
    assert!(timer_cmds.contains(&AppCommand::RedrawAll));
    session.render_surface(&mut shell, bar_id, bar_spec.sizing);
    assert!(
        session.tree_json().contains("14:33"),
        "expected clock to advance to 14:33"
    );

    // Unread count > 99 clamps to "99+", and low battery updates live.
    tx.send(ShellEvent::StatusUpdated(StatusSummary {
        network_connected: true,
        volume_percent: 50,
        muted: false,
        battery_percent: Some(10),
        charging: false,
        unread_notifications: 120,
    }))
    .unwrap();
    let _ = session.drain_events(&mut shell, &rx, TritonShell::handle_event);
    session.render_surface(&mut shell, bar_id, bar_spec.sizing);
    let updated = session.tree_json();
    assert!(
        updated.contains("99+"),
        "expected unread badge >99 to clamp to 99+"
    );
    assert!(
        updated.contains("10%"),
        "expected battery percentage to update to 10%"
    );
}

#[test]
fn reduce_motion_swaps_bar_reveal_and_workspace_pill_springs_for_120ms_fades() {
    let mut shell = TritonShell::new(BackendMode::Fake);
    let mut session = HeadlessSession::<ShellMessage>::new((1920.0, 1080.0), 1.0);
    let (tx, rx) = event_channel::<ShellEvent>();

    // Enable reduce_motion on both shell and headless session animator.
    tx.send(ShellEvent::SetReduceMotion(true)).unwrap();
    let cmds = session.drain_events(&mut shell, &rx, TritonShell::handle_event);
    assert!(cmds.contains(&AppCommand::SetReduceMotion(true)));
    session.set_reduce_motion(true);

    let (bar_id, bar_spec) = shell
        .surfaces()
        .into_iter()
        .find(|(id, _)| id.role == SurfaceRole::Bar)
        .expect("autohide bar mounted");

    // Trigger reveal in Autohide mode.
    let reveal_tag = bar_reveal_timer_tag(bar_id.output);
    let _ = session.pointer_enter(&mut shell, bar_id, (960.0, 1.0));
    session.advance(BAR_REVEAL_DELAY_DEFAULT);
    let _ = shell.on_timer(reveal_tag, session.now());
    assert!(shell.is_animating(bar_id, session.now()));

    // Mid-fade at 60ms: islands are already at their final y=8px (no spatial slide)
    // and opacity is ~0.5 (linear 120ms fade).
    session.advance(Duration::from_millis(60));
    session.render_surface(&mut shell, bar_id, bar_spec.sizing);
    let mid_left = session
        .hit_rect("bar-island-left-eDP-1")
        .expect("left island visible mid-fade");
    assert!(
        (mid_left.y - 8.0).abs() < 1.0,
        "reduce_motion must not slide islands vertically; got y={}",
        mid_left.y
    );

    // At 120ms (total fade duration): bar reveal is completely settled.
    session.advance(Duration::from_millis(60));
    assert!(
        !shell.is_animating(bar_id, session.now()),
        "reduce_motion bar reveal must settle by 120ms"
    );

    // Workspace pill switch under reduce_motion settles cleanly within 120ms.
    session.render_surface(&mut shell, bar_id, bar_spec.sizing);
    let msgs = session.tap_key("bar-ws-eDP-1-2").expect("click ws 2");
    for msg in msgs {
        let _ = shell.update(bar_id, msg, session.now());
    }
    session.render_surface(&mut shell, bar_id, bar_spec.sizing);
    session.advance(Duration::from_millis(120));
    session.render_surface(&mut shell, bar_id, bar_spec.sizing);
    let ws2 = session.hit_rect("bar-ws-eDP-1-2").unwrap();
    assert!(
        (ws2.w - 56.0).abs() < 0.5,
        "workspace pill under reduce_motion should reach 56px by 120ms, got {}",
        ws2.w
    );
}

#[test]
fn multi_output_autohide_isolates_reveal_timers_per_output() {
    let mut shell = TritonShell::new(BackendMode::RealServices);
    let mut session = HeadlessSession::<ShellMessage>::new((1920.0, 1080.0), 1.0);

    let _ = session.set_outputs(
        &mut shell,
        vec![
            OutputInfo::new("eDP-1", (1920, 1080)),
            OutputInfo::new("DP-1", (2560, 1440)),
        ],
    );

    let bars: Vec<_> = shell
        .surfaces()
        .into_iter()
        .filter(|(id, _)| id.role == SurfaceRole::Bar)
        .collect();
    assert_eq!(bars.len(), 2);
    let (bar_a, spec_a) = &bars[0];
    let (bar_b, spec_b) = &bars[1];
    assert_ne!(
        bar_reveal_timer_tag(bar_a.output),
        bar_reveal_timer_tag(bar_b.output)
    );

    // Reveal only bar_a; bar_b must remain retracted.
    let _ = session.pointer_enter(&mut shell, *bar_a, (100.0, 1.0));
    session.advance(BAR_REVEAL_DELAY_DEFAULT);
    let _ = shell.on_timer(bar_reveal_timer_tag(bar_a.output), session.now());
    session.advance(Duration::from_millis(900));

    session.render_surface(&mut shell, *bar_a, spec_a.sizing);
    assert_eq!(session.frame().blur_rects.len(), 2);

    session.render_surface(&mut shell, *bar_b, spec_b.sizing);
    assert_eq!(session.frame().blur_rects.len(), 0);
}

fn first_bar_strip_fill_alpha(session: &HeadlessSession<ShellMessage>) -> f32 {
    for batch in &session.draw_list().batches {
        if let DrawBatch::Quads { instances, .. } = batch {
            for q in instances {
                if q.dst[2] >= 1900.0 && (q.dst[3] - 56.0).abs() < 1.0 {
                    return q.fill[3];
                }
            }
        }
    }
    panic!("expected 1920x56 pinned bar strip quad in DrawList");
}
