//! Integration tests for the morph-in-place Glass OSD (#17) driven through
//! `triton_engine::HeadlessSession` at the `TritonShell` seam.

use std::time::Duration;

use triton::{
    BackendMode, OsdPayload, ShellEvent, ShellMessage, SurfaceRole, TIMER_OSD_HIDE, TIMER_OSD_REAP,
    TritonShell,
};
use triton_engine::{
    AppCommand, HeadlessSession, ShellApp, SurfaceLayer, SurfaceSize, SurfaceSpec, event_channel,
};

const OSD_HOLD: Duration = Duration::from_millis(1200);

fn fake_shell() -> TritonShell {
    TritonShell::new(BackendMode::Fake)
}

fn osd_spec(shell: &TritonShell) -> Option<(triton::SurfaceId, SurfaceSpec)> {
    shell
        .surfaces()
        .into_iter()
        .find(|(id, _)| id.role == SurfaceRole::Osd)
}

fn show_osd(
    shell: &mut TritonShell,
    payload: OsdPayload,
    now: std::time::Instant,
) -> Vec<AppCommand<triton::SurfaceId>> {
    shell.handle_event(
        ShellEvent::ShowOsd {
            payload,
            output: None,
        },
        now,
    )
}

/// Drive the reap deadline until the OSD fully unmounts, asserting bounded
/// convergence so a wedged exit animation fails loudly instead of looping.
fn reap_until_unmounted(shell: &mut TritonShell, session: &mut HeadlessSession<ShellMessage>) {
    for _ in 0..40 {
        if osd_spec(shell).is_none() {
            return;
        }
        session.advance(Duration::from_millis(100));
        let _ = shell.on_timer(TIMER_OSD_REAP, session.now());
    }
    assert!(
        osd_spec(shell).is_none(),
        "osd must unmount after its exit morph settles"
    );
}

#[test]
fn all_six_payloads_render_pointer_transparent_glass_pill() {
    let payloads: Vec<(OsdPayload, &str)> = vec![
        (
            OsdPayload::Volume {
                level: 0.6,
                muted: false,
            },
            "polyline",
        ),
        (OsdPayload::Brightness { level: 0.4 }, "polyline"),
        (OsdPayload::Mic { muted: true }, "muted"),
        (
            OsdPayload::KeyboardLayout {
                name: "English (Dvorak)".to_owned(),
            },
            "English (Dvorak)",
        ),
        (
            OsdPayload::Workspace {
                label: "3".to_owned(),
            },
            "3",
        ),
        (OsdPayload::LowBattery { percent: 15 }, "15%"),
    ];

    for (payload, expected_in_tree) in payloads {
        let mut shell = fake_shell();
        let mut session = HeadlessSession::<ShellMessage>::new((420.0, 224.0), 1.0);
        let cmds = show_osd(&mut shell, payload, session.now());
        assert!(cmds.contains(&AppCommand::ScheduleTimer {
            tag: TIMER_OSD_HIDE,
            delay: OSD_HOLD,
        }));

        let (id, spec) = osd_spec(&shell).expect("osd surface must mount");
        assert_eq!(spec.layer, SurfaceLayer::Overlay);
        assert!(spec.shaped_input);
        assert!(matches!(spec.sizing, SurfaceSize::StableExtent { .. }));

        // Let the entry morph settle so the pill is fully opaque (compositor
        // blur regions are gated on effective opacity).
        session.advance(Duration::from_millis(900));
        session.render_surface(&mut shell, id, spec.sizing);
        // Pointer transparency: nothing in the pill captures input, so the
        // compositor receives an empty input region and clicks pass through.
        assert!(
            session.frame().input_rects.is_empty(),
            "osd must emit empty input_rects for pointer transparency ({expected_in_tree})"
        );
        // Glass pill must report a compositor blur region.
        assert!(
            !session.frame().blur_rects.is_empty(),
            "glass pill must emit blur_rects ({expected_in_tree})"
        );
        assert!(
            session.tree_json().contains(expected_in_tree),
            "osd tree should contain {expected_in_tree:?}: {}",
            session.tree_json()
        );
    }
}

#[test]
fn hold_timer_refreshes_and_payload_switch_morphs_in_place() {
    let mut shell = fake_shell();
    let mut session = HeadlessSession::<ShellMessage>::new((420.0, 224.0), 1.0);

    let cmds = show_osd(
        &mut shell,
        OsdPayload::Volume {
            level: 0.5,
            muted: false,
        },
        session.now(),
    );
    assert!(cmds.contains(&AppCommand::ScheduleTimer {
        tag: TIMER_OSD_HIDE,
        delay: OSD_HOLD,
    }));
    let (id_before, spec_before) = osd_spec(&shell).expect("osd mounted");

    // A repeat event partway through the hold re-arms the full deadline and
    // keeps the same surface id: morph-in-place, never teardown/reallocate.
    session.advance(Duration::from_millis(800));
    let cmds = show_osd(
        &mut shell,
        OsdPayload::Brightness { level: 0.9 },
        session.now(),
    );
    assert!(
        cmds.contains(&AppCommand::ScheduleTimer {
            tag: TIMER_OSD_HIDE,
            delay: OSD_HOLD,
        }),
        "repeat events must refresh the 1.2s hold timer"
    );
    let (id_after, spec_after) = osd_spec(&shell).expect("osd still mounted");
    assert_eq!(id_before, id_after, "payload switch must reuse the surface");
    assert_eq!(spec_before.output, spec_after.output);
    assert_eq!(spec_before.sizing, spec_after.sizing);

    // When the hold timer does fire, the pill begins its exit morph but the
    // surface stays mounted until the morph settles.
    session.advance(Duration::from_millis(500));
    let cmds = shell.on_timer(TIMER_OSD_HIDE, session.now());
    assert!(cmds.contains(&AppCommand::ScheduleTimer {
        tag: TIMER_OSD_REAP,
        delay: Duration::from_millis(100),
    }));
    assert!(osd_spec(&shell).is_some());
}

#[test]
fn exit_morph_keeps_surface_mounted_until_settled_then_unmounts() {
    let mut shell = fake_shell();
    let mut session = HeadlessSession::<ShellMessage>::new((420.0, 224.0), 1.0);

    let _ = show_osd(&mut shell, OsdPayload::Mic { muted: false }, session.now());
    let (id, _) = osd_spec(&shell).expect("osd mounted");

    // Entry morph animates: not settled at t=0.
    assert!(shell.is_animating(id, session.now()));
    session.advance(Duration::from_millis(900));
    assert!(
        !shell.is_animating(id, session.now()),
        "entry spring settles well under a second"
    );

    // Hold expiry starts the exit morph; the surface remains mounted so the
    // morph can render out instead of popping away.
    session.advance(OSD_HOLD);
    let cmds = shell.on_timer(TIMER_OSD_HIDE, session.now());
    assert!(cmds.contains(&AppCommand::ScheduleTimer {
        tag: TIMER_OSD_REAP,
        delay: Duration::from_millis(100),
    }));
    assert!(
        osd_spec(&shell).is_some(),
        "surface stays mounted during the exit morph"
    );
    assert!(shell.is_animating(id, session.now()));

    reap_until_unmounted(&mut shell, &mut session);
}

#[test]
fn revive_during_exit_morphs_back_in_without_teardown() {
    let mut shell = fake_shell();
    let mut session = HeadlessSession::<ShellMessage>::new((420.0, 224.0), 1.0);

    let _ = show_osd(
        &mut shell,
        OsdPayload::Volume {
            level: 0.3,
            muted: false,
        },
        session.now(),
    );
    let (id, _) = osd_spec(&shell).expect("osd mounted");

    session.advance(OSD_HOLD);
    let _ = shell.on_timer(TIMER_OSD_HIDE, session.now());
    session.advance(Duration::from_millis(50));

    // A new event mid-exit revives the same surface and re-arms the hold.
    let cmds = show_osd(
        &mut shell,
        OsdPayload::Volume {
            level: 0.8,
            muted: false,
        },
        session.now(),
    );
    assert!(cmds.contains(&AppCommand::ScheduleTimer {
        tag: TIMER_OSD_HIDE,
        delay: OSD_HOLD,
    }));
    let (id_after, _) = osd_spec(&shell).expect("osd revived, still mounted");
    assert_eq!(id, id_after, "revive must reuse the same surface");

    // A stale reap timer firing after revival is a no-op: still mounted.
    let _ = shell.on_timer(TIMER_OSD_REAP, session.now());
    assert!(osd_spec(&shell).is_some());
}

#[test]
fn reduce_motion_uses_fade_and_unmounts_promptly() {
    let mut shell = fake_shell();
    let mut session = HeadlessSession::<ShellMessage>::new((420.0, 224.0), 1.0);

    let _ = shell.handle_event(ShellEvent::SetReduceMotion(true), session.now());
    session.set_reduce_motion(true);

    let _ = show_osd(
        &mut shell,
        OsdPayload::Brightness { level: 0.7 },
        session.now(),
    );
    let (id, _) = osd_spec(&shell).expect("osd mounted");

    // 120ms fade entry: settled shortly after.
    session.advance(Duration::from_millis(150));
    assert!(
        !shell.is_animating(id, session.now()),
        "reduce_motion entry is a 120ms fade, not a spring"
    );

    session.advance(OSD_HOLD);
    let _ = shell.on_timer(TIMER_OSD_HIDE, session.now());
    session.advance(Duration::from_millis(150));
    let _ = shell.on_timer(TIMER_OSD_REAP, session.now());
    assert!(
        osd_spec(&shell).is_none(),
        "reduce_motion exit fade must have settled and unmounted"
    );
}

#[test]
fn explicit_hide_unmounts_immediately_and_cancels_timers() {
    let mut shell = fake_shell();
    let mut session = HeadlessSession::<ShellMessage>::new((420.0, 224.0), 1.0);
    let (tx, rx) = event_channel::<ShellEvent>();

    tx.send(ShellEvent::ShowOsd {
        payload: OsdPayload::Workspace {
            label: "2".to_owned(),
        },
        output: None,
    })
    .unwrap();
    let _ = session.drain_events(&mut shell, &rx, TritonShell::handle_event);
    assert!(osd_spec(&shell).is_some());

    tx.send(ShellEvent::HideOsd).unwrap();
    let cmds = session.drain_events(&mut shell, &rx, TritonShell::handle_event);
    assert!(cmds.contains(&AppCommand::CancelTimer(TIMER_OSD_HIDE)));
    assert!(cmds.contains(&AppCommand::CancelTimer(TIMER_OSD_REAP)));
    assert!(osd_spec(&shell).is_none());

    // A stale hold timer firing after the explicit hide is a no-op.
    let cmds = shell.on_timer(TIMER_OSD_HIDE, session.now());
    assert_eq!(cmds, Vec::<AppCommand<triton::SurfaceId>>::new());
}

#[test]
fn show_osd_without_connected_outputs_is_a_noop() {
    let mut shell = TritonShell::new(BackendMode::RealServices);
    let session = HeadlessSession::<ShellMessage>::new((420.0, 224.0), 1.0);
    let cmds = show_osd(
        &mut shell,
        OsdPayload::Volume {
            level: 0.5,
            muted: false,
        },
        session.now(),
    );
    assert_eq!(cmds, Vec::<AppCommand<triton::SurfaceId>>::new());
    assert!(shell.state().osd.is_none());
}
