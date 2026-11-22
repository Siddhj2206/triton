//! Integration tests for `TritonShell` declarative surface reconciliation
//! driven through `triton_engine::HeadlessSession`.

use std::collections::BTreeSet;
use std::time::Duration;

use triton::{
    BackendMode, BarMode, OsdPayload, ShellEvent, ShellMessage, SurfaceRole, TIMER_OSD_HIDE,
    TritonShell,
};
use triton_engine::{
    AppCommand, HeadlessSession, OutputInfo, ShellApp, SurfaceKeyboard, SurfaceLayer, SurfaceSize,
    event_channel,
};

fn role_output_set(shell: &TritonShell) -> BTreeSet<(SurfaceRole, String)> {
    shell
        .surfaces()
        .into_iter()
        .map(|(id, spec)| {
            let (role, name) = shell
                .surface_descriptor(id)
                .expect("every surface id resolves to (role, output)");
            assert_eq!(spec.output.as_deref(), Some(name));
            assert_eq!(spec.namespace, role.namespace());
            (role, name.to_owned())
        })
        .collect()
}

#[test]
fn reconciles_surfaces_across_multi_output_hotplug_and_removal() {
    let mut shell = TritonShell::new(BackendMode::Fake);
    let mut session = HeadlessSession::<ShellMessage>::new((1920.0, 1080.0), 1.0);

    // Fake dev backend starts with synthetic eDP-1 (Background + Bar, no OSD).
    let initial = role_output_set(&shell);
    assert_eq!(
        initial,
        BTreeSet::from([
            (SurfaceRole::Background, "eDP-1".to_owned()),
            (SurfaceRole::Bar, "eDP-1".to_owned()),
        ])
    );

    // Initialize HeadlessSession with eDP-1, then hotplug DP-1.
    let _ = session.set_outputs(&mut shell, vec![OutputInfo::new("eDP-1", (1920, 1080))]);
    let cmds = session.add_output(&mut shell, OutputInfo::new("DP-1", (2560, 1440)));
    assert!(cmds.contains(&AppCommand::RedrawAll));

    let dual = role_output_set(&shell);
    assert_eq!(
        dual,
        BTreeSet::from([
            (SurfaceRole::Background, "eDP-1".to_owned()),
            (SurfaceRole::Bar, "eDP-1".to_owned()),
            (SurfaceRole::Background, "DP-1".to_owned()),
            (SurfaceRole::Bar, "DP-1".to_owned()),
        ])
    );

    // Unplug eDP-1: its surfaces disappear and focused_output falls back to DP-1.
    let cmds = session.remove_output(&mut shell, "eDP-1");
    assert!(cmds.contains(&AppCommand::RedrawAll));
    assert_eq!(shell.state().focused_output.as_deref(), Some("DP-1"));
    assert_eq!(
        role_output_set(&shell),
        BTreeSet::from([
            (SurfaceRole::Background, "DP-1".to_owned()),
            (SurfaceRole::Bar, "DP-1".to_owned()),
        ])
    );
}

#[test]
fn bar_mode_transitions_reconcile_autohide_pinned_and_hidden() {
    let mut shell = TritonShell::new(BackendMode::Fake);
    let mut session = HeadlessSession::new((1920.0, 1080.0), 1.0);
    let (tx, rx) = event_channel::<ShellEvent>();

    // Default mode is Autohide: floating StableExtent with shaped input, no exclusive zone.
    let autohide_bar = shell
        .surfaces()
        .into_iter()
        .find(|(id, _)| id.role == SurfaceRole::Bar)
        .expect("bar mounted in Autohide");
    assert_eq!(autohide_bar.1.layer, SurfaceLayer::Top);
    assert_eq!(autohide_bar.1.exclusive_zone, 0);
    assert!(autohide_bar.1.shaped_input);
    assert!(matches!(
        autohide_bar.1.sizing,
        SurfaceSize::StableExtent { .. }
    ));

    // Render Autohide bar through HeadlessSession.
    session.render_surface(&mut shell, autohide_bar.0, autohide_bar.1.sizing);
    assert!(!session.draw_list().batches.is_empty());

    // Switch to Pinned: 56px Mica strip with exclusive_zone = 56.
    tx.send(ShellEvent::SetBarMode(BarMode::Pinned)).unwrap();
    let cmds = session.drain_events(&mut shell, &rx, TritonShell::handle_event);
    assert!(cmds.contains(&AppCommand::RedrawAll));
    let pinned_bar = shell
        .surfaces()
        .into_iter()
        .find(|(id, _)| id.role == SurfaceRole::Bar)
        .expect("bar mounted in Pinned");
    assert_eq!(pinned_bar.1.exclusive_zone, 56);
    assert!(!pinned_bar.1.shaped_input);
    assert_eq!(pinned_bar.1.sizing, SurfaceSize::Fixed(1920, 56));

    session.render_surface(&mut shell, pinned_bar.0, pinned_bar.1.sizing);
    assert!(!session.draw_list().batches.is_empty());

    // Switch to Hidden: Bar unmounts while Background remains.
    tx.send(ShellEvent::SetBarMode(BarMode::Hidden)).unwrap();
    let _ = session.drain_events(&mut shell, &rx, TritonShell::handle_event);
    assert_eq!(
        role_output_set(&shell),
        BTreeSet::from([(SurfaceRole::Background, "eDP-1".to_owned())])
    );

    // ToggleBarHidden restores the last visible mode (Pinned).
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

#[test]
fn osd_mounts_only_on_focused_output_and_unmounts_after_hold_timer() {
    let mut shell = TritonShell::new(BackendMode::RealServices);
    let mut session = HeadlessSession::new((1920.0, 1080.0), 1.0);
    let (tx, rx) = event_channel::<ShellEvent>();

    let _ = session.set_outputs(
        &mut shell,
        vec![
            OutputInfo::new("eDP-1", (1920, 1080)),
            OutputInfo::new("DP-1", (2560, 1440)),
        ],
    );
    tx.send(ShellEvent::FocusedOutputChanged(Some("DP-1".to_owned())))
        .unwrap();
    tx.send(ShellEvent::ShowOsd {
        payload: OsdPayload::Volume {
            level: 0.75,
            muted: false,
        },
        output: None,
    })
    .unwrap();

    let cmds = session.drain_events(&mut shell, &rx, TritonShell::handle_event);
    assert!(cmds.contains(&AppCommand::ScheduleTimer {
        tag: TIMER_OSD_HIDE,
        delay: Duration::from_millis(1200),
    }));

    let osd_surfaces: Vec<_> = shell
        .surfaces()
        .into_iter()
        .filter(|(id, _)| id.role == SurfaceRole::Osd)
        .collect();
    assert_eq!(osd_surfaces.len(), 1);
    let (osd_id, osd_spec) = &osd_surfaces[0];
    assert_eq!(osd_spec.output.as_deref(), Some("DP-1"));
    assert_eq!(osd_spec.layer, SurfaceLayer::Overlay);
    assert_eq!(osd_spec.keyboard, SurfaceKeyboard::None);
    assert!(osd_spec.shaped_input);

    // Render OSD through HeadlessSession.
    session.render_surface(&mut shell, *osd_id, osd_spec.sizing);
    assert!(!session.draw_list().batches.is_empty());

    // Advance virtual clock and fire hold timer -> OSD unmounts cleanly.
    session.advance(Duration::from_millis(1200));
    let timer_cmds = shell.on_timer(TIMER_OSD_HIDE, session.now());
    assert!(timer_cmds.contains(&AppCommand::RedrawAll));
    assert!(
        !shell
            .surfaces()
            .iter()
            .any(|(id, _)| id.role == SurfaceRole::Osd)
    );
}

#[test]
fn fake_dev_backend_has_synthetic_state_while_real_services_starts_clean() {
    let fake = TritonShell::new(BackendMode::Fake);
    assert_eq!(fake.state().backend, BackendMode::Fake);
    assert_eq!(fake.state().outputs.len(), 1);
    assert_eq!(fake.state().workspaces.len(), 4);
    assert!(fake.state().focused_window.is_some());
    assert_eq!(fake.surfaces().len(), 2);

    let real = TritonShell::new(BackendMode::RealServices);
    assert_eq!(real.state().backend, BackendMode::RealServices);
    assert_eq!(real.state().outputs.len(), 0);
    assert_eq!(real.state().workspaces.len(), 0);
    assert!(real.state().focused_window.is_none());
    assert_eq!(real.surfaces().len(), 0);
}
