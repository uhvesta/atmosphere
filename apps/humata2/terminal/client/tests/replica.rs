//! Shared widget/protocol behavior independent of any real socket or process host.
use ghostty_vt::{Grid, Terminal};
use gpui_kit::{
    AnyWindowHandle, Bounds, ClipboardItem, Entity, Focusable, ScrollDelta, WindowBounds,
    WindowOptions, point, prelude::*, px, size, test::TestWindowExt,
};
use std::{cell::RefCell, rc::Rc};
use terminal_client::{Assets, Options, RemoteWorkspace, SendError, Transport, TransportEvent};
use terminal_wire::{
    ClientFrame, HostInfo, PROTOCOL_VERSION, SNAPSHOT_ENGINE, ServerFrame, SessionInfo,
    SessionState,
};
#[derive(Default)]
struct SocketState {
    incoming: Vec<TransportEvent>,
    sent: Vec<String>,
    busy: bool,
}
struct MockSocket(Rc<RefCell<SocketState>>);
impl Transport for MockSocket {
    fn drain(&mut self) -> Vec<TransportEvent> {
        std::mem::take(&mut self.0.borrow_mut().incoming)
    }
    fn send(&mut self, text: String) -> Result<(), SendError> {
        self.0.borrow_mut().sent.push(text);
        Ok(())
    }
    fn reconnect(&mut self) {}
    fn is_idle(&self) -> bool {
        !self.0.borrow().busy
    }
}
fn frames(state: &Rc<RefCell<SocketState>>) -> Vec<ClientFrame> {
    std::mem::take(&mut state.borrow_mut().sent)
        .into_iter()
        .map(|s| serde_json::from_str(&s).unwrap())
        .collect()
}
fn snapshot(terminal: &Terminal, offset: u64) -> ServerFrame {
    let size = terminal.size();
    ServerFrame::Snapshot {
        id: 1,
        engine: SNAPSHOT_ENGINE.into(),
        bytes: terminal.checkpoint(4 * 1024 * 1024).unwrap(),
        offset,
        cols: size.cols,
        rows: size.rows,
        live: true,
        exited: None,
    }
}
fn receive(
    cx: &mut gpui_kit::HeadlessAppContext,
    app: &Entity<RemoteWorkspace>,
    frame: ServerFrame,
) {
    cx.update(|cx| app.update(cx, |app, cx| app.receive(frame, cx)));
    cx.run_until_parked();
}
fn draw(
    cx: &mut gpui_kit::HeadlessAppContext,
    window: AnyWindowHandle,
    app: &Entity<RemoteWorkspace>,
) {
    cx.update_window(window, |_, w, cx| {
        w.render_frame(cx);
        w.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| app.update(cx, |app, cx| app.poll(cx)));
}
fn text(cx: &mut gpui_kit::HeadlessAppContext, app: &Entity<RemoteWorkspace>) -> String {
    cx.update(|cx| {
        let pane = app.read(cx).pane(1).unwrap();
        let g = pane.read(cx).view().read(cx).grid();
        (0..g.size().rows)
            .filter_map(|y| g.row(y))
            .map(|row| {
                row.cells
                    .iter()
                    .map(|c| c.text.as_str())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    })
}
fn memory_fixture() -> (
    gpui_kit::HeadlessAppContext,
    AnyWindowHandle,
    Entity<RemoteWorkspace>,
    Rc<RefCell<SocketState>>,
) {
    let resources = humata2_engine::Resources::new(Assets)
        .font(include_bytes!("../assets/Inter-Regular.ttf"))
        .font(include_bytes!("../assets/JetBrainsMono-Regular.ttf"));
    let mut cx = humata2_engine::headless(&resources).unwrap();
    let socket = Rc::new(RefCell::new(SocketState {
        incoming: vec![TransportEvent::Opened],
        ..Default::default()
    }));
    let shared = socket.clone();
    let (window, app) = cx
        .update(|cx| {
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        point(px(0.), px(0.)),
                        size(px(980.), px(700.)),
                    ))),
                    show: false,
                    focus: false,
                    ..Default::default()
                },
                cx,
                |w, cx| {
                    cx.new(|cx| {
                        RemoteWorkspace::new(
                            MockSocket(shared),
                            Options {
                                token: "test-token".into(),
                                title: "memory".into(),
                                diagnostics: None,
                            },
                            w,
                            cx,
                        )
                    })
                },
            )
        })
        .unwrap();
    cx.update(|cx| app.update(cx, |app, cx| app.poll(cx)));
    receive(
        &mut cx,
        &app,
        ServerFrame::Sessions {
            host: HostInfo {
                label: "host".into(),
                os: "linux".into(),
                arch: "aarch64".into(),
                protocol_version: PROTOCOL_VERSION,
                engine: SNAPSHOT_ENGINE.into(),
            },
            sessions: (1..=3)
                .map(|id| SessionInfo {
                    id,
                    uuid: format!("00000000-0000-4000-8000-{id:012x}"),
                    title: format!("shell{id}"),
                    automatic_title: format!("shell{id}"),
                    name_override: None,
                    attributes: Default::default(),
                    cwd: "/remote".into(),
                    program: "sh".into(),
                    state: SessionState::Running,
                    recovery_available: false,
                })
                .collect(),
        },
    );
    for id in 1..=2 {
        receive(
            &mut cx,
            &app,
            ServerFrame::Control {
                id,
                controller: id == 1,
            },
        );
        let mut terminal = Terminal::new(60, 24, 10_000).unwrap();
        terminal.set_scrollback_limits(10_000, 32 << 20).unwrap();
        if id == 1 {
            for line in 0..12_000 {
                terminal.feed(
                    format!("{line:05} dense saved terminal history across tab eviction\r\n")
                        .as_bytes(),
                );
            }
            assert!(
                terminal.scrollback_rows() >= 9_500,
                "dense fixture retained only {} history rows",
                terminal.scrollback_rows()
            );
        }
        terminal.feed(format!("terminal{id}").as_bytes());
        let bytes = terminal.checkpoint(16 << 20).unwrap();
        if id == 1 {
            let before = terminal.scrollback_rows();
            let mut restored = Terminal::from_checkpoint(&bytes).unwrap();
            let immediate = restored.scrollback_rows();
            restored.feed(&[]);
            let after_empty = restored.scrollback_rows();
            restored.sync();
            eprintln!(
                "memory fixture history: before={before} decoded={immediate} empty_feed={after_empty} sync={}",
                restored.scrollback_rows()
            );
            assert_eq!(before, immediate);
            assert_eq!(before, after_empty);
            assert_eq!(before, restored.scrollback_rows());
        }
        receive(
            &mut cx,
            &app,
            ServerFrame::Snapshot {
                id,
                engine: SNAPSHOT_ENGINE.into(),
                bytes,
                offset: 9,
                cols: 60,
                rows: 24,
                live: true,
                exited: None,
            },
        );
    }
    draw(&mut cx, window, &app);
    frames(&socket);
    (cx, window, app, socket)
}
fn click_tab(
    cx: &mut gpui_kit::HeadlessAppContext,
    window: AnyWindowHandle,
    app: &Entity<RemoteWorkspace>,
    id: u64,
) {
    cx.update_window(window, |_, w, cx| w.click(format!("remote-tab-0-{id}"), cx))
        .unwrap();
    cx.run_until_parked();
    draw(cx, window, app);
}
#[test]
fn memory_saver_releases_only_idle_tabs_after_written_input_and_restores_fresh_state() {
    let (mut cx, window, app, socket) = memory_fixture();
    let old = cx.update(|cx| {
        let pane = app.read(cx).pane(1).unwrap();
        let retained = pane.read(cx).view().read(cx).grid().scrollback_rows();
        assert!(
            retained >= 4_500 && retained <= ghostty_vt::CLIENT_SCROLLBACK_MAX_LINES + 512,
            "restored dense replica should be bounded near the client cap (got {retained} rows)"
        );
        let weak = (pane.downgrade(), pane.read(cx).view().downgrade());
        pane.update(cx, |pane, cx| pane.paste_text("first".into(), cx));
        app.update(cx, |app, cx| {
            app.set_memory_saver(true, std::time::Duration::ZERO, cx)
        });
        weak
    });
    click_tab(&mut cx, window, &app, 3);
    cx.update(|cx| app.update(cx, |app, cx| app.poll(cx)));
    assert!(
        cx.update(|cx| app.read(cx).pane(1).is_some()),
        "input without a host written acknowledgement must pin its replica"
    );
    assert!(
        !frames(&socket)
            .iter()
            .any(|frame| matches!(frame, ClientFrame::Detach { id: 1 }))
    );
    receive(&mut cx, &app, ServerFrame::InputWritten { id: 1, seq: 1 });
    // Ordered Ack→Detach is safe even while another session's socket traffic
    // continues. The VT remains until the host's Detached acknowledgement.
    socket.borrow_mut().incoming.push(TransportEvent::Text(
        serde_json::to_string(&ServerFrame::Output {
            id: 2,
            offset: 9,
            bytes: b"ongoing visible output".to_vec(),
        })
        .unwrap(),
    ));
    // Another visible terminal can keep producing output/ACK traffic while
    // this inactive terminal parks after its own input and ACKs have drained.
    cx.update(|cx| app.update(cx, |app, cx| app.poll(cx)));
    draw(&mut cx, window, &app);
    let sent = frames(&socket);
    assert_eq!(
        sent.iter()
            .filter(|frame| matches!(frame, ClientFrame::Detach { id: 1 }))
            .count(),
        1
    );
    assert!(
        !sent
            .iter()
            .any(|frame| matches!(frame, ClientFrame::Stop { .. }))
    );
    assert!(cx.update(|cx| app.read(cx).pane(1).is_some()));
    assert!(
        old.0.upgrade().is_some() && old.1.upgrade().is_some(),
        "keep the VT until the ordered detach barrier"
    );
    let residency: serde_json::Value =
        cx.update(|cx| serde_json::from_str(&app.read(cx).inspect(cx)).unwrap());
    assert_eq!(residency["memory_saver"]["resident_count"], 2);
    assert_eq!(residency["memory_saver"]["parked_count"], 1);
    assert!(
        cx.update(|cx| app.read(cx).pane(2).is_some()),
        "a visible unfocused mirror must remain resident"
    );
    receive(
        &mut cx,
        &app,
        ServerFrame::Output {
            id: 1,
            offset: 9,
            bytes: b"stale".to_vec(),
        },
    );
    receive(
        &mut cx,
        &app,
        ServerFrame::ResyncRequired {
            id: 1,
            reason: "queued before detach".into(),
        },
    );
    let mut terminal = Terminal::new(60, 24, 1000).unwrap();
    terminal.feed(b"fresh checkpoint");
    receive(&mut cx, &app, snapshot(&terminal, 25));
    assert!(
        cx.update(|cx| app.read(cx).pane(1).is_some()),
        "a queued old snapshot must not replace a detaching replica"
    );
    click_tab(&mut cx, window, &app, 1);
    assert!(
        !frames(&socket)
            .iter()
            .any(|frame| matches!(frame, ClientFrame::Open { id: 1 })),
        "rapid wake must wait for the host detach barrier"
    );
    receive(&mut cx, &app, snapshot(&terminal, 25));
    assert!(
        cx.update(|cx| app.read(cx).pane(1).is_some()),
        "a late pre-detach snapshot must remain ignored even after refocus"
    );
    receive(&mut cx, &app, ServerFrame::Detached { id: 1 });
    draw(&mut cx, window, &app);
    assert!(
        old.0.upgrade().is_none() && old.1.upgrade().is_none(),
        "the barrier releases pane, VT and row caches"
    );
    cx.update(|cx| app.update(cx, |app, cx| app.poll(cx)));
    assert_eq!(
        frames(&socket)
            .iter()
            .filter(|frame| matches!(frame, ClientFrame::Open { id: 1 }))
            .count(),
        1
    );
    assert!(
        cx.update(|cx| app.read(cx).pane(1).is_none()),
        "waking cannot accept input until a fresh snapshot arrives"
    );
    receive(
        &mut cx,
        &app,
        ServerFrame::Control {
            id: 1,
            controller: true,
        },
    );
    receive(&mut cx, &app, snapshot(&terminal, 25));
    draw(&mut cx, window, &app);
    assert!(text(&mut cx, &app).contains("fresh checkpoint"));
    cx.update(|cx| {
        let pane = app.read(cx).pane(1).unwrap();
        pane.update(cx, |pane, cx| pane.paste_text("second".into(), cx));
        app.update(cx, |app, cx| app.poll(cx));
    });
    assert!(
        frames(&socket)
            .iter()
            .any(|frame| matches!(frame,ClientFrame::Input{id:1,seq:2,bytes} if bytes==b"second")),
        "waking must retain the input sequence highwater"
    );
}
#[test]
fn memory_saver_pins_search_selection_and_history_bookmarks_and_can_be_disabled() {
    let (mut cx, window, app, socket) = memory_fixture();
    cx.update_window(window, |_, w, cx| {
        let pane = app.read(cx).pane(1).unwrap();
        pane.update(cx, |pane, cx| pane.open_find(w, cx));
    })
    .unwrap();
    cx.update(|cx| {
        app.update(cx, |app, cx| {
            app.set_memory_saver(true, std::time::Duration::ZERO, cx)
        })
    });
    click_tab(&mut cx, window, &app, 3);
    cx.update(|cx| app.update(cx, |app, cx| app.poll(cx)));
    assert!(
        cx.update(|cx| app.read(cx).pane(1).is_some()),
        "search state must survive inactive-tab parking policy"
    );
    click_tab(&mut cx, window, &app, 1);
    cx.update_window(window, |_, w, cx| {
        let pane = app.read(cx).pane(1).unwrap();
        pane.update(cx, |pane, cx| {
            pane.close_find(w, cx);
            pane.view().clone().update(cx, |view, _| {
                view.grid_mut()
                    .select(
                        ghostty_vt::ViewportPoint { x: 0, y: 0 },
                        ghostty_vt::ViewportPoint { x: 3, y: 0 },
                    )
                    .unwrap()
            });
        });
    })
    .unwrap();
    click_tab(&mut cx, window, &app, 3);
    cx.update(|cx| app.update(cx, |app, cx| app.poll(cx)));
    assert!(
        cx.update(|cx| app.read(cx).pane(1).is_some()),
        "selected text must not be discarded"
    );
    cx.update(|cx| {
        let pane = app.read(cx).pane(1).unwrap();
        pane.update(cx, |pane, cx| {
            pane.view().clone().update(cx, |view, _| {
                let grid = view.grid_mut();
                grid.clear_selection().unwrap();
                grid.feed(&b"line\r\n".repeat(40));
                grid.scroll_viewport(ghostty_vt::Scroll::Top).unwrap();
            })
        });
    });
    draw(&mut cx, window, &app);
    cx.update(|cx| app.update(cx, |app, cx| app.poll(cx)));
    assert!(
        frames(&socket)
            .iter()
            .any(|frame| matches!(frame, ClientFrame::Detach { id: 1 })),
        "a scrolled-up hidden terminal must not permanently block parking"
    );
    receive(&mut cx, &app, ServerFrame::Detached { id: 1 });
    assert!(
        cx.update(|cx| app.read(cx).pane(1).is_none()),
        "parking must free the pane once Detached arrives"
    );
    // Disabling memory saver prevents hidden tabs from detaching.
    cx.update(|cx| {
        app.update(cx, |app, cx| {
            app.set_memory_saver(false, std::time::Duration::ZERO, cx)
        })
    });
    click_tab(&mut cx, window, &app, 1);
    let mut back = Terminal::new(60, 24, 1000).unwrap();
    back.feed(b"back\r\n");
    receive(&mut cx, &app, snapshot(&back, 20));
    frames(&socket);
    click_tab(&mut cx, window, &app, 3);
    draw(&mut cx, window, &app);
    cx.update(|cx| app.update(cx, |app, cx| app.poll(cx)));
    assert!(cx.update(|cx| app.read(cx).pane(1).is_some()));
    assert!(
        !frames(&socket)
            .iter()
            .any(|frame| matches!(frame, ClientFrame::Detach { id: 1 }))
    );
}
/// The memory saver waits out its whole delay on the test clock: a hidden
/// tab is kept a second short of it and freed once it has passed.
#[test]
fn memory_saver_frees_a_hidden_tab_only_once_its_delay_has_passed() {
    const DELAY: std::time::Duration = std::time::Duration::from_secs(30);
    let (mut cx, window, app, socket) = memory_fixture();
    cx.update(|cx| app.update(cx, |app, cx| app.set_memory_saver(true, DELAY, cx)));
    click_tab(&mut cx, window, &app, 3);
    draw(&mut cx, window, &app);
    let detached = |sent: Vec<ClientFrame>| {
        sent.iter()
            .any(|frame| matches!(frame, ClientFrame::Detach { id: 1 }))
    };

    cx.advance_clock(DELAY - std::time::Duration::from_secs(1));
    draw(&mut cx, window, &app);
    assert!(!detached(frames(&socket)), "the hidden tab was freed before its delay");
    assert!(cx.update(|cx| app.read(cx).pane(1).is_some()));

    cx.advance_clock(std::time::Duration::from_secs(1));
    draw(&mut cx, window, &app);
    assert!(detached(frames(&socket)), "the hidden tab was kept past its delay");
}

#[test]
fn memory_saver_keeps_a_waiting_multiline_paste_in_memory() {
    let (mut cx, window, app, socket) = memory_fixture();
    cx.update(|cx| {
        let pane = app.read(cx).pane(1).unwrap();
        pane.update(cx, |pane, cx| pane.paste_text("one\ntwo".into(), cx));
        assert!(pane.read(cx).has_pending_paste());
        app.update(cx, |app, cx| {
            app.set_memory_saver(true, std::time::Duration::ZERO, cx)
        });
    });
    click_tab(&mut cx, window, &app, 3);
    cx.update(|cx| app.update(cx, |app, cx| app.poll(cx)));
    cx.update(|cx| {
        let pane = app
            .read(cx)
            .pane(1)
            .expect("pending user confirmation must pin its pane");
        assert_eq!(pane.read(cx).pending_paste().unwrap().text, "one\ntwo");
    });
    assert!(!frames(&socket).iter().any(|frame| matches!(
        frame,
        ClientFrame::Input { .. } | ClientFrame::Detach { id: 1 }
    )));
}
#[test]
fn focused_terminal_owns_tab_and_shift_tab_before_root_focus_traversal() {
    let (mut cx, window, app, socket) = memory_fixture();
    cx.update_window(window, |_, w, cx| {
        let pane = app.read(cx).pane(1).unwrap();
        pane.update(cx, |pane, cx| pane.focus_input(w, cx));
    })
    .unwrap();
    draw(&mut cx, window, &app);
    frames(&socket);
    cx.update_window(window, |_, w, cx| {
        w.press("tab", cx);
        w.press("shift-tab", cx);
        let pane = app.read(cx).pane(1).unwrap();
        assert!(pane.read(cx).focus_handle(cx).is_focused(w));
    })
    .unwrap();
    cx.update(|cx| app.update(cx, |app, cx| app.poll(cx)));
    let bytes: Vec<_> = frames(&socket)
        .into_iter()
        .filter_map(|frame| match frame {
            ClientFrame::Input { id: 1, bytes, .. } => Some(bytes),
            _ => None,
        })
        .flatten()
        .collect();
    assert_eq!(bytes, b"\t\x1b[Z");
    cx.update_window(window, |_, w, cx| {
        let pane = app.read(cx).pane(1).unwrap();
        pane.update(cx, |pane, cx| pane.open_find(w, cx));
    })
    .unwrap();
    draw(&mut cx, window, &app);
    frames(&socket);
    cx.update_window(window, |_, w, cx| w.press("tab", cx))
        .unwrap();
    cx.update(|cx| app.update(cx, |app, cx| app.poll(cx)));
    assert!(
        !frames(&socket)
            .iter()
            .any(|frame| matches!(frame, ClientFrame::Input { .. })),
        "Tab in a find field must not leak to the PTY"
    );
    cx.update_window(window, |_, w, cx| {
        let pane = app.read(cx).pane(1).unwrap();
        pane.update(cx, |pane, cx| pane.close_find(w, cx));
    })
    .unwrap();
    draw(&mut cx, window, &app);
    frames(&socket);
    cx.update_window(window, |_, w, cx| {
        use gpui_kit::EntityInputHandler;
        let view = app.read(cx).pane(1).unwrap().read(cx).view().clone();
        w.click("remote-tab-0-3", cx);
        view.update(cx, |view, cx| {
            view.replace_text_in_range(None, "stale commit", w, cx)
        });
        assert!(!view.read(cx).focus_handle(cx).is_focused(w));
    })
    .unwrap();
    cx.update(|cx| app.update(cx, |app, cx| app.poll(cx)));
    assert!(
        !frames(&socket)
            .iter()
            .any(|frame| matches!(frame, ClientFrame::Input { .. })),
        "a loading tab must immediately blur the previous PTY before its checkpoint arrives"
    );
}
#[test]
fn background_snapshot_preserves_the_other_terminal_and_find_field_focus() {
    let (mut cx, window, app, socket) = memory_fixture();
    receive(
        &mut cx,
        &app,
        ServerFrame::Control {
            id: 2,
            controller: true,
        },
    );
    cx.update_window(window, |_, w, cx| {
        let pane = app.read(cx).pane(2).unwrap();
        pane.update(cx, |pane, cx| pane.focus_input(w, cx));
    })
    .unwrap();
    draw(&mut cx, window, &app);
    frames(&socket);
    let mut terminal = Terminal::new(60, 24, 1000).unwrap();
    terminal.feed(b"updated background");
    receive(&mut cx, &app, snapshot(&terminal, 100));
    cx.update_window(window, |_, w, cx| {
        let pane = app.read(cx).pane(2).unwrap();
        assert!(
            pane.read(cx).focus_handle(cx).is_focused(w),
            "an unsolicited background snapshot must not move terminal focus"
        );
    })
    .unwrap();
    cx.update_window(window, |_, w, cx| {
        let pane = app.read(cx).pane(2).unwrap();
        pane.update(cx, |pane, cx| pane.open_find(w, cx));
    })
    .unwrap();
    draw(&mut cx, window, &app);
    let focused = cx.update_window(window, |_, w, cx| w.focused(cx)).unwrap();
    receive(&mut cx, &app, snapshot(&terminal, 101));
    cx.update_window(window, |_, w, cx| {
        assert_eq!(
            w.focused(cx),
            focused,
            "background restoration must preserve an existing find/text field focus"
        )
    })
    .unwrap();
}
#[test]
fn metadata_sessions_do_not_resnapshot_or_discard_local_find_state() {
    let (mut cx, window, app, socket) = memory_fixture();
    let original = cx.update(|cx| app.read(cx).pane(1).unwrap().downgrade());
    cx.update_window(window, |_, w, cx| {
        let pane = app.read(cx).pane(1).unwrap();
        pane.update(cx, |pane, cx| {
            pane.set_find_query(
                terminal_pane::Query {
                    text: "local search draft".into(),
                    ..Default::default()
                },
                w,
                cx,
            )
        });
    })
    .unwrap();
    draw(&mut cx, window, &app);
    frames(&socket);
    let state: serde_json::Value =
        cx.update(|cx| serde_json::from_str(&app.read(cx).inspect(cx)).unwrap());
    let host = serde_json::from_value(state["host"].clone()).unwrap();
    let mut sessions: Vec<SessionInfo> = serde_json::from_value(state["sessions"].clone()).unwrap();
    sessions[0].title = "renamed by owner".into();
    sessions[0]
        .attributes
        .insert("agent".into(), "custom".into());
    receive(&mut cx, &app, ServerFrame::Sessions { host, sessions });
    cx.update(|cx| app.update(cx, |app, cx| app.poll(cx)));
    // Model a host faithfully answering any requested opens. Assertions are
    // about retained local state, rather than the number/order of requests.
    let terminal = Terminal::new(60, 24, 1000).unwrap();
    for frame in frames(&socket) {
        if let ClientFrame::Open { id: 1 } = frame {
            receive(&mut cx, &app, snapshot(&terminal, 100));
        }
    }
    cx.update(|cx| {
        assert!(
            original.upgrade().is_some(),
            "metadata updates must keep the existing pane entity"
        );
        let pane = app.read(cx).pane(1).unwrap();
        assert_eq!(
            pane.read(cx).find_status().unwrap().query.text,
            "local search draft"
        );
    });
}
#[test]
fn authoritative_replica_orders_output_resize_resync_and_never_duplicates_replies() {
    let resources = humata2_engine::Resources::new(Assets)
        .font(include_bytes!("../assets/Inter-Regular.ttf"))
        .font(include_bytes!("../assets/JetBrainsMono-Regular.ttf"));
    let mut cx = humata2_engine::headless(&resources).unwrap();
    let socket = Rc::new(RefCell::new(SocketState {
        incoming: vec![TransportEvent::Opened],
        sent: Vec::new(),
        busy: false,
    }));
    let shared = socket.clone();
    let (window, app) = cx
        .update(|cx| {
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        point(px(0.), px(0.)),
                        size(px(380.), px(440.)),
                    ))),
                    show: false,
                    focus: false,
                    ..Default::default()
                },
                cx,
                |w, cx| {
                    cx.new(|cx| {
                        RemoteWorkspace::new(
                            MockSocket(shared),
                            Options {
                                token: "test-token".into(),
                                title: "test".into(),
                                diagnostics: None,
                            },
                            w,
                            cx,
                        )
                    })
                },
            )
        })
        .unwrap();
    cx.update(|cx| app.update(cx, |app, cx| app.poll(cx)));
    assert!(matches!(
        frames(&socket).as_slice(),
        [ClientFrame::Authenticate {
            protocol: PROTOCOL_VERSION,
            ..
        }]
    ));
    receive(
        &mut cx,
        &app,
        ServerFrame::Sessions {
            host: HostInfo {
                label: "remote".into(),
                os: "linux".into(),
                arch: "aarch64".into(),
                protocol_version: PROTOCOL_VERSION,
                engine: SNAPSHOT_ENGINE.into(),
            },
            sessions: vec![SessionInfo {
                id: 1,
                uuid: "00000000-0000-4000-8000-000000000001".into(),
                title: "shell".into(),
                automatic_title: "shell".into(),
                name_override: None,
                attributes: Default::default(),
                cwd: "/remote/only".into(),
                program: "zsh".into(),
                state: SessionState::Running,
                recovery_available: false,
            }],
        },
    );
    receive(
        &mut cx,
        &app,
        ServerFrame::Control {
            id: 1,
            controller: true,
        },
    );
    let mut terminal = Terminal::new(80, 24, 1000).unwrap();
    terminal.feed(b"hello");
    receive(&mut cx, &app, snapshot(&terminal, 5));
    draw(&mut cx, window, &app);
    let grid_size = cx.update(|cx| {
        app.read(cx)
            .pane(1)
            .unwrap()
            .read(cx)
            .view()
            .read(cx)
            .grid()
            .size()
    });
    assert_eq!(
        grid_size,
        ghostty_vt::Size { cols: 80, rows: 24 },
        "fitting the browser/native window must request resize, not mutate authoritative VT state"
    );
    let sent = frames(&socket);
    assert!(
        sent.iter()
            .any(|f| matches!(f, ClientFrame::Resize { id: 1, .. }))
    );
    receive(
        &mut cx,
        &app,
        ServerFrame::Resized {
            id: 1,
            cols: 90,
            rows: 25,
            cell_width: 8,
            cell_height: 19,
        },
    );
    draw(&mut cx, window, &app);
    assert_eq!(
        cx.update(|cx| app
            .read(cx)
            .pane(1)
            .unwrap()
            .read(cx)
            .view()
            .read(cx)
            .grid()
            .size()),
        ghostty_vt::Size { cols: 90, rows: 25 }
    );
    receive(
        &mut cx,
        &app,
        ServerFrame::Output {
            id: 1,
            offset: 5,
            bytes: b" world".to_vec(),
        },
    );
    receive(
        &mut cx,
        &app,
        ServerFrame::Output {
            id: 1,
            offset: 5,
            bytes: b" world".to_vec(),
        },
    );
    draw(&mut cx, window, &app);
    assert!(text(&mut cx, &app).contains("hello world"));
    assert!(!text(&mut cx, &app).contains("world world"));
    frames(&socket);
    // A device-status query must not become a second PTY reply from a replica.
    receive(
        &mut cx,
        &app,
        ServerFrame::Output {
            id: 1,
            offset: 11,
            bytes: b"\x1b[5n".to_vec(),
        },
    );
    draw(&mut cx, window, &app);
    assert!(
        !frames(&socket)
            .iter()
            .any(|f| matches!(f, ClientFrame::Input { .. }))
    );
    cx.update_window(window, |_, w, cx| {
        w.activate_window();
        let pane = app.read(cx).pane(1).unwrap();
        w.focus(&pane.read(cx).focus_handle(cx), cx);
        w.press("a", cx);
    })
    .unwrap();
    draw(&mut cx, window, &app);
    assert!(
        frames(&socket)
            .iter()
            .any(|f| matches!(f,ClientFrame::Input{id:1,seq:1,bytes} if bytes==b"a"))
    );
    receive(
        &mut cx,
        &app,
        ServerFrame::Output {
            id: 1,
            offset: 50,
            bytes: b"gap".to_vec(),
        },
    );
    draw(&mut cx, window, &app);
    assert!(
        frames(&socket)
            .iter()
            .any(|f| matches!(f, ClientFrame::SnapshotRequest { id: 1 }))
    );
    cx.update_window(window, |_, w, cx| w.press("b", cx))
        .unwrap();
    draw(&mut cx, window, &app);
    assert!(
        !frames(&socket)
            .iter()
            .any(|f| matches!(f, ClientFrame::Input { .. })),
        "input is disabled until the output gap is repaired"
    );
    terminal.feed(b" recovered");
    receive(&mut cx, &app, snapshot(&terminal, 50));
    draw(&mut cx, window, &app);
    assert!(text(&mut cx, &app).contains("hello recovered"));
    // A read-only mirror may lay out at a different size, then take control.
    terminal.feed(b"\r\x1b[70GRIGHTEDGE");
    receive(&mut cx, &app, snapshot(&terminal, 60));
    draw(&mut cx, window, &app);
    receive(
        &mut cx,
        &app,
        ServerFrame::Control {
            id: 1,
            controller: false,
        },
    );
    frames(&socket);
    draw(&mut cx, window, &app);
    assert!(
        !frames(&socket)
            .iter()
            .any(|f| matches!(f, ClientFrame::Resize { .. })),
        "a read-only mirror must not resize the host"
    );
    let before: serde_json::Value =
        cx.update(|cx| serde_json::from_str(&app.read(cx).inspect(cx)).unwrap());
    let before = &before["terminals"][0];
    assert!(
        before["viewport"]["width"].as_f64().unwrap()
            > before["mirror_clip"]["width"].as_f64().unwrap(),
        "a narrow mirror retains the full shared grid in a scrollable viewport: viewport={} clip={}",
        before["viewport"],
        before["mirror_clip"]
    );
    cx.update_window(window, |_, w, cx| {
        w.scroll(
            "terminal-replica-pan",
            ScrollDelta::Pixels(point(px(-10000.), px(0.))),
            cx,
        )
    })
    .unwrap();
    draw(&mut cx, window, &app);
    let after: serde_json::Value =
        cx.update(|cx| serde_json::from_str(&app.read(cx).inspect(cx)).unwrap());
    assert!(
        after["terminals"][0]["mirror_clip"]["offset_x"]
            .as_f64()
            .unwrap()
            < 0.,
        "horizontal scrolling must reveal columns beyond the narrow mirror's edge"
    );
    let terminal = &after["terminals"][0];
    let right = terminal["viewport"]["x"].as_f64().unwrap()
        + terminal["viewport"]["width"].as_f64().unwrap();
    let visible_right = terminal["mirror_clip"]["x"].as_f64().unwrap()
        + terminal["mirror_clip"]["width"].as_f64().unwrap();
    assert!(
        right <= visible_right + 1.,
        "panning to the end exposes the shared grid's rightmost column"
    );
    assert!(terminal["text"].as_str().unwrap().contains("RIGHTEDGE"));
    cx.update_window(window, |_, w, cx| {
        w.scroll(
            "terminal-replica-pan",
            ScrollDelta::Pixels(point(px(0.), px(-10000.))),
            cx,
        )
    })
    .unwrap();
    draw(&mut cx, window, &app);
    let bottom: serde_json::Value =
        cx.update(|cx| serde_json::from_str(&app.read(cx).inspect(cx)).unwrap());
    let terminal = &bottom["terminals"][0];
    let bottom_edge = terminal["viewport"]["y"].as_f64().unwrap()
        + terminal["viewport"]["height"].as_f64().unwrap();
    let visible_bottom = terminal["mirror_clip"]["y"].as_f64().unwrap()
        + terminal["mirror_clip"]["height"].as_f64().unwrap();
    assert!(
        bottom_edge <= visible_bottom + 1.,
        "a read-only mirror must expose its last shared row as well as its last column"
    );

    assert!(
        !frames(&socket)
            .iter()
            .any(|f| matches!(f, ClientFrame::Resize { .. } | ClientFrame::Input { .. })),
        "view panning must not resize or type into the shared process"
    );
    receive(
        &mut cx,
        &app,
        ServerFrame::Control {
            id: 1,
            controller: true,
        },
    );
    draw(&mut cx, window, &app);
    assert!(
        frames(&socket)
            .iter()
            .any(|f| matches!(f,ClientFrame::Resize{id:1,cols,rows,..} if *cols!=80||*rows!=24)),
        "taking control must fit the new viewport even after its read-only layout"
    );
    assert_eq!(
        cx.update(|cx| app
            .read(cx)
            .pane(1)
            .unwrap()
            .read(cx)
            .view()
            .read(cx)
            .grid()
            .size()),
        ghostty_vt::Size { cols: 80, rows: 24 }
    );
    // Clipboard is a frontend operation even though the process host is remote.
    cx.update_window(window, |_, w, cx| {
        cx.write_to_clipboard(ClipboardItem::new_string("pasted".into()));
        let pane = app.read(cx).pane(1).unwrap();
        w.focus(&pane.read(cx).focus_handle(cx), cx);
        w.press("cmd-v", cx);
    })
    .unwrap();
    draw(&mut cx, window, &app);
    assert!(
        frames(&socket)
            .iter()
            .any(|f| matches!(f,ClientFrame::Input{id:1,bytes,..} if bytes==b"pasted"))
    );

    // Return from the find input and commit through the real platform handler
    // immediately, without an intervening test render or animation wait.
    cx.update(|cx| {
        cx.update_window(window, |_, w, cx| {
            let pane = app.read(cx).pane(1).unwrap();
            pane.update(cx, |pane, cx| pane.open_find(w, cx));
        })
        .unwrap()
    });
    draw(&mut cx, window, &app);
    frames(&socket);
    cx.update(|cx| {
        cx.update_window(window, |_, w, cx| {
            w.dispatch_event(
                gpui_kit::PlatformInput::KeyDown(gpui_kit::KeyDownEvent {
                    keystroke: gpui_kit::Keystroke::parse_with_os("escape", cx.platform_info().os)
                        .unwrap(),
                    is_held: false,
                    prefer_character_input: false,
                }),
                cx,
            );
        })
        .unwrap()
    });
    cx.update(|cx| {
        cx.update_window(window, |_, w, cx| {
            use gpui_kit::EntityInputHandler;
            let pane = app.read(cx).pane(1).unwrap();
            assert!(pane.read(cx).find_status().is_none());
            let view = pane.read(cx).view().clone();
            view.update(cx, |view, cx| {
                view.replace_and_mark_text_in_range(None, "pending", None, w, cx)
            });
            let mut key = gpui_kit::Keystroke::parse_with_os("z", cx.platform_info().os).unwrap();
            key.key_char = Some("z".into());
            w.dispatch_keystroke(key, cx);
        })
        .unwrap()
    });
    cx.update(|cx| app.update(cx, |app, cx| app.poll(cx)));
    assert!(
        frames(&socket)
            .iter()
            .any(|f| matches!(f,ClientFrame::Input{id:1,bytes,..} if bytes==b"z")),
        "the first IME commit after Escape must reach the terminal"
    );
}
