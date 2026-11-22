//! Integration tests for `triton-engine`'s multi-output, pointer enter/leave,
//! and external event channel seams.

use std::time::{Duration, Instant};

use triton_engine::{
    AppCommand, Element, EventReceiver, EventSender, HeadlessSession, NodeKey, OutputInfo, Rgba,
    ShellApp, SurfaceAnchor, SurfaceLayer, SurfaceSize, SurfaceSpec, ViewContext, box_,
    event_channel,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum SurfaceId {
    Bar(usize),
}

struct MultiOutputApp {
    outputs: Vec<OutputInfo>,
    updates_seen: usize,
}

impl MultiOutputApp {
    fn new() -> Self {
        Self {
            outputs: Vec::new(),
            updates_seen: 0,
        }
    }
}

impl ShellApp for MultiOutputApp {
    type SurfaceId = SurfaceId;
    type Message = ();

    fn surfaces(&self) -> Vec<(SurfaceId, SurfaceSpec)> {
        self.outputs
            .iter()
            .enumerate()
            .map(|(idx, out)| {
                let spec = SurfaceSpec::island(
                    "triton-bar",
                    SurfaceAnchor::TOP | SurfaceAnchor::LEFT | SurfaceAnchor::RIGHT,
                    (8, 0, 0, 0),
                )
                .on_output(out.name.clone());
                (SurfaceId::Bar(idx), spec)
            })
            .collect()
    }

    fn view(&self, _id: SurfaceId, _ctx: &mut ViewContext<'_>) -> Element<()> {
        box_().w(200.0).h(56.0).bg(Rgba::WHITE)
    }

    fn update(&mut self, _id: SurfaceId, _msg: (), _now: Instant) -> Vec<AppCommand<SurfaceId>> {
        Vec::new()
    }

    fn is_animating(&self, _id: SurfaceId, _now: Instant) -> bool {
        false
    }

    fn on_outputs_changed(
        &mut self,
        outputs: &[OutputInfo],
        _now: Instant,
    ) -> Vec<AppCommand<SurfaceId>> {
        self.outputs = outputs.to_vec();
        self.updates_seen += 1;
        vec![AppCommand::RedrawAll]
    }
}

#[test]
fn multi_output_inventory_reconciles_per_output_surfaces_on_hotplug_and_removal() {
    let mut app = MultiOutputApp::new();
    let mut session = HeadlessSession::<()>::new((1920.0, 1080.0), 1.0);

    // Default SurfaceSpec targets no specific output (compositor default).
    let default_spec = SurfaceSpec::island("triton-bar", SurfaceAnchor::TOP, (0, 0, 0, 0));
    assert_eq!(default_spec.output, None);
    assert_eq!(default_spec.layer, SurfaceLayer::Top);

    // Initial startup inventory with one output.
    let cmds = session.set_outputs(&mut app, vec![OutputInfo::new("eDP-1", (1920, 1080))]);
    assert_eq!(cmds, vec![AppCommand::RedrawAll]);
    assert_eq!(app.updates_seen, 1);
    assert_eq!(session.outputs().len(), 1);

    let specs = app.surfaces();
    assert_eq!(specs.len(), 1);
    assert_eq!(specs[0].1.output.as_deref(), Some("eDP-1"));

    // Hotplug a second monitor at virtual t + 5s.
    session.advance(Duration::from_secs(5));
    let cmds = session.add_output(&mut app, OutputInfo::new("DP-1", (2560, 1440)));
    assert_eq!(cmds, vec![AppCommand::RedrawAll]);
    assert_eq!(app.updates_seen, 2);
    let names: Vec<Option<String>> = app.surfaces().into_iter().map(|(_, s)| s.output).collect();
    assert_eq!(
        names,
        vec![Some("eDP-1".to_owned()), Some("DP-1".to_owned())]
    );

    // Update an existing output's geometry in place (same name replaces, no duplicate).
    let mut updated_dp = OutputInfo::new("DP-1", (3840, 2160));
    updated_dp.scale_factor = 2;
    session.add_output(&mut app, updated_dp);
    assert_eq!(session.outputs().len(), 2);
    assert_eq!(session.outputs()[1].logical_size, Some((3840, 2160)));
    assert_eq!(session.outputs()[1].scale_factor, 2);

    // Unplug DP-1.
    let cmds = session.remove_output(&mut app, "DP-1");
    assert_eq!(cmds, vec![AppCommand::RedrawAll]);
    assert_eq!(app.surfaces().len(), 1);
    assert_eq!(app.surfaces()[0].1.output.as_deref(), Some("eDP-1"));

    // Removing a nonexistent output is a no-op.
    let before = app.updates_seen;
    let cmds = session.remove_output(&mut app, "HDMI-A-99");
    assert_eq!(cmds, Vec::new());
    assert_eq!(app.updates_seen, before);

    // Render the remaining surface headlessly.
    let frame = session.render_surface(&mut app, SurfaceId::Bar(0), SurfaceSize::Fixed(200, 56));
    assert_eq!(frame.content_size, (200.0, 56.0));
}

const REVEAL_TIMER: u32 = 1;
const HIDE_TIMER: u32 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
enum PointerMsg {
    Clicked,
}

struct AutohideApp {
    entered: Vec<(SurfaceId, (f32, f32), Instant)>,
    left: Vec<(SurfaceId, Instant)>,
    clicks: usize,
}

impl ShellApp for AutohideApp {
    type SurfaceId = SurfaceId;
    type Message = PointerMsg;

    fn surfaces(&self) -> Vec<(SurfaceId, SurfaceSpec)> {
        vec![(
            SurfaceId::Bar(0),
            SurfaceSpec::island("triton-bar", SurfaceAnchor::TOP, (8, 0, 0, 0)),
        )]
    }

    fn view(&self, _id: SurfaceId, _ctx: &mut ViewContext<'_>) -> Element<PointerMsg> {
        box_()
            .id("btn")
            .w(120.0)
            .h(40.0)
            .bg(Rgba::WHITE)
            .on_click(|| PointerMsg::Clicked)
    }

    fn update(
        &mut self,
        _id: SurfaceId,
        msg: PointerMsg,
        _now: Instant,
    ) -> Vec<AppCommand<SurfaceId>> {
        match msg {
            PointerMsg::Clicked => {
                self.clicks += 1;
                vec![AppCommand::Redraw(SurfaceId::Bar(0))]
            }
        }
    }

    fn is_animating(&self, _id: SurfaceId, _now: Instant) -> bool {
        false
    }

    fn on_pointer_enter(
        &mut self,
        id: SurfaceId,
        pos: (f32, f32),
        now: Instant,
    ) -> Vec<AppCommand<SurfaceId>> {
        self.entered.push((id, pos, now));
        vec![
            AppCommand::CancelTimer(HIDE_TIMER),
            AppCommand::ScheduleTimer {
                tag: REVEAL_TIMER,
                delay: Duration::from_millis(120),
            },
        ]
    }

    fn on_pointer_leave(&mut self, id: SurfaceId, now: Instant) -> Vec<AppCommand<SurfaceId>> {
        self.left.push((id, now));
        vec![
            AppCommand::CancelTimer(REVEAL_TIMER),
            AppCommand::ScheduleTimer {
                tag: HIDE_TIMER,
                delay: Duration::from_millis(600),
            },
        ]
    }
}

#[test]
fn pointer_enter_and_leave_notify_app_while_preserving_widget_hover_and_click() {
    let mut app = AutohideApp {
        entered: Vec::new(),
        left: Vec::new(),
        clicks: 0,
    };
    let mut session = HeadlessSession::<PointerMsg>::new((200.0, 56.0), 1.0);
    session.render_surface(&mut app, SurfaceId::Bar(0), SurfaceSize::Fixed(200, 56));

    // Pointer enters over "btn" (0..120, 0..40).
    let t_enter = session.now();
    let cmds = session.pointer_enter(&mut app, SurfaceId::Bar(0), (50.0, 20.0));
    assert_eq!(
        app.entered,
        vec![(SurfaceId::Bar(0), (50.0, 20.0), t_enter)]
    );
    assert_eq!(
        cmds,
        vec![
            AppCommand::CancelTimer(HIDE_TIMER),
            AppCommand::ScheduleTimer {
                tag: REVEAL_TIMER,
                delay: Duration::from_millis(120),
            },
        ]
    );
    // Widget hover hit-testing still works on pointer enter.
    assert!(session.interaction().is_hovered(NodeKey::named("btn")));

    // Clicking the widget still dispatches its message.
    let msgs = session.tap_key("btn").expect("btn hit rect exists");
    assert_eq!(msgs, vec![PointerMsg::Clicked]);

    // Pointer leaves the surface at virtual t + 200ms.
    session.advance(Duration::from_millis(200));
    let t_leave = session.now();
    let cmds = session.pointer_leave(&mut app, SurfaceId::Bar(0));
    assert_eq!(app.left, vec![(SurfaceId::Bar(0), t_leave)]);
    assert_eq!(
        cmds,
        vec![
            AppCommand::CancelTimer(REVEAL_TIMER),
            AppCommand::ScheduleTimer {
                tag: HIDE_TIMER,
                delay: Duration::from_millis(600),
            },
        ]
    );
    // Widget hover state is cleared on pointer leave.
    assert!(!session.interaction().is_hovered(NodeKey::named("btn")));
}

#[test]
fn external_event_channel_delivers_cross_thread_events_fifo_in_headless_session() {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum ExtEvent {
        WorkspaceChanged(u32),
        ToggleHidden,
    }

    struct EventApp {
        workspaces: Vec<u32>,
        hidden: bool,
        timestamps: Vec<Instant>,
    }

    impl EventApp {
        fn on_external(&mut self, ev: ExtEvent, now: Instant) -> Vec<AppCommand<SurfaceId>> {
            self.timestamps.push(now);
            match ev {
                ExtEvent::WorkspaceChanged(ws) => {
                    self.workspaces.push(ws);
                    vec![AppCommand::RedrawAll]
                }
                ExtEvent::ToggleHidden => {
                    self.hidden = !self.hidden;
                    vec![AppCommand::RedrawAll]
                }
            }
        }
    }

    impl ShellApp for EventApp {
        type SurfaceId = SurfaceId;
        type Message = ();

        fn surfaces(&self) -> Vec<(SurfaceId, SurfaceSpec)> {
            if self.hidden {
                Vec::new()
            } else {
                vec![(
                    SurfaceId::Bar(0),
                    SurfaceSpec::island("triton-bar", SurfaceAnchor::TOP, (8, 0, 0, 0)),
                )]
            }
        }

        fn view(&self, _id: SurfaceId, _ctx: &mut ViewContext<'_>) -> Element<()> {
            box_().w(100.0).h(56.0).bg(Rgba::WHITE)
        }

        fn update(
            &mut self,
            _id: SurfaceId,
            _msg: (),
            _now: Instant,
        ) -> Vec<AppCommand<SurfaceId>> {
            Vec::new()
        }

        fn is_animating(&self, _id: SurfaceId, _now: Instant) -> bool {
            false
        }
    }

    let (tx, rx): (EventSender<ExtEvent>, EventReceiver<ExtEvent>) = event_channel();
    let tx_bg = tx.clone();

    // Push events from a background thread (just like niri IPC / socket server).
    let handle = std::thread::Builder::new()
        .name("test-ext-event-sender".into())
        .spawn(move || {
            tx_bg.send(ExtEvent::WorkspaceChanged(2)).unwrap();
            tx_bg.send(ExtEvent::WorkspaceChanged(5)).unwrap();
            tx_bg.send(ExtEvent::ToggleHidden).unwrap();
        })
        .expect("spawn sender thread");
    handle.join().expect("sender thread succeeded");

    let mut app = EventApp {
        workspaces: Vec::new(),
        hidden: false,
        timestamps: Vec::new(),
    };
    let mut session = HeadlessSession::<()>::new((400.0, 56.0), 1.0);
    session.advance(Duration::from_millis(750));
    let expected_now = session.now();

    let cmds = session.drain_events(&mut app, &rx, EventApp::on_external);
    assert_eq!(cmds.len(), 3);
    assert_eq!(app.workspaces, vec![2, 5]);
    assert!(app.hidden);
    assert_eq!(app.surfaces(), Vec::new());
    assert_eq!(app.timestamps, vec![expected_now; 3]);

    // Draining again when empty yields no commands.
    let empty_cmds = session.drain_events(&mut app, &rx, EventApp::on_external);
    assert_eq!(empty_cmds, Vec::new());

    // Dropping the receiver causes subsequent sends to return an error cleanly.
    drop(rx);
    assert!(tx.send(ExtEvent::ToggleHidden).is_err());
}
