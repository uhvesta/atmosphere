//! Embedding proof: no RemoteWorkspace, network, resource installation or app chrome.
use ghostty_vt::{Colors, Grid, Rgb, Terminal};
use gpui_kit::{
    AnyWindowHandle, AssetSource, Bounds, Context, Entity, Render, SharedString, Subscription,
    Window, WindowBounds, WindowOptions, div, point, prelude::*, px, size, test::TestWindowExt,
};
use std::{
    borrow::Cow,
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};
use terminal_pane::{PaneEvent, Pty, TerminalPane};
use terminal_replica::{
    FitPolicy, HostAction, NewTerminal, ReplicaConfig, ReplicaError, ReplicaEvent, ReplicaPhase,
    ReplicaSink, SendError, ShortcutPolicy, TerminalReplica, register_host_shortcuts,
};
use terminal_view::{CellSize, Theme};
use terminal_wire::{
    ClientFrame, InjectionOptions, InputTicket, MAX_INPUT_BYTES, SNAPSHOT_ENGINE, ServerFrame,
    SessionInfo, SessionState,
};
struct EmptyAssets;
impl AssetSource for EmptyAssets {
    fn load(&self, _: &str) -> gpui_kit::Result<Option<Cow<'static, [u8]>>> {
        Ok(None)
    }
    fn list(&self, _: &str) -> gpui_kit::Result<Vec<SharedString>> {
        Ok(Vec::new())
    }
}
#[derive(Default)]
struct Sink {
    frames: RefCell<Vec<ClientFrame>>,
    reject: Cell<bool>,
    reject_snapshot: Cell<bool>,
    reject_ack: Cell<bool>,
}
impl ReplicaSink for Sink {
    fn send(&self, frame: ClientFrame) -> Result<(), SendError> {
        if self.reject.get() && matches!(frame, ClientFrame::Input { .. }) {
            return Err(SendError::Full);
        }
        if self.reject_snapshot.get() && matches!(frame, ClientFrame::SnapshotRequest { .. }) {
            return Err(SendError::Full);
        }
        if self.reject_ack.get() && matches!(frame, ClientFrame::Ack { .. }) {
            return Err(SendError::Full);
        }
        self.frames.borrow_mut().push(frame);
        Ok(())
    }
    // Deliberately unknown: the ordered Detached barrier proves delivery.
}
#[derive(Debug)]
enum Seen {
    Queued(InputTicket),
    Written(InputTicket),
    Uncertain(InputTicket),
    Bell,
    Provider,
    Shortcut(HostAction),
    Refused,
}
struct Embed {
    child: Entity<TerminalReplica>,
    _events: Subscription,
    outside: Rc<Cell<usize>>,
    outside_focus: gpui_kit::FocusHandle,
}
impl Render for Embed {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .track_focus(&self.outside_focus)
            .on_action({
                let outside = self.outside.clone();
                move |_: &NewTerminal, _, _| outside.set(outside.get() + 1)
            })
            .child(self.child.clone())
    }
}
fn info() -> SessionInfo {
    SessionInfo {
        id: 1,
        uuid: "00000000-0000-4000-8000-000000000001".into(),
        title: "supplied".into(),
        automatic_title: "supplied".into(),
        name_override: None,
        attributes: Default::default(),
        cwd: "/not-accessed".into(),
        program: "supplied".into(),
        state: SessionState::Running,
        recovery_available: false,
    }
}
fn theme() -> Theme {
    Theme {
        font_family: "JetBrains Mono".into(),
        ..Default::default()
    }
}
fn config(idle: Option<Duration>, shortcuts: ShortcutPolicy) -> ReplicaConfig {
    ReplicaConfig {
        session: info(),
        theme: theme(),
        memory_saver: idle,
        shortcuts,
        fit: FitPolicy::default(),
        default_colors: None,
    }
}
fn options() -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
            point(px(0.), px(0.)),
            size(px(800.), px(600.)),
        ))),
        show: false,
        focus: false,
        ..Default::default()
    }
}
struct Harness {
    root: Entity<Embed>,
    outside: Rc<Cell<usize>>,
    cx: gpui_kit::HeadlessAppContext,
    window: AnyWindowHandle,
    child: Entity<TerminalReplica>,
    sink: Rc<Sink>,
    seen: Rc<RefCell<Vec<Seen>>>,
}
impl Harness {
    fn new(idle: Option<Duration>) -> Self {
        Self::with_shortcuts(idle, ShortcutPolicy::default())
    }
    fn with_shortcuts(idle: Option<Duration>, shortcuts: ShortcutPolicy) -> Self {
        Self::configured(config(idle, shortcuts))
    }
    fn configured(config: ReplicaConfig) -> Self {
        let shortcuts = config.shortcuts;
        let resources = humata2_engine::Resources::new(EmptyAssets)
            .font(include_bytes!("../assets/Inter-Regular.ttf"))
            .font(include_bytes!("../assets/JetBrainsMono-Regular.ttf"));
        let mut cx = humata2_engine::headless(&resources).unwrap();
        let sink = Rc::new(Sink::default());
        let sender = sink.clone();
        let seen = Rc::new(RefCell::new(Vec::new()));
        let observed = seen.clone();
        let outside = Rc::new(Cell::new(0));
        let outside_root = outside.clone();
        cx.update(|cx| register_host_shortcuts(shortcuts, cx));
        let before = cx.update(|cx| gpui_kit::component::theme::Theme::global(cx).mode);
        let (window, root) = cx
            .update(|cx| {
                gpui_kit::open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                            point(px(0.), px(0.)),
                            size(px(800.), px(600.)),
                        ))),
                        show: false,
                        focus: false,
                        ..Default::default()
                    },
                    cx,
                    |window, cx| {
                        cx.new(|cx| {
                            let child = cx.new(|cx| {
                                TerminalReplica::new(config, sender, window, cx).unwrap()
                            });
                            let events =
                                cx.subscribe(&child, move |_, _, event: &ReplicaEvent, _| {
                                    let event = match event {
                                        ReplicaEvent::InputQueued(ticket) => {
                                            Some(Seen::Queued(ticket.clone()))
                                        }
                                        ReplicaEvent::InputWritten(ticket) => {
                                            Some(Seen::Written(ticket.clone()))
                                        }
                                        ReplicaEvent::InputUncertain(ticket) => {
                                            Some(Seen::Uncertain(ticket.clone()))
                                        }
                                        ReplicaEvent::Pane(PaneEvent::Bell) => Some(Seen::Bell),
                                        ReplicaEvent::Pane(PaneEvent::InputRefused) => {
                                            Some(Seen::Refused)
                                        }
                                        ReplicaEvent::Provider(_) => Some(Seen::Provider),
                                        ReplicaEvent::ShortcutRequested { action, .. } => {
                                            Some(Seen::Shortcut(*action))
                                        }
                                        _ => None,
                                    };
                                    if let Some(event) = event {
                                        observed.borrow_mut().push(event);
                                    }
                                });
                            Embed {
                                child,
                                _events: events,
                                outside: outside_root,
                                outside_focus: cx.focus_handle(),
                            }
                        })
                    },
                )
            })
            .unwrap();
        assert_eq!(
            cx.update(|cx| gpui_kit::component::theme::Theme::global(cx).mode),
            before
        );
        let child = cx.update(|cx| root.read(cx).child.clone());
        Self {
            root,
            outside,
            cx,
            window,
            child,
            sink,
            seen,
        }
    }
    fn bind(&mut self) {
        self.cx.update(|cx| {
            self.child.update(cx, |child, cx| {
                child.bind_connection(7, info(), cx).unwrap()
            })
        });
        self.cx.run_until_parked();
    }
    fn apply(&mut self, frame: ServerFrame) -> Result<(), ReplicaError> {
        let result = self
            .cx
            .update_window(self.window, |_, window, cx| {
                self.child
                    .update(cx, |child, cx| child.apply(frame, 7, window, cx))
            })
            .unwrap();
        self.cx.run_until_parked();
        result
    }
    fn draw(&mut self) {
        self.cx
            .update_window(self.window, |_, window, cx| {
                window.render_frame(cx);
                window.render_frame(cx);
            })
            .unwrap();
        self.cx.run_until_parked();
    }
    fn snapshot(&mut self, terminal: &Terminal, offset: u64) {
        let size = terminal.size();
        self.apply(ServerFrame::Snapshot {
            id: 1,
            engine: SNAPSHOT_ENGINE.into(),
            bytes: terminal.checkpoint(16 << 20).unwrap(),
            offset,
            cols: size.cols,
            rows: size.rows,
            live: true,
            exited: None,
        })
        .unwrap();
    }
    fn live(&mut self, terminal: &Terminal) {
        self.bind();
        self.apply(ServerFrame::Control {
            id: 1,
            controller: true,
        })
        .unwrap();
        self.snapshot(terminal, 0);
        self.draw();
        self.sink.frames.borrow_mut().clear();
    }
    fn inject(
        &mut self,
        text: &str,
        options: InjectionOptions,
    ) -> Result<InputTicket, ReplicaError> {
        let result = self.cx.update(|cx| {
            self.child
                .update(cx, |child, cx| child.inject_text(text, options, cx))
        });
        self.cx.run_until_parked();
        result
    }
}
#[test]
fn standalone_export_restore_is_atomic_readonly_and_keeps_same_connection_sequences() {
    let mut h = Harness::new(None);
    let mut terminal = Terminal::new(80, 24, 1000).unwrap();
    terminal.feed(b"supplied state\x1b[?2004h");
    h.live(&terminal);
    let queued = h.inject("hello", InjectionOptions::raw()).unwrap();
    assert_eq!(queued.seq, 1);
    assert!(
        h.seen
            .borrow()
            .iter()
            .any(|event| matches!(event,Seen::Queued(ticket)if ticket==&queued))
    );
    assert!(
        !h.seen
            .borrow()
            .iter()
            .any(|event| matches!(event, Seen::Written(_)))
    );
    let checkpoint =
        h.cx.update(|cx| h.child.read(cx).checkpoint(16 << 20, cx).unwrap());
    let old = h.cx.update(|cx| h.child.read(cx).pane().unwrap().clone());
    let mut bad = checkpoint.clone();
    bad.bytes[0] ^= 1;
    let error =
        h.cx.update_window(h.window, |_, window, cx| {
            h.child
                .update(cx, |child, cx| child.restore_checkpoint(&bad, window, cx))
        })
        .unwrap();
    assert!(matches!(error, Err(ReplicaError::Snapshot { .. })));
    assert_eq!(
        h.cx.update(|cx| h.child.read(cx).state().phase),
        ReplicaPhase::Live
    );
    assert_eq!(
        h.cx.update(|cx| h.child.read(cx).pane().unwrap().entity_id()),
        old.entity_id()
    );
    h.cx.update_window(h.window, |_, window, cx| {
        h.child.update(cx, |child, cx| {
            child.restore_checkpoint(&checkpoint, window, cx).unwrap()
        })
    })
    .unwrap();
    h.cx.run_until_parked();
    assert!(matches!(
        h.inject("blocked", InjectionOptions::raw()),
        Err(ReplicaError::NotReady { .. })
    ));
    assert!(
        h.seen
            .borrow()
            .iter()
            .any(|event| matches!(event,Seen::Uncertain(ticket)if ticket==&queued))
    );
    h.bind();
    h.apply(ServerFrame::Control {
        id: 1,
        controller: true,
    })
    .unwrap();
    h.snapshot(&terminal, 0);
    h.sink.frames.borrow_mut().clear();
    let next = h.inject("next", InjectionOptions::raw()).unwrap();
    assert_eq!(next.seq, 2);
    assert!(
        matches!(h.sink.frames.borrow().as_slice(),[ClientFrame::Input{seq:2,bytes,..}]if bytes==b"next")
    );
    h.apply(ServerFrame::InputWritten { id: 1, seq: 2 })
        .unwrap();
    assert!(
        h.seen
            .borrow()
            .iter()
            .any(|event| matches!(event,Seen::Written(ticket)if ticket==&next))
    );
    assert!(
        !h.seen
            .borrow()
            .iter()
            .any(|event| matches!(event,Seen::Written(ticket)if ticket==&queued))
    );
}
#[test]
fn encoded_injection_budget_is_atomic_and_does_not_consume_sequence_numbers() {
    let mut h = Harness::new(None);
    let mut terminal = Terminal::new(80, 24, 1000).unwrap();
    terminal.feed(b"\x1b[?2004h");
    h.live(&terminal);
    let text = "x".repeat(MAX_INPUT_BYTES - 12);
    let mut options = InjectionOptions::paste();
    options.submit = true;
    assert!(
        matches!(h.inject(&text,options),Err(ReplicaError::InputTooLarge{actual,maximum})if actual>maximum)
    );
    assert!(h.sink.frames.borrow().is_empty());
    assert_eq!(h.cx.update(|cx| h.child.read(cx).state().input_seq), 0);
    h.sink.reject.set(true);
    assert!(matches!(
        h.inject("retry", InjectionOptions::raw()),
        Err(ReplicaError::Send { .. })
    ));
    assert_eq!(h.cx.update(|cx| h.child.read(cx).state().input_seq), 0);
    h.sink.reject.set(false);
    let ticket = h.inject("ok", InjectionOptions::raw()).unwrap();
    assert_eq!(ticket.seq, 1);
    assert!(
        matches!(h.sink.frames.borrow().as_slice(),[ClientFrame::Input{bytes,..}]if bytes==b"ok")
    );
}
#[test]
fn reused_route_cannot_bind_another_uuid_or_restore_its_authority() {
    let mut h = Harness::new(None);
    let terminal = Terminal::new(80, 24, 1000).unwrap();
    h.live(&terminal);
    let old =
        h.cx.update(|cx| h.child.read(cx).pane().unwrap().entity_id());
    let mut wrong = info();
    wrong.uuid = "00000000-0000-4000-8000-000000000002".into();
    let result = h.cx.update(|cx| {
        h.child
            .update(cx, |child, cx| child.bind_connection(8, wrong, cx))
    });
    assert!(matches!(result, Err(ReplicaError::WrongIdentity)));
    assert_eq!(
        h.cx.update(|cx| h.child.read(cx).pane().unwrap().entity_id()),
        old
    );
    assert!(matches!(
        h.inject("never", InjectionOptions::raw()),
        Err(ReplicaError::NotReady { .. })
    ));
    let result =
        h.cx.update_window(h.window, |_, window, cx| {
            h.child.update(cx, |child, cx| {
                child.apply(
                    ServerFrame::Control {
                        id: 1,
                        controller: true,
                    },
                    8,
                    window,
                    cx,
                )
            })
        })
        .unwrap();
    assert!(matches!(result, Err(ReplicaError::UnboundConnection)));
}
#[test]
fn busy_inactive_terminal_keeps_vt_until_detached_and_wakes_from_fresh_checkpoint() {
    let mut h = Harness::new(Some(Duration::ZERO));
    let mut terminal = Terminal::new(80, 24, 1000).unwrap();
    terminal.feed(b"before");
    h.live(&terminal);
    let old =
        h.cx.update(|cx| h.child.read(cx).pane().unwrap().read(cx).view().downgrade());
    h.cx.update(|cx| {
        h.child
            .update(cx, |child, cx| child.set_visible(false, cx).unwrap())
    });
    h.cx.run_until_parked();
    assert_eq!(
        h.cx.update(|cx| h.child.read(cx).state().phase),
        ReplicaPhase::Detaching
    );
    assert!(
        old.upgrade().is_some(),
        "the VT stays resident until ordered host acknowledgement"
    );
    for _ in 0..100 {
        h.apply(ServerFrame::Output {
            id: 1,
            offset: 0,
            bytes: b"busy output".to_vec(),
        })
        .unwrap();
    }
    h.cx.update(|cx| {
        h.child
            .update(cx, |child, cx| child.set_visible(true, cx).unwrap())
    });
    h.cx.run_until_parked();
    h.snapshot(&terminal, 0);
    assert_eq!(
        h.cx.update(|cx| h.child.read(cx).state().phase),
        ReplicaPhase::Detaching
    );
    h.apply(ServerFrame::Detached { id: 1 }).unwrap();
    h.draw();
    assert!(old.upgrade().is_none());
    assert_eq!(
        h.cx.update(|cx| h.child.read(cx).state().phase),
        ReplicaPhase::Restoring
    );
    assert!(matches!(
        h.inject("too early", InjectionOptions::raw()),
        Err(ReplicaError::NotReady { .. })
    ));
    terminal.feed(b" fresh background state");
    h.apply(ServerFrame::Control {
        id: 1,
        controller: true,
    })
    .unwrap();
    h.snapshot(&terminal, 100);
    assert_eq!(
        h.cx.update(|cx| h.child.read(cx).state().phase),
        ReplicaPhase::Live
    );
    h.inject("resumed", InjectionOptions::raw()).unwrap();
}

/// Keys typed into a tab before its screen arrives reach no program: the
/// app is told once, and the screen takes the keys that follow.
#[test]
fn keys_before_the_first_screen_are_told_once_then_the_screen_takes_them() {
    let mut h = Harness::new(None);
    let press = |h: &mut Harness, key: &str| {
        h.cx.update_window(h.window, |_, window, cx| {
            let key = gpui_kit::Keystroke::parse_with_os(key, cx.platform_info().os).unwrap();
            window.dispatch_keystroke(key, cx);
        })
        .unwrap();
        h.cx.run_until_parked();
    };
    h.cx.update_window(h.window, |_, window, cx| {
        window.activate_window();
        h.child
            .update(cx, |child, cx| child.focus_input(window, cx));
        window.render_frame(cx);
        window.render_frame(cx);
    })
    .unwrap();
    h.cx.run_until_parked();
    press(&mut h, "a");
    press(&mut h, "b");
    let refused = |h: &Harness| {
        h.seen
            .borrow()
            .iter()
            .filter(|event| matches!(event, Seen::Refused))
            .count()
    };
    assert_eq!(refused(&h), 1);

    let terminal = Terminal::new(80, 24, 1000).unwrap();
    h.live(&terminal);
    press(&mut h, "up");
    assert!(
        h.sink
            .frames
            .borrow()
            .iter()
            .any(|frame| matches!(frame, ClientFrame::Input { .. }))
    );
    assert_eq!(refused(&h), 1);
}

#[test]
fn snapshot_request_backpressure_disables_focused_input_and_allows_retry() {
    let mut h = Harness::new(None);
    let terminal = Terminal::new(80, 24, 1000).unwrap();
    h.live(&terminal);
    h.cx.update_window(h.window, |_, window, cx| {
        window.activate_window();
        h.child
            .update(cx, |child, cx| child.focus_input(window, cx));
        window.render_frame(cx);
        window.render_frame(cx);
    })
    .unwrap();
    h.cx.run_until_parked();
    h.sink.frames.borrow_mut().clear();
    h.cx.update_window(h.window, |_, window, cx| {
        let key = gpui_kit::Keystroke::parse_with_os("up", cx.platform_info().os).unwrap();
        window.dispatch_keystroke(key, cx);
    })
    .unwrap();
    h.cx.run_until_parked();
    assert!(
        h.sink
            .frames
            .borrow()
            .iter()
            .any(|frame| matches!(frame, ClientFrame::Input { .. }))
    );
    h.apply(ServerFrame::InputWritten { id: 1, seq: 1 })
        .unwrap();
    h.sink.frames.borrow_mut().clear();

    h.sink.reject_snapshot.set(true);
    assert!(matches!(
        h.apply(ServerFrame::Output {
            id: 1,
            offset: 1,
            bytes: b"gap".to_vec()
        }),
        Err(ReplicaError::Send { .. })
    ));
    assert_eq!(
        h.cx.update(|cx| h.child.read(cx).state().phase),
        ReplicaPhase::Restoring
    );
    h.sink.reject_snapshot.set(false);
    h.cx.update_window(h.window, |_, window, cx| {
        let key = gpui_kit::Keystroke::parse_with_os("up", cx.platform_info().os).unwrap();
        window.dispatch_keystroke(key, cx);
    })
    .unwrap();
    h.cx.run_until_parked();
    assert!(
        !h.sink
            .frames
            .borrow()
            .iter()
            .any(|frame| matches!(frame, ClientFrame::Input { .. })),
        "failed resync must disable focused terminal input immediately"
    );
    h.apply(ServerFrame::ResyncRequired {
        id: 1,
        reason: "retry after queue drains".into(),
    })
    .unwrap();
    assert!(
        h.sink
            .frames
            .borrow()
            .iter()
            .any(|frame| matches!(frame, ClientFrame::SnapshotRequest { id: 1 })),
        "a rejected request must not leave the replica permanently opening"
    );
}

#[test]
fn a_hidden_replica_frees_its_screen_and_shows_again_from_a_fresh_snapshot() {
    let mut h = Harness::new(Some(Duration::ZERO));
    let mut terminal = Terminal::new(80, 24, 1000).unwrap();
    terminal.feed(b"before");
    h.live(&terminal);
    h.cx.update(|cx| {
        h.child
            .update(cx, |child, cx| child.set_visible(false, cx).unwrap())
    });
    h.cx.run_until_parked();
    h.apply(ServerFrame::Detached { id: 1 }).unwrap();
    h.draw();
    assert!(
        h.cx.update(|cx| h.child.read(cx).pane().is_none()),
        "the screen is freed once the host confirms the detach"
    );
    h.cx.update(|cx| {
        h.child
            .update(cx, |child, cx| child.set_visible(true, cx).unwrap())
    });
    h.cx.run_until_parked();
    terminal.feed(b" and after");
    h.apply(ServerFrame::Control {
        id: 1,
        controller: true,
    })
    .unwrap();
    h.snapshot(&terminal, 50);
    h.draw();
    assert_eq!(
        h.cx.update(|cx| h.child.read(cx).state().phase),
        ReplicaPhase::Live
    );
    assert!(h.cx.update(|cx| h.child.read(cx).pane().is_some()));
}

#[test]
fn a_replica_hidden_for_less_than_the_delay_keeps_its_screen() {
    let mut h = Harness::new(Some(Duration::from_secs(3600)));
    let terminal = Terminal::new(80, 24, 1000).unwrap();
    h.live(&terminal);
    h.cx.update(|cx| {
        h.child
            .update(cx, |child, cx| child.set_visible(false, cx).unwrap())
    });
    h.cx.run_until_parked();
    assert_eq!(
        h.cx.update(|cx| h.child.read(cx).state().phase),
        ReplicaPhase::Live
    );
    assert!(
        !h.sink
            .frames
            .borrow()
            .iter()
            .any(|frame| matches!(frame, ClientFrame::Detach { .. }))
    );
}

#[test]
fn typing_the_host_has_not_confirmed_keeps_a_hidden_replica_until_it_is() {
    let mut h = Harness::new(Some(Duration::ZERO));
    let terminal = Terminal::new(80, 24, 1000).unwrap();
    h.live(&terminal);
    let ticket = h.inject("ls", InjectionOptions::raw()).unwrap();
    h.cx.update(|cx| {
        h.child
            .update(cx, |child, cx| child.set_visible(false, cx).unwrap())
    });
    h.cx.run_until_parked();
    assert_eq!(
        h.cx.update(|cx| h.child.read(cx).state().phase),
        ReplicaPhase::Live,
        "input whose outcome is unknown pins the screen"
    );
    assert!(
        !h.sink
            .frames
            .borrow()
            .iter()
            .any(|frame| matches!(frame, ClientFrame::Detach { .. }))
    );
    h.apply(ServerFrame::InputWritten {
        id: 1,
        seq: ticket.seq,
    })
    .unwrap();
    assert_eq!(
        h.cx.update(|cx| h.child.read(cx).state().phase),
        ReplicaPhase::Detaching,
        "once the host confirms the write the screen can be freed"
    );
}

#[test]
fn host_input_refusal_resolves_ticket_without_written_or_replay() {
    let mut h = Harness::new(Some(Duration::ZERO));
    let terminal = Terminal::new(80, 24, 1000).unwrap();
    h.live(&terminal);
    let ticket = h
        .inject("host will refuse", InjectionOptions::raw())
        .unwrap();
    h.sink.frames.borrow_mut().clear();
    h.apply(ServerFrame::Error {
        id: Some(1),
        code: "input_backpressure".into(),
        message: "not queued".into(),
    })
    .unwrap();
    assert!(
        h.seen
            .borrow()
            .iter()
            .any(|event| matches!(event, Seen::Uncertain(actual) if actual == &ticket))
    );
    assert!(
        !h.seen
            .borrow()
            .iter()
            .any(|event| matches!(event, Seen::Written(actual) if actual == &ticket))
    );
    assert_eq!(
        h.cx.update(|cx| h.child.read(cx).state().written_seq),
        0,
        "a refusal is not a PTY written acknowledgement"
    );
    assert!(
        !h.sink
            .frames
            .borrow()
            .iter()
            .any(|frame| matches!(frame, ClientFrame::Input { .. })),
        "refused input must not be replayed automatically"
    );
    h.cx.update(|cx| {
        h.child
            .update(cx, |child, cx| child.set_visible(false, cx).unwrap())
    });
    h.cx.run_until_parked();
    assert_eq!(
        h.cx.update(|cx| h.child.read(cx).state().phase),
        ReplicaPhase::Detaching,
        "resolved refused input must not pin an inactive replica forever"
    );
}

#[test]
fn initially_hidden_bound_replica_opens_when_first_shown() {
    for saver in [None, Some(Duration::ZERO)] {
        let mut h = Harness::new(saver);
        h.cx.update(|cx| {
            h.child
                .update(cx, |child, cx| child.set_visible(false, cx).unwrap())
        });
        h.cx.run_until_parked();
        h.bind();
        assert!(
            !h.sink
                .frames
                .borrow()
                .iter()
                .any(|frame| matches!(frame, ClientFrame::Open { .. })),
            "binding a hidden terminal must not attach it prematurely"
        );
        h.cx.update(|cx| {
            h.child
                .update(cx, |child, cx| child.set_visible(true, cx).unwrap())
        });
        h.cx.run_until_parked();
        assert!(
            h.sink
                .frames
                .borrow()
                .iter()
                .any(|frame| matches!(frame, ClientFrame::Open { id: 1 })),
            "the first show must attach a bound terminal even if it was never parked"
        );
        let terminal = Terminal::new(80, 24, 1000).unwrap();
        h.apply(ServerFrame::Control {
            id: 1,
            controller: true,
        })
        .unwrap();
        h.snapshot(&terminal, 0);
        h.draw();
        assert_eq!(
            h.cx.update(|cx| h.child.read(cx).state().phase),
            ReplicaPhase::Live
        );
        h.inject("first shown", InjectionOptions::raw()).unwrap();
    }
}

#[test]
fn cached_restore_during_detach_is_atomic_and_preserves_reconnect_progress() {
    let mut h = Harness::new(Some(Duration::ZERO));
    let mut terminal = Terminal::new(80, 24, 1000).unwrap();
    terminal.feed(b"still attached");
    h.live(&terminal);
    let checkpoint =
        h.cx.update(|cx| h.child.read(cx).checkpoint(16 << 20, cx).unwrap());
    h.cx.update(|cx| {
        h.child
            .update(cx, |child, cx| child.set_visible(false, cx).unwrap())
    });
    h.cx.run_until_parked();
    let before = h.cx.update(|cx| h.child.read(cx).state());
    assert_eq!(before.phase, ReplicaPhase::Detaching);
    let pane =
        h.cx.update(|cx| h.child.read(cx).pane().unwrap().entity_id());
    h.sink.frames.borrow_mut().clear();
    let result =
        h.cx.update_window(h.window, |_, window, cx| {
            h.child.update(cx, |child, cx| {
                child.restore_checkpoint(&checkpoint, window, cx)
            })
        })
        .unwrap();
    assert!(matches!(result, Err(ReplicaError::LifecycleBusy)));
    assert_eq!(
        h.cx.update(|cx| h.child.read(cx).state()),
        before,
        "a refused restore must preserve the in-flight detach atomically"
    );
    assert_eq!(
        h.cx.update(|cx| h.child.read(cx).pane().unwrap().entity_id()),
        pane
    );
    h.apply(ServerFrame::Detached { id: 1 }).unwrap();
    h.cx.update(|cx| {
        h.child
            .update(cx, |child, cx| child.set_visible(true, cx).unwrap())
    });
    h.cx.run_until_parked();
    h.bind();
    assert!(
        h.sink
            .frames
            .borrow()
            .iter()
            .any(|frame| matches!(frame, ClientFrame::Open { id: 1 }))
    );
    assert_eq!(
        h.cx.update(|cx| h.child.read(cx).state().phase),
        ReplicaPhase::Restoring
    );
    h.apply(ServerFrame::Control {
        id: 1,
        controller: true,
    })
    .unwrap();
    h.snapshot(&terminal, 0);
    h.draw();
    assert_eq!(
        h.cx.update(|cx| h.child.read(cx).state().phase),
        ReplicaPhase::Live
    );
    h.inject("fresh attach", InjectionOptions::raw()).unwrap();
}

#[test]
fn snapshot_ack_backpressure_keeps_focused_input_synchronized_and_retries_ack() {
    let mut h = Harness::new(None);
    let terminal = Terminal::new(80, 24, 1000).unwrap();
    h.bind();
    h.apply(ServerFrame::Control {
        id: 1,
        controller: true,
    })
    .unwrap();
    h.sink.reject_ack.set(true);
    assert!(matches!(
        h.apply(ServerFrame::Snapshot {
            id: 1,
            engine: SNAPSHOT_ENGINE.into(),
            bytes: terminal.checkpoint(16 << 20).unwrap(),
            offset: 37,
            cols: 80,
            rows: 24,
            live: true,
            exited: None,
        }),
        Err(ReplicaError::Send { .. })
    ));
    assert_eq!(
        h.cx.update(|cx| h.child.read(cx).state().phase),
        ReplicaPhase::Live
    );
    h.cx.update_window(h.window, |_, window, cx| {
        window.activate_window();
        h.child
            .update(cx, |child, cx| child.focus_input(window, cx));
        window.render_frame(cx);
        window.render_frame(cx);
    })
    .unwrap();
    h.cx.run_until_parked();
    h.sink.frames.borrow_mut().clear();
    h.cx.update_window(h.window, |_, window, cx| {
        let key = gpui_kit::Keystroke::parse_with_os("up", cx.platform_info().os).unwrap();
        window.dispatch_keystroke(key, cx);
    })
    .unwrap();
    h.cx.run_until_parked();
    assert!(
        h.sink
            .frames
            .borrow()
            .iter()
            .any(|frame| matches!(frame, ClientFrame::Input { .. })),
        "a committed live snapshot must enable the focused pane even if its acknowledgement is backpressured"
    );
    h.sink.reject_ack.set(false);
    h.cx.update(|cx| h.child.update(cx, |child, cx| child.outbound_flushed(cx)));
    h.cx.run_until_parked();
    assert!(
        h.sink
            .frames
            .borrow()
            .iter()
            .any(|frame| matches!(frame, ClientFrame::Ack { id: 1, offset: 37 }))
    );
}

#[test]
fn unavailable_snapshot_does_not_claim_process_exit() {
    let mut h = Harness::new(None);
    let terminal = Terminal::new(80, 24, 1000).unwrap();
    h.bind();
    h.apply(ServerFrame::Control {
        id: 1,
        controller: true,
    })
    .unwrap();
    h.apply(ServerFrame::Snapshot {
        id: 1,
        engine: SNAPSHOT_ENGINE.into(),
        bytes: terminal.checkpoint(16 << 20).unwrap(),
        offset: 0,
        cols: 80,
        rows: 24,
        live: false,
        exited: None,
    })
    .unwrap();
    assert_eq!(
        h.cx.update(|cx| h.child.read(cx).state().phase),
        ReplicaPhase::Unavailable
    );
    assert!(matches!(
        h.inject("blocked", InjectionOptions::raw()),
        Err(ReplicaError::NotReady { .. })
    ));
    h.apply(ServerFrame::Exited { id: 1, code: None }).unwrap();
    assert_eq!(
        h.cx.update(|cx| h.child.read(cx).state().phase),
        ReplicaPhase::Exited
    );
}

/// Lets the pane draw its own disconnected banner, as an app that has no
/// notice of its own does.
fn show_offline_banner(h: &mut Harness) {
    let pane = h.cx.update(|cx| h.child.read(cx).pane().unwrap().clone());
    h.cx.update(|cx| pane.update(cx, |pane, cx| pane.set_offline_banner(true, cx)));
}

fn shows_clipboard_prompt(h: &mut Harness) -> bool {
    h.draw();
    h.cx.update_window(h.window, |_, window, _| {
        window.try_find("clipboard-read-prompt").is_some()
    })
    .unwrap()
}

/// D30: a reconnect that sends again output the view already has never
/// asks about a clipboard read in it again; only a read in new output asks.
#[test]
fn only_a_clipboard_read_in_new_output_asks() {
    let mut h = Harness::new(None);
    h.live(&Terminal::new(80, 24, 1000).unwrap());
    let read = b"\x1b]52;c;?\x07".to_vec();
    let end = read.len() as u64;
    h.apply(ServerFrame::Output {
        id: 1,
        offset: 0,
        bytes: read.clone(),
    })
    .unwrap();
    assert!(shows_clipboard_prompt(&mut h), "a live read asks");
    h.cx.update_window(h.window, |_, window, cx| window.click("clipboard-refuse", cx))
        .unwrap();
    h.cx.run_until_parked();
    assert!(!shows_clipboard_prompt(&mut h));

    // The reconnect: a snapshot that already has the read, then the same
    // bytes again from before it.
    let mut caught_up = Terminal::new(80, 24, 1000).unwrap();
    caught_up.feed(&read);
    h.snapshot(&caught_up, end);
    h.apply(ServerFrame::Output {
        id: 1,
        offset: 0,
        bytes: read.clone(),
    })
    .unwrap();
    assert!(!shows_clipboard_prompt(&mut h), "output the view has never asks again");

    h.apply(ServerFrame::Output {
        id: 1,
        offset: end,
        bytes: read,
    })
    .unwrap();
    assert!(shows_clipboard_prompt(&mut h), "a read in new output asks");
}

fn shows_offline_banner(h: &mut Harness) -> bool {
    h.draw();
    h.cx.update_window(h.window, |_, window, _| {
        window.try_find("disconnected-banner").is_some()
    })
    .unwrap()
}

/// A program that ended is not a lost connection: the banner that says the
/// machine is out of reach and the program keeps running shows only while
/// the connection is lost.
#[test]
fn an_exited_program_does_not_read_as_a_lost_connection() {
    let terminal = Terminal::new(80, 24, 1000).unwrap();

    let mut lost = Harness::new(None);
    lost.live(&terminal);
    show_offline_banner(&mut lost);
    lost.cx
        .update(|cx| lost.child.update(cx, |child, cx| child.set_connected(false, 7, cx)))
        .unwrap();
    assert!(shows_offline_banner(&mut lost));

    let mut ended = Harness::new(None);
    ended.live(&terminal);
    show_offline_banner(&mut ended);
    ended.apply(ServerFrame::Exited { id: 1, code: Some(0) }).unwrap();
    assert!(!shows_offline_banner(&mut ended));
    // It still takes no typing.
    assert!(matches!(
        ended.inject("after", InjectionOptions::raw()),
        Err(ReplicaError::NotReady { .. })
    ));
}

#[test]
fn recreating_an_embedded_entity_preserves_sequence_highwater_without_replaying_pending_input() {
    let mut h = Harness::new(None);
    let terminal = Terminal::new(80, 24, 1000).unwrap();
    h.live(&terminal);
    let pending = h
        .inject("uncertain command", InjectionOptions::raw())
        .unwrap();
    let checkpoint =
        h.cx.update(|cx| h.child.read(cx).checkpoint(16 << 20, cx).unwrap());
    h.sink.frames.borrow_mut().clear();
    let sink = h.sink.clone();
    let replacement =
        h.cx.update_window(h.window, |_, window, cx| {
            TerminalReplica::create(
                ReplicaConfig {
                    session: info(),
                    theme: Theme::default(),
                    memory_saver: None,
                    shortcuts: ShortcutPolicy::default(),
                    fit: FitPolicy::default(),
                    default_colors: None,
                },
                sink,
                window,
                cx,
            )
            .unwrap()
        })
        .unwrap();
    h.cx.update_window(h.window, |_, window, cx| {
        replacement.update(cx, |child, cx| {
            child.restore_checkpoint(&checkpoint, window, cx).unwrap()
        })
    })
    .unwrap();
    h.cx.update(|cx| {
        h.root.update(cx, |root, cx| {
            root.child = replacement.clone();
            cx.notify();
        })
    });
    h.child = replacement;
    h.draw();
    assert!(h.sink.frames.borrow().is_empty());
    assert!(matches!(
        h.inject("not fresh", InjectionOptions::raw()),
        Err(ReplicaError::NotReady { .. })
    ));
    h.bind();
    h.apply(ServerFrame::Control {
        id: 1,
        controller: true,
    })
    .unwrap();
    h.snapshot(&terminal, 0);
    h.sink.frames.borrow_mut().clear();
    let next = h.inject("new text", InjectionOptions::raw()).unwrap();
    assert_eq!(next.seq, pending.seq + 1);
    assert!(
        matches!(h.sink.frames.borrow().as_slice(),[ClientFrame::Input{seq:2,bytes,..}]if bytes==b"new text")
    );
}
#[test]
fn supplied_provider_and_terminal_bell_reach_embedder_without_process_exit() {
    let mut h = Harness::new(None);
    let terminal = Terminal::new(80, 24, 1000).unwrap();
    h.live(&terminal);
    h.apply(ServerFrame::Output {
        id: 1,
        offset: 0,
        bytes: vec![7],
    })
    .unwrap();
    let event = terminal_provider::ProviderEvent {
        schema: "fixture/v1".into(),
        provider: "fixture".into(),
        session_id: "external-session".into(),
        cwd: None,
        signal: terminal_provider::ProviderSignal::SessionEnded,
    };
    h.cx.update(|cx| {
        h.child
            .update(cx, |child, cx| child.provider_event(event, cx))
    });
    h.cx.run_until_parked();
    assert!(
        h.seen
            .borrow()
            .iter()
            .any(|event| matches!(event, Seen::Bell))
    );
    assert!(
        h.seen
            .borrow()
            .iter()
            .any(|event| matches!(event, Seen::Provider))
    );
    assert_eq!(
        h.cx.update(|cx| h.child.read(cx).state().phase),
        ReplicaPhase::Live
    );
}
#[test]
fn opt_in_host_shortcut_is_scoped_to_focused_terminal_and_bubbles_outside_it() {
    let mut h = Harness::with_shortcuts(
        None,
        ShortcutPolicy {
            new_terminal: true,
            ..Default::default()
        },
    );
    let terminal = Terminal::new(80, 24, 1000).unwrap();
    h.live(&terminal);
    h.cx.update_window(h.window, |_, window, cx| {
        window.activate_window();
        h.child
            .update(cx, |child, cx| child.focus_input(window, cx));
        window.render_frame(cx);
    })
    .unwrap();
    h.cx.run_until_parked();
    h.draw();
    h.cx.update_window(h.window, |_, window, cx| {
        let key = if cx.platform_info().is_mac() {
            "cmd-t"
        } else {
            "ctrl-t"
        };
        window.press(key, cx);
    })
    .unwrap();
    h.cx.run_until_parked();
    assert!(
        h.seen
            .borrow()
            .iter()
            .any(|event| matches!(event, Seen::Shortcut(HostAction::NewTerminal)))
    );
    assert_eq!(h.outside.get(), 0);
    assert!(
        !h.sink
            .frames
            .borrow()
            .iter()
            .any(|frame| matches!(frame, ClientFrame::Input { .. }))
    );
    h.cx.update_window(h.window, |_, window, cx| {
        let outside = h.root.read(cx).outside_focus.clone();
        window.focus(&outside, cx);
        window.render_frame(cx);
        window.dispatch_action(Box::new(NewTerminal), cx);
    })
    .unwrap();
    h.cx.run_until_parked();
    assert_eq!(h.outside.get(), 1);
}

#[test]
fn focused_pane_paste_reports_oversized_and_full_refusal_without_consuming_input_sequence() {
    let mut h = Harness::new(None);
    let mut terminal = Terminal::new(80, 24, 1000).unwrap();
    terminal.feed(b"\x1b[?2004h");
    h.live(&terminal);
    for text in ["x".repeat(MAX_INPUT_BYTES), "small paste".into()] {
        h.sink.reject.set(text.len() < MAX_INPUT_BYTES);
        h.cx.update(|cx| {
            let pane = h.child.read(cx).pane().unwrap().clone();
            pane.update(cx, |pane, cx| pane.paste_text(text, cx));
        });
        h.cx.run_until_parked();
        assert!(
            !h.sink
                .frames
                .borrow()
                .iter()
                .any(|frame| matches!(frame, ClientFrame::Input { .. }))
        );
        assert_eq!(h.cx.update(|cx| h.child.read(cx).state().input_seq), 0);
    }
    h.sink.reject.set(false);
    assert_eq!(h.inject("fresh", InjectionOptions::raw()).unwrap().seq, 1);
}

#[test]
fn programmatic_injection_cannot_overtake_pending_paste_confirmation() {
    let mut h = Harness::new(None);
    let terminal = Terminal::new(80, 24, 1000).unwrap();
    h.live(&terminal);
    h.cx.update(|cx| {
        let pane = h.child.read(cx).pane().unwrap().clone();
        pane.update(cx, |pane, cx| pane.paste_text("one\ntwo".into(), cx));
    });
    h.cx.run_until_parked();
    assert!(
        matches!(h.inject("not yet",InjectionOptions{submit:true,..InjectionOptions::raw()}),Err(ReplicaError::PendingPaste{terminal_uuid}) if terminal_uuid==info().uuid)
    );
    assert_eq!(h.cx.update(|cx| h.child.read(cx).state().input_seq), 0);
    assert!(
        !h.sink
            .frames
            .borrow()
            .iter()
            .any(|frame| matches!(frame, ClientFrame::Input { .. }))
    );
    h.cx.update(|cx| {
        let pane = h.child.read(cx).pane().unwrap().clone();
        pane.update(cx, |pane, cx| pane.cancel_paste(cx));
    });
    assert_eq!(h.inject("fresh", InjectionOptions::raw()).unwrap().seq, 1);
}

/// A pane with no replica around it, as an app running its own PTY shows it.
struct Bare(Entity<TerminalPane>);
impl Render for Bare {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.0.clone())
    }
}
struct Still;
impl Pty for Still {
    fn write(&mut self, _: &[u8]) {}
    fn resize(&mut self, _: ghostty_vt::Size, _: CellSize) {}
}
fn screen() -> Terminal {
    let mut terminal = Terminal::new(80, 24, 1000).unwrap();
    terminal.feed(
        b"\x1b[1;32muser@host\x1b[0m:~$ ls -l\r\n\x1b[34mdrwxr-xr-x\x1b[0m src\r\n-rw-r--r-- Cargo.toml\r\n$ ",
    );
    terminal
}
fn picture(
    cx: &mut gpui_kit::HeadlessAppContext,
    window: AnyWindowHandle,
) -> humata2_engine::frame::Picture {
    cx.update_window(window, |_, window, _| humata2_engine::frame::pixels(window))
        .unwrap()
        .unwrap()
}
fn resizes(h: &Harness) -> Vec<(u16, u16, u16, u16)> {
    h.sink
        .frames
        .borrow()
        .iter()
        .filter_map(|frame| match frame {
            ClientFrame::Resize {
                cols,
                rows,
                cell_width,
                cell_height,
                ..
            } => Some((*cols, *rows, *cell_width, *cell_height)),
            _ => None,
        })
        .collect()
}

#[test]
fn a_controlling_replica_draws_the_same_pixels_as_a_bare_pane() {
    let mut h = Harness::new(None);
    h.bind();
    h.apply(ServerFrame::Control {
        id: 1,
        controller: true,
    })
    .unwrap();
    h.snapshot(&screen(), 0);
    h.draw();
    // The replica keeps the host's grid until the host answers its Resize.
    let (cols, rows, cell_width, cell_height) = resizes(&h)
        .pop()
        .expect("a controlling replica asks the host to fit its bounds");
    h.apply(ServerFrame::Resized {
        id: 1,
        cols,
        rows,
        cell_width,
        cell_height,
    })
    .unwrap();
    h.draw();

    let (bare, _bare_root): (AnyWindowHandle, _) = h
        .cx
        .update(|cx| {
            gpui_kit::open_window(options(), cx, |window, cx| {
                let pane = cx.new(|cx| TerminalPane::new(screen(), Still, theme(), window, cx));
                cx.new(|_| Bare(pane))
            })
        })
        .unwrap();
    for _ in 0..3 {
        h.cx.update_window(bare, |_, window, cx| window.render_frame(cx))
            .unwrap();
        h.cx.run_until_parked();
    }
    h.draw();

    let replica = picture(&mut h.cx, h.window);
    let bare = picture(&mut h.cx, bare);
    assert_eq!((replica.width, replica.height), (bare.width, bare.height));
    let differing = replica
        .rgba
        .chunks(4)
        .zip(bare.rgba.chunks(4))
        .filter(|(replica, bare)| replica != bare)
        .count();
    assert_eq!(differing, 0, "pixels that differ from a bare pane");
}

#[test]
fn a_replica_that_fits_only_when_focused_asks_for_its_size_once_focused() {
    let mut config = config(None, ShortcutPolicy::default());
    config.fit = FitPolicy::Focused;
    let mut h = Harness::configured(config);
    h.bind();
    h.apply(ServerFrame::Control {
        id: 1,
        controller: true,
    })
    .unwrap();
    h.snapshot(&Terminal::new(80, 24, 1000).unwrap(), 0);
    h.draw();
    assert!(
        resizes(&h).is_empty(),
        "an unfocused view leaves the size to the view in focus"
    );

    h.cx.update_window(h.window, |_, window, cx| {
        window.activate_window();
        h.child
            .update(cx, |child, cx| child.focus_input(window, cx));
        window.render_frame(cx);
        window.render_frame(cx);
    })
    .unwrap();
    h.cx.run_until_parked();
    h.draw();
    assert!(
        !resizes(&h).is_empty(),
        "a focused view asks the host to fit its bounds"
    );
}

/// D271 point 3: a view that fits when shown sizes the terminal when it first
/// shows it and again while it has focus.
#[test]
fn a_replica_that_fits_when_shown_sizes_once_then_again_with_focus() {
    let mut config = config(None, ShortcutPolicy::default());
    config.fit = FitPolicy::Shown;
    let mut h = Harness::configured(config);
    h.bind();
    h.apply(ServerFrame::Control {
        id: 1,
        controller: true,
    })
    .unwrap();
    h.snapshot(&Terminal::new(80, 24, 1000).unwrap(), 0);
    h.draw();
    let first = resizes(&h);
    assert!(!first.is_empty(), "a view first showing the terminal sizes it");

    h.cx.update_window(h.window, |_, window, cx| {
        window.resize(size(px(400.), px(300.)));
        window.bounds_changed(cx);
    })
    .unwrap();
    h.draw();

    h.cx.update_window(h.window, |_, window, cx| {
        window.activate_window();
        h.child
            .update(cx, |child, cx| child.focus_input(window, cx));
        window.render_frame(cx);
        window.render_frame(cx);
    })
    .unwrap();
    h.cx.run_until_parked();
    h.draw();
    let now = resizes(&h);
    assert!(now.len() > first.len(), "the focused view fits its bounds");
    assert_ne!(now.last(), first.last(), "it fits the new bounds");
}

/// A side panel opening narrows an unfocused, visible view: it fits its new
/// bounds instead of drawing past its right edge (D750).
#[test]
fn an_unfocused_shown_replica_refits_when_its_own_bounds_change() {
    let mut config = config(None, ShortcutPolicy::default());
    config.fit = FitPolicy::Shown;
    let mut h = Harness::configured(config);
    h.bind();
    h.apply(ServerFrame::Control {
        id: 1,
        controller: true,
    })
    .unwrap();
    h.snapshot(&Terminal::new(80, 24, 1000).unwrap(), 0);
    h.draw();
    let first = resizes(&h);
    let (cols, ..) = *first.last().expect("a view first showing the terminal sizes it");

    h.cx.update_window(h.window, |_, window, cx| {
        window.resize(size(px(400.), px(300.)));
        window.bounds_changed(cx);
    })
    .unwrap();
    h.draw();
    let now = resizes(&h);
    let (narrowed, ..) = *now.last().expect("resizes");
    assert!(
        narrowed < cols,
        "the narrowed view fits its new bounds: {narrowed} columns, was {cols}"
    );
}

/// D271 point 3: when another view sizes the terminal, an unfocused view
/// whose own bounds did not change leaves that size alone, so two views of
/// one terminal never take turns sizing it.
#[test]
fn an_unfocused_shown_replica_never_takes_back_a_size_another_view_set() {
    let mut config = config(None, ShortcutPolicy::default());
    config.fit = FitPolicy::Shown;
    let mut h = Harness::configured(config);
    h.bind();
    h.apply(ServerFrame::Control {
        id: 1,
        controller: true,
    })
    .unwrap();
    h.snapshot(&Terminal::new(80, 24, 1000).unwrap(), 0);
    h.draw();
    let first = resizes(&h);
    let (_, _, cell_width, cell_height) =
        *first.last().expect("a view first showing the terminal sizes it");

    // Another view sizes it: the host reports the size, then a fresh screen.
    h.apply(ServerFrame::Resized {
        id: 1,
        cols: 200,
        rows: 60,
        cell_width,
        cell_height,
    })
    .unwrap();
    h.draw();
    h.snapshot(&Terminal::new(200, 60, 1000).unwrap(), 0);
    h.draw();
    assert_eq!(resizes(&h), first, "the other view keeps the size it set");
}

fn background(h: &mut Harness) -> Rgb {
    h.cx.update(|cx| {
        let pane = h.child.read(cx).pane().unwrap().clone();
        let view = pane.read(cx).view().clone();
        view.read(cx).grid().colors().background
    })
}

#[test]
fn default_colors_survive_every_new_checkpoint() {
    let dark = Colors {
        background: Rgb::hex(0x101418),
        ..Colors::default()
    };
    let light = Colors {
        foreground: Rgb::hex(0x1a1a1a),
        background: Rgb::hex(0xfafafa),
        ..Colors::default()
    };
    let mut config = config(None, ShortcutPolicy::default());
    config.default_colors = Some(dark.clone());
    let mut h = Harness::configured(config);
    let terminal = Terminal::new(80, 24, 1000).unwrap();
    h.live(&terminal);
    assert_eq!(background(&mut h), dark.background);
    h.snapshot(&terminal, 0);
    h.draw();
    assert_eq!(background(&mut h), dark.background);

    h.cx.update(|cx| {
        h.child
            .update(cx, |child, cx| child.set_default_colors(Some(light.clone()), cx))
    })
    .unwrap();
    h.draw();
    assert_eq!(background(&mut h), light.background);
    h.snapshot(&terminal, 0);
    h.draw();
    assert_eq!(background(&mut h), light.background);
}
