//! Deterministic async completion boundary: no operating-system clipboard access.
use super::*;
use gpui_kit::gpui;
use gpui_kit::{ClipboardReadError, TestAppContext, WindowOptions};
use terminal_grid::FakeGrid;

#[derive(Debug, PartialEq, Eq)]
enum RecordedInput {
    Paste(Paste),
    Key(terminal_grid::Key, KeyAction),
    Text(String),
}
#[derive(Default)]
struct ClipboardLog {
    pastes: Vec<Paste>,
    errors: Vec<ClipboardReadError>,
    sequence: Vec<RecordedInput>,
    overflow: usize,
    guarded: bool,
    refused: bool,
}
struct Host(Rc<RefCell<ClipboardLog>>);
impl TerminalHost<FakeGrid> for Host {
    fn key(&mut self, _: &mut FakeGrid, event: KeyEvent<'_>) {
        self.0
            .borrow_mut()
            .sequence
            .push(RecordedInput::Key(event.key, event.action));
    }
    fn text(&mut self, _: &mut FakeGrid, text: &str) {
        self.0
            .borrow_mut()
            .sequence
            .push(RecordedInput::Text(text.into()));
    }
    fn paste_input_accepted(&self) -> bool {
        !self.0.borrow().refused
    }
    fn paste_is_pending(&self) -> bool {
        self.0.borrow().guarded
    }
    fn input_buffer_failed(&mut self, _: crate::InputBufferError) {
        self.0.borrow_mut().overflow += 1;
    }
    fn paste(&mut self, _: &mut FakeGrid, paste: Paste) {
        self.0
            .borrow_mut()
            .sequence
            .push(RecordedInput::Paste(paste.clone()));
        self.0.borrow_mut().pastes.push(paste);
    }
    fn clipboard_read_failed(&mut self, error: ClipboardReadError) {
        self.0.borrow_mut().errors.push(error);
    }
}
fn open(
    cx: &mut TestAppContext,
) -> (
    gpui_kit::AnyWindowHandle,
    gpui_kit::Entity<TerminalView<FakeGrid>>,
    Rc<RefCell<ClipboardLog>>,
) {
    let log = Rc::new(RefCell::new(ClipboardLog::default()));
    let host = Host(log.clone());
    let (window, view) = cx.update(|cx| {
        gpui_kit::init(cx);
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| {
                TerminalView::new(FakeGrid::new(40, 10), host, Theme::default(), window, cx)
            })
        })
        .unwrap()
    });
    cx.update_window(window, |_, window, cx| {
        window.activate_window();
        view.update(cx, |view, cx| view.focus_input(window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    (window, view, log)
}
#[gpui_kit::test]
fn asynchronous_clipboard_failures_reach_the_focused_host_with_typed_causes(
    cx: &mut TestAppContext,
) {
    let (window, view, log) = open(cx);
    let errors = vec![
        ClipboardReadError::Unavailable,
        ClipboardReadError::Denied("fixture permission refused".into()),
        ClipboardReadError::UnsupportedContent,
    ];
    cx.update_window(window, |_, window, cx| {
        for error in errors.clone() {
            view.update(cx, |view, cx| {
                let id = view.begin_clipboard_read(cx).unwrap();
                view.clipboard_read_completed(id, Err(error), window, cx)
            });
        }
    })
    .unwrap();
    assert_eq!(log.borrow().errors, errors);
    assert!(log.borrow().pastes.is_empty());
}
#[gpui_kit::test]
fn late_clipboard_completion_cannot_paste_or_report_errors_after_focus_moves(
    cx: &mut TestAppContext,
) {
    let (window, view, log) = open(cx);
    cx.update_window(window, |_, window, cx| {
        let id = view.update(cx, |view, cx| view.begin_clipboard_read(cx).unwrap());
        let outside = cx.focus_handle();
        window.focus(&outside, cx);
        view.update(cx, |view, cx| {
            view.clipboard_read_completed(
                id,
                Ok(Some(ClipboardItem::new_string("wrong recipient".into()))),
                window,
                cx,
            );
            view.clipboard_read_completed(id, Err(ClipboardReadError::Unavailable), window, cx);
        });
        assert!(log.borrow().pastes.is_empty());
        assert!(log.borrow().errors.is_empty());
        view.update(cx, |view, cx| {
            view.focus_input(window, cx);
            let id = view.begin_clipboard_read(cx).unwrap();
            view.clipboard_read_completed(
                id,
                Ok(Some(ClipboardItem::new_string("right recipient".into()))),
                window,
                cx,
            );
        });
    })
    .unwrap();
    assert_eq!(
        log.borrow().pastes,
        vec![Paste::Text("right recipient".into())]
    );
}

fn enter(
    view: &mut TerminalView<FakeGrid>,
    window: &mut Window,
    cx: &mut Context<TerminalView<FakeGrid>>,
) {
    let keystroke = gpui_kit::Keystroke::parse_with_os("enter", cx.platform_info().os).unwrap();
    view.key_down(
        &KeyDownEvent {
            keystroke: keystroke.clone(),
            is_held: false,
            prefer_character_input: false,
        },
        window,
        cx,
    );
    view.key_up(&KeyUpEvent { keystroke }, window, cx);
}
#[gpui_kit::test]
fn delayed_clipboard_then_immediate_enter_and_committed_text_keep_input_order(
    cx: &mut TestAppContext,
) {
    let (window, view, log) = open(cx);
    cx.update_window(window, |_, window, cx| {
        view.update(cx, |view, cx| {
            let id = view.begin_clipboard_read(cx).unwrap();
            enter(view, window, cx);
            EntityInputHandler::replace_text_in_range(view, None, "next", window, cx);
            assert!(log.borrow().sequence.is_empty());
            view.clipboard_read_completed(
                id,
                Ok(Some(ClipboardItem::new_string("command".into()))),
                window,
                cx,
            );
        })
    })
    .unwrap();
    assert_eq!(
        log.borrow().sequence,
        vec![
            RecordedInput::Paste(Paste::Text("command".into())),
            RecordedInput::Key(terminal_grid::Key::Enter, KeyAction::Press),
            RecordedInput::Key(terminal_grid::Key::Enter, KeyAction::Release),
            RecordedInput::Text("next".into())
        ]
    );
}
#[gpui_kit::test]
fn two_clipboard_reads_flush_in_gesture_order_even_when_the_second_finishes_first(
    cx: &mut TestAppContext,
) {
    let (window, view, log) = open(cx);
    cx.update_window(window, |_, window, cx| {
        view.update(cx, |view, cx| {
            let first = view.begin_clipboard_read(cx).unwrap();
            enter(view, window, cx);
            let second = view.begin_clipboard_read(cx).unwrap();
            enter(view, window, cx);
            view.clipboard_read_completed(
                second,
                Ok(Some(ClipboardItem::new_string("second".into()))),
                window,
                cx,
            );
            assert!(log.borrow().sequence.is_empty());
            view.clipboard_read_completed(
                first,
                Ok(Some(ClipboardItem::new_string("first".into()))),
                window,
                cx,
            );
        })
    })
    .unwrap();
    assert_eq!(
        log.borrow().sequence,
        vec![
            RecordedInput::Paste(Paste::Text("first".into())),
            RecordedInput::Key(terminal_grid::Key::Enter, KeyAction::Press),
            RecordedInput::Key(terminal_grid::Key::Enter, KeyAction::Release),
            RecordedInput::Paste(Paste::Text("second".into())),
            RecordedInput::Key(terminal_grid::Key::Enter, KeyAction::Press),
            RecordedInput::Key(terminal_grid::Key::Enter, KeyAction::Release)
        ]
    );
}
#[gpui_kit::test]
fn clipboard_failure_cancels_following_enter_without_executing_the_previous_line(
    cx: &mut TestAppContext,
) {
    let (window, view, log) = open(cx);
    cx.update_window(window, |_, window, cx| {
        view.update(cx, |view, cx| {
            let id = view.begin_clipboard_read(cx).unwrap();
            enter(view, window, cx);
            view.clipboard_read_completed(
                id,
                Err(ClipboardReadError::Denied("fixture denied".into())),
                window,
                cx,
            );
            assert!(!view.has_pending_clipboard());
        })
    })
    .unwrap();
    assert!(log.borrow().sequence.is_empty());
    assert_eq!(log.borrow().errors.len(), 1);
}
#[gpui_kit::test]
fn clipboard_barrier_canceled_by_blur_cannot_replay_after_refocus(cx: &mut TestAppContext) {
    let (window, view, log) = open(cx);
    let id = cx
        .update_window(window, |_, window, cx| {
            view.update(cx, |view, cx| {
                let id = view.begin_clipboard_read(cx).unwrap();
                enter(view, window, cx);
                id
            })
        })
        .unwrap();
    let outside = cx.update(|cx| cx.focus_handle());
    cx.update_window(window, |_, window, cx| window.focus(&outside, cx))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.focus_input(window, cx);
            let new = view.begin_clipboard_read(cx).unwrap();
            view.clipboard_read_completed(
                id,
                Ok(Some(ClipboardItem::new_string("stale".into()))),
                window,
                cx,
            );
            assert!(log.borrow().sequence.is_empty());
            view.clipboard_read_completed(
                new,
                Ok(Some(ClipboardItem::new_string("fresh".into()))),
                window,
                cx,
            );
        })
    })
    .unwrap();
    assert_eq!(log.borrow().pastes, vec![Paste::Text("fresh".into())]);
}
#[gpui_kit::test]
fn pending_clipboard_typeahead_is_bounded_and_dropped_atomically_on_overflow(
    cx: &mut TestAppContext,
) {
    let (window, view, log) = open(cx);
    cx.update_window(window, |_, window, cx| {
        view.update(cx, |view, cx| {
            let id = view.begin_clipboard_read(cx).unwrap();
            view.send_input(
                PendingInputEvent::Text("x".repeat(crate::pending_input::MAX_BYTES)),
                cx,
            );
            assert!(!view.has_pending_clipboard());
            view.clipboard_read_completed(
                id,
                Ok(Some(ClipboardItem::new_string("stale".into()))),
                window,
                cx,
            );
        })
    })
    .unwrap();
    assert!(log.borrow().sequence.is_empty());
    assert_eq!(log.borrow().overflow, 1);
}
#[gpui_kit::test]
fn buffered_enter_cannot_approve_a_paste_that_requires_confirmation(cx: &mut TestAppContext) {
    let (window, view, log) = open(cx);
    log.borrow_mut().guarded = true;
    cx.update_window(window, |_, window, cx| {
        view.update(cx, |view, cx| {
            let id = view.begin_clipboard_read(cx).unwrap();
            enter(view, window, cx);
            view.clipboard_read_completed(
                id,
                Ok(Some(ClipboardItem::new_string("one\ntwo".into()))),
                window,
                cx,
            );
        })
    })
    .unwrap();
    assert_eq!(log.borrow().sequence.len(), 1);
    assert_eq!(log.borrow().pastes.len(), 1);
}

#[gpui_kit::test]
fn refused_paste_cancels_buffered_enter_instead_of_submitting_an_existing_line(
    cx: &mut TestAppContext,
) {
    let (window, view, log) = open(cx);
    log.borrow_mut().refused = true;
    cx.update_window(window, |_, window, cx| {
        view.update(cx, |view, cx| {
            let id = view.begin_clipboard_read(cx).unwrap();
            enter(view, window, cx);
            view.clipboard_read_completed(
                id,
                Ok(Some(ClipboardItem::new_string("not accepted".into()))),
                window,
                cx,
            );
            assert!(!view.has_pending_clipboard());
        })
    })
    .unwrap();
    assert_eq!(
        log.borrow().sequence,
        vec![RecordedInput::Paste(Paste::Text("not accepted".into()))]
    );
}

#[gpui_kit::test]
fn refused_paste_does_not_leak_a_later_key_release_and_fresh_keys_still_work(
    cx: &mut TestAppContext,
) {
    let (window, view, log) = open(cx);
    log.borrow_mut().refused = true;
    cx.update_window(window, |_, window, cx| {
        view.update(cx, |view, cx| {
            let id = view.begin_clipboard_read(cx).unwrap();
            let keystroke =
                gpui_kit::Keystroke::parse_with_os("enter", cx.platform_info().os).unwrap();
            view.key_down(
                &KeyDownEvent {
                    keystroke: keystroke.clone(),
                    is_held: false,
                    prefer_character_input: false,
                },
                window,
                cx,
            );
            view.clipboard_read_completed(
                id,
                Ok(Some(ClipboardItem::new_string("refused".into()))),
                window,
                cx,
            );
            view.key_up(&KeyUpEvent { keystroke }, window, cx);
            assert_eq!(log.borrow().sequence.len(), 1);
            log.borrow_mut().sequence.clear();
            enter(view, window, cx);
        })
    })
    .unwrap();
    assert_eq!(
        log.borrow().sequence,
        vec![
            RecordedInput::Key(terminal_grid::Key::Enter, KeyAction::Press),
            RecordedInput::Key(terminal_grid::Key::Enter, KeyAction::Release)
        ]
    );
}

#[gpui_kit::test]
fn unfocused_key_release_never_reaches_the_previous_terminal(cx: &mut TestAppContext) {
    let (window, view, log) = open(cx);
    cx.update_window(window, |_, window, cx| {
        let outside = cx.focus_handle();
        window.focus(&outside, cx);
        view.update(cx, |view, cx| {
            let keystroke =
                gpui_kit::Keystroke::parse_with_os("enter", cx.platform_info().os).unwrap();
            view.key_up(&KeyUpEvent { keystroke }, window, cx);
            assert_eq!(view.input_stats().unfocused_key_ups, 1);
        });
    })
    .unwrap();
    assert!(log.borrow().sequence.is_empty());
}
