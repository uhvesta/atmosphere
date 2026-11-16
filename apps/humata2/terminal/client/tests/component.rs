//! Embedding proof: no RemoteWorkspace, network, resource installation or app chrome.
use ghostty_vt::Terminal;
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
use terminal_pane::PaneEvent;
use terminal_replica::{
    HostAction, NewTerminal, ReplicaConfig, ReplicaError, ReplicaEvent, ReplicaPhase, ReplicaSink,
    SendError, ShortcutPolicy, TerminalReplica, register_host_shortcuts,
};
use terminal_view::Theme;
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
                                TerminalReplica::new(
                                    ReplicaConfig {
                                        session: info(),
                                        theme: Theme {
                                            font_family: "JetBrains Mono".into(),
                                            ..Default::default()
                                        },
                                        memory_saver: idle,
                                        shortcuts,
                                    },
                                    sender,
                                    window,
                                    cx,
                                )
                                .unwrap()
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
