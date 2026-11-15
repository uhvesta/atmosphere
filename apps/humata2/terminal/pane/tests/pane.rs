//! The pane over a real terminal in a headless window, driven through
//! real input: what the program is sent for keys, pastes, the mouse and
//! focus; selection and copy; the find bar; scrolling the history while
//! output streams; and what the pane tells the app.

mod support;

// `#[gpui_kit::test]` expands to paths under `gpui`.
use gpui_kit::gpui;
use gpui_kit::test::{ClickOptions, TestInput, TestWindowExt};
use gpui_kit::{
    ClipboardItem, ElementInputHandler, Modifiers, ModifiersChangedEvent, PlatformInput,
    ScrollDelta, TestAppContext, point, px, size,
};
use support::{Told, open, shortcut};
use terminal_pane::{Query, Script};

const COPY: &str = if cfg!(target_os = "macos") {
    "cmd-c"
} else {
    "ctrl-shift-c"
};
const PASTE: &str = if cfg!(target_os = "macos") {
    "cmd-v"
} else {
    "ctrl-shift-v"
};

fn find_shortcut() -> String {
    shortcut("cmd-f", "ctrl-shift-f")
}

#[gpui_kit::test]
fn typed_keys_reach_the_program_in_its_encoding(cx: &mut TestAppContext) {
    let pane = open(cx, 100);
    pane.frame(cx, |window, cx| {
        window.press("a", cx);
        window.press("shift-b", cx);
        window.press("enter", cx);
        window.press("ctrl-c", cx);
        window.press("up", cx);
        window.press("f5", cx);
    });
    assert_eq!(pane.sent(), b"aB\r\x03\x1b[A\x1b[15~".to_vec());

    // Application cursor keys, as `vim` and `less` ask for.
    pane.feed(cx, b"\x1b[?1h");
    pane.frame(cx, |window, cx| window.press("up", cx));
    assert_eq!(pane.sent(), b"\x1bOA".to_vec());
}

#[cfg(target_os = "macos")]
#[gpui_kit::test]
fn option_types_its_character_unless_it_is_set_to_act_as_alt(cx: &mut TestAppContext) {
    let pane = open(cx, 100);
    let keystroke = || gpui_kit::Keystroke {
        modifiers: Modifiers::alt(),
        key: "f".into(),
        key_char: Some("ƒ".into()),
    };
    pane.frame(cx, |window, cx| {
        window.dispatch_keystroke(keystroke(), cx);
    });
    assert_eq!(String::from_utf8(pane.sent()).as_deref(), Ok("ƒ"));

    pane.pane.update(cx, |pane, cx| {
        pane.view().update(cx, |view, _| {
            view.grid_mut().set_option_as_alt(ghostty_vt::OptionAsAlt::On);
        });
    });
    pane.frame(cx, |window, cx| {
        window.dispatch_keystroke(keystroke(), cx);
    });
    assert_eq!(pane.sent(), b"\x1bf".to_vec());
}

#[gpui_kit::test]
fn a_one_line_paste_is_sent_and_bracketed_when_asked(cx: &mut TestAppContext) {
    let pane = open(cx, 100);
    cx.write_to_clipboard(ClipboardItem::new_string("echo hi".to_owned()));
    pane.frame(cx, |window, cx| window.press(PASTE, cx));
    assert_eq!(pane.sent(), b"echo hi".to_vec());

    pane.feed(cx, b"\x1b[?2004h");
    cx.write_to_clipboard(ClipboardItem::new_string("one\ntwo".to_owned()));
    pane.frame(cx, |window, cx| window.press(PASTE, cx));
    // Bracketed, a multi-line paste needs no prompt: the program sees it
    // as one paste and runs nothing.
    assert_eq!(pane.sent(), b"\x1b[200~one\ntwo\x1b[201~".to_vec());
    assert!(pane.pane.read_with(cx, |pane, _| pane.pending_paste().is_none()));
}

#[gpui_kit::test]
fn a_multi_line_paste_waits_for_the_user(cx: &mut TestAppContext) {
    let pane = open(cx, 100);
    cx.write_to_clipboard(ClipboardItem::new_string("rm -rf build\nmake\n".to_owned()));
    pane.frame(cx, |window, cx| {
        window.press(PASTE, cx);
    });
    assert!(pane.sent().is_empty(), "nothing runs before the user agrees");
    pane.frame(cx, |window, cx| {
        assert!(window.try_find("paste-prompt").is_some());
        window.click("paste-cancel", cx);
    });
    assert!(pane.sent().is_empty());
    pane.frame(cx, |window, _| assert!(window.try_find("paste-prompt").is_none()));

    pane.frame(cx, |window, cx| window.press(PASTE, cx));
    pane.frame(cx, |window, cx| window.click("paste-confirm", cx));
    assert_eq!(pane.sent(), b"rm -rf build\rmake\r".to_vec());
    pane.frame(cx, |window, _| assert!(window.try_find("paste-prompt").is_none()));
}

#[gpui_kit::test]
fn dragging_selects_and_the_copy_shortcut_copies_the_text(cx: &mut TestAppContext) {
    let pane = open(cx, 100);
    pane.feed(cx, b"hello world\r\nsecond line");
    pane.frame(cx, |window, cx| {
        let from = pane.cell(window, 6, 0, cx);
        let to = pane.cell(window, 5, 1, cx);
        window.drag(from, to, cx);
    });
    pane.frame(cx, |window, cx| window.press(COPY, cx));
    let copied = cx.read_from_clipboard().and_then(|item| item.text());
    assert_eq!(copied.as_deref(), Some("world\nsecond"));
    assert!(pane.sent().is_empty(), "copying sends nothing");
}

#[gpui_kit::test]
fn a_double_click_copies_a_whole_path(cx: &mut TestAppContext) {
    let pane = open(cx, 100);
    pane.feed(cx, b"see ~/src/humata-2/OVERVIEW.md, then");
    pane.frame(cx, |window, cx| {
        let origin = window.find("terminal").bounds().origin;
        let at = pane.cell(window, 10, 0, cx) - origin;
        window.click_with_options("terminal", ClickOptions::new().with_offset(at).with_count(2), cx);
    });
    pane.frame(cx, |window, cx| window.press(COPY, cx));
    let copied = cx.read_from_clipboard().and_then(|item| item.text());
    assert_eq!(copied.as_deref(), Some("~/src/humata-2/OVERVIEW.md"));
}

#[gpui_kit::test]
fn programs_that_take_the_mouse_get_it_unless_shift_is_held(cx: &mut TestAppContext) {
    let pane = open(cx, 100);
    // Button tracking with SGR reports, as `htop` and `vim` ask for.
    pane.feed(cx, b"some text here\x1b[?1000h\x1b[?1006h");
    pane.frame(cx, |window, cx| {
        let at = pane.cell(window, 2, 0, cx);
        window.drag(at, at, cx);
    });
    let sent = String::from_utf8(pane.sent()).expect("reports are text");
    assert!(sent.starts_with("\x1b[<0;3;1M"), "{sent:?}");
    assert!(sent.ends_with("\x1b[<0;3;1m"), "{sent:?}");

    pane.frame(cx, |window, cx| {
        window.scroll("terminal", ScrollDelta::Lines(point(0., 1.)), cx);
    });
    let sent = String::from_utf8(pane.sent()).expect("reports are text");
    assert!(sent.starts_with("\x1b[<64;"), "the wheel is reported: {sent:?}");

    // With Shift held, a drag selects as usual.
    pane.frame(cx, |window, cx| {
        let from = pane.cell(window, 0, 0, cx);
        let to = pane.cell(window, 3, 0, cx);
        window.dispatch_event(
            PlatformInput::ModifiersChanged(ModifiersChangedEvent {
                modifiers: Modifiers::shift(),
                capslock: Default::default(),
            }),
            cx,
        );
        window.drag(from, to, cx);
    });
    let sent = pane.sent();
    assert!(sent.is_empty(), "{:?}", String::from_utf8_lossy(&sent));
    pane.frame(cx, |window, cx| window.press(COPY, cx));
    let copied = cx.read_from_clipboard().and_then(|item| item.text());
    assert_eq!(copied.as_deref(), Some("some"));
}

#[gpui_kit::test]
fn programs_that_ask_are_told_when_the_pane_gains_and_loses_focus(cx: &mut TestAppContext) {
    let pane = open(cx, 100);
    pane.feed(cx, b"\x1b[?1004h");
    pane.frame(cx, |window, cx| window.press(&find_shortcut(), cx));
    pane.frame(cx, |window, cx| window.press("escape", cx));
    assert_eq!(pane.sent(), b"\x1b[O\x1b[I".to_vec());
}

#[gpui_kit::test]
fn the_find_bar_finds_in_the_history_and_escape_gives_the_keyboard_back(cx: &mut TestAppContext) {
    let pane = open(cx, 1000);
    let mut output = String::new();
    for n in 0..60 {
        let word = if n % 20 == 5 { "Needle" } else { "hay" };
        output.push_str(&format!("line {n} {word}\r\n"));
    }
    pane.feed(cx, output.as_bytes());

    pane.frame(cx, |window, cx| window.press(&find_shortcut(), cx));
    pane.frame(cx, |window, cx| {
        assert!(window.try_find("find-bar").is_some());
        assert!(!pane.terminal_focused(window, cx), "the find input has the keyboard");
        window.input("needle", cx);
    });
    assert!(pane.sent().is_empty(), "typing in the find bar isn't sent");
    let status = pane.pane.read_with(cx, |pane, _| pane.find_status()).expect("find is open");
    assert_eq!(status.matches, 3);
    // It starts on the newest match.
    assert_eq!(status.current, Some(2));

    // Enter goes up toward older output, and wraps.
    pane.frame(cx, |window, cx| window.press("enter", cx));
    pane.frame(cx, |window, cx| window.press("enter", cx));
    let current = |cx: &mut TestAppContext| {
        pane.pane
            .read_with(cx, |pane, _| pane.find_status())
            .and_then(|status| status.current)
    };
    assert_eq!(current(cx), Some(0));
    pane.frame(cx, |window, cx| window.press("enter", cx));
    assert_eq!(current(cx), Some(2));
    pane.frame(cx, |window, cx| window.press("shift-enter", cx));
    assert_eq!(current(cx), Some(0));

    // Every match is highlighted; the one the bar is on stands out.
    let marks = pane.pane.read_with(cx, |pane, cx| pane.view().read(cx).marks().to_vec());
    assert_eq!(marks.len(), 3);
    assert_eq!(marks.iter().filter(|mark| mark.current).count(), 1);

    // Match case: "needle" no longer matches "Needle".
    pane.frame(cx, |window, cx| window.click("find-case", cx));
    let status = pane.pane.read_with(cx, |pane, _| pane.find_status()).expect("find is open");
    assert!(status.query.case_sensitive);
    assert_eq!(status.matches, 0);

    pane.frame(cx, |window, cx| window.press("escape", cx));
    pane.frame(cx, |window, cx| {
        assert!(window.try_find("find-bar").is_none());
        assert!(pane.terminal_focused(window, cx), "Escape gives the keyboard back");
    });
    let marks = pane.pane.read_with(cx, |pane, cx| pane.view().read(cx).marks().len());
    assert_eq!(marks, 0, "closing find clears its highlights");
    pane.frame(cx, |window, cx| window.press("x", cx));
    assert_eq!(pane.sent(), b"x".to_vec());
}

#[gpui_kit::test]
fn a_broken_regular_expression_says_so_and_finds_nothing(cx: &mut TestAppContext) {
    let pane = open(cx, 100);
    pane.feed(cx, b"error: 42\r\nwarning: 7\r\n");
    let query = |text: &str| Query {
        text: text.to_owned(),
        case_sensitive: false,
        regex: true,
    };
    pane.frame(cx, |window, cx| {
        pane.pane
            .update(cx, |pane, cx| pane.set_find_query(query(r"\w+: \d+"), window, cx));
    });
    let status = pane.pane.read_with(cx, |pane, _| pane.find_status()).expect("find is open");
    assert_eq!((status.matches, status.error.is_some()), (2, false));

    pane.frame(cx, |window, cx| {
        pane.pane
            .update(cx, |pane, cx| pane.set_find_query(query(r"(\d+"), window, cx));
    });
    let status = pane.pane.read_with(cx, |pane, _| pane.find_status()).expect("find is open");
    assert_eq!(status.matches, 0);
    assert!(status.error.is_some());
}

#[gpui_kit::test]
fn matches_follow_new_output_while_the_bar_is_open(cx: &mut TestAppContext) {
    let pane = open(cx, 100);
    pane.feed(cx, b"one TODO\r\n");
    pane.frame(cx, |window, cx| window.press(&find_shortcut(), cx));
    pane.frame(cx, |window, cx| window.input("todo", cx));
    pane.feed(cx, b"two TODO\r\nthree TODO\r\n");
    let status = pane.pane.read_with(cx, |pane, _| pane.find_status()).expect("find is open");
    assert_eq!(status.matches, 3);
}

fn scrollbar(pane: &support::Opened, cx: &mut TestAppContext) -> ghostty_vt::Scrollbar {
    pane.pane.read_with(cx, |pane, cx| {
        pane.view().read(cx).grid().scrollbar().expect("the terminal has a scrollbar")
    })
}

#[gpui_kit::test]
fn the_view_stays_where_the_user_scrolled_while_output_streams(cx: &mut TestAppContext) {
    let pane = open(cx, 1000);
    let lines = Script::agent_stream(200);
    for chunk in lines.chunks() {
        pane.pane.update(cx, |pane, cx| pane.feed(chunk, cx));
    }
    pane.frame(cx, |_, _| {});
    // At the bottom, the view follows the output.
    assert!(scrollbar(&pane, cx).at_bottom());

    pane.frame(cx, |window, cx| {
        window.scroll("terminal", ScrollDelta::Lines(point(0., 30.)), cx);
    });
    let scrolled = scrollbar(&pane, cx);
    assert!(!scrolled.at_bottom());
    assert!(pane.sent().is_empty(), "scrolling the history sends nothing");

    // More output doesn't move what the user is reading.
    for chunk in Script::agent_stream(50).chunks() {
        pane.pane.update(cx, |pane, cx| pane.feed(chunk, cx));
    }
    pane.frame(cx, |_, _| {});
    let after = scrollbar(&pane, cx);
    assert_eq!(after.offset, scrolled.offset);
    assert!(after.total > scrolled.total);

    // Typing goes back to the newest output.
    pane.frame(cx, |window, cx| window.press("q", cx));
    assert!(scrollbar(&pane, cx).at_bottom());
    assert_eq!(pane.sent(), b"q".to_vec());
}

#[gpui_kit::test]
fn on_the_alternate_screen_the_wheel_sends_arrows_when_asked(cx: &mut TestAppContext) {
    let pane = open(cx, 100);
    // `less` style: alternate screen with alternate scroll (1007).
    pane.feed(cx, b"\x1b[?1049h\x1b[?1007h");
    pane.frame(cx, |window, cx| {
        window.scroll("terminal", ScrollDelta::Lines(point(0., 2.)), cx);
    });
    assert_eq!(pane.sent(), b"\x1b[A\x1b[A".to_vec());
}

#[gpui_kit::test]
fn resizing_the_window_tells_the_program_the_new_size(cx: &mut TestAppContext) {
    let pane = open(cx, 100);
    let first = pane.pty.borrow().sizes.last().copied().expect("the pane sized the pty");
    cx.simulate_window_resize(pane.window, size(px(960.), px(600.)));
    pane.frame(cx, |_, _| {});
    pane.frame(cx, |_, _| {});
    let last = pane.pty.borrow().sizes.last().copied().expect("the pane sized the pty");
    assert!(last.cols > first.cols && last.rows > first.rows, "{first:?} then {last:?}");
    // The terminal took the size too: a line as wide as the pane fits on
    // one row.
    let line = "x".repeat(usize::from(last.cols));
    pane.feed(cx, line.as_bytes());
    assert_eq!(pane.screen(cx)[0], line);
}

#[gpui_kit::test]
fn titles_bells_and_clipboard_writes_are_told_to_the_app(cx: &mut TestAppContext) {
    let pane = open(cx, 100);
    // OSC 2 title, BEL, OSC 52 write of "hi" and an OSC 52 read.
    pane.feed(cx, b"\x1b]2;make test\x07\x07\x1b]52;c;aGk=\x07\x1b]52;c;?\x07");
    assert_eq!(
        pane.told(),
        vec![
            Told::Title("make test".to_owned()),
            Told::Bell,
            Told::ClipboardWrite("hi".to_owned()),
            Told::ClipboardRead,
        ]
    );
    // Nothing answers the read until the app does.
    assert!(pane.sent().is_empty());
}

#[gpui_kit::test]
fn while_disconnected_input_is_dropped_and_the_pane_says_so(cx: &mut TestAppContext) {
    let pane = open(cx, 100);
    pane.pane.update(cx, |pane, cx| pane.set_connected(false, cx));
    pane.frame(cx, |window, cx| {
        assert!(window.try_find("disconnected-banner").is_some());
        window.press("a", cx);
    });
    cx.write_to_clipboard(ClipboardItem::new_string("ls".to_owned()));
    pane.frame(cx, |window, cx| window.press(PASTE, cx));
    assert!(pane.sent().is_empty());

    pane.pane.update(cx, |pane, cx| pane.set_connected(true, cx));
    pane.frame(cx, |window, cx| {
        assert!(window.try_find("disconnected-banner").is_none());
        window.press("b", cx);
    });
    // What was typed while offline never arrives late.
    assert_eq!(pane.sent(), b"b".to_vec());
}

#[gpui_kit::test]
fn an_input_method_commits_its_text_to_the_program(cx: &mut TestAppContext) {
    let pane = open(cx, 100);
    let view = pane.pane.read_with(cx, |pane, _| pane.view().clone());
    pane.frame(cx, |window, cx| {
        let bounds = window.find("terminal").bounds();
        let mut input = TestInput::new(ElementInputHandler::new(bounds, view.clone()), window);
        input.compose_text(None, "に", None, cx);
        assert_eq!(view.read(cx).preedit(), Some("に"));
        input.commit_text("日本", cx);
    });
    assert_eq!(String::from_utf8(pane.sent()).as_deref(), Ok("日本"));
    assert_eq!(pane.pane.read_with(cx, |pane, cx| pane.view().read(cx).preedit().map(str::to_owned)), None);
}

#[gpui_kit::test]
fn tricky_output_lands_in_the_right_cells(cx: &mut TestAppContext) {
    let pane = open(cx, 100);
    pane.feed(cx, &Script::tricky_rendering().into_bytes());
    let screen = pane.screen(cx);
    assert_eq!(screen[0], "wide: 日本語の文字 | 한국어");
    assert!(screen[1].starts_with("emoji: 👍"), "{:?}", screen[1]);
    assert_eq!(screen[2], "combining: e\u{301} a\u{308} n\u{303}");
    assert_eq!(&screen[3..6], ["box: ┌──┬──┐", "     │  │  │", "     └──┴──┘"]);
    // A wide character takes two cells: six of them and a space put the
    // bar at column 19.
    let column = pane.pane.read_with(cx, |pane, cx| {
        use ghostty_vt::Grid as _;
        let terminal = pane.view().read(cx).grid();
        terminal
            .row(0)
            .and_then(|row| row.cells.iter().position(|cell| cell.text == "|"))
    });
    assert_eq!(column, Some(19));
    let (shape, blinking) = pane.pane.read_with(cx, |pane, cx| {
        use ghostty_vt::Grid as _;
        let style = pane.view().read(cx).grid().cursor_style();
        (style.shape, style.blinking)
    });
    assert_eq!((shape, blinking), (ghostty_vt::CursorShape::Bar, true));
}

#[gpui_kit::test]
fn a_full_screen_program_leaves_the_shell_as_it_was(cx: &mut TestAppContext) {
    let pane = open(cx, 100);
    pane.feed(cx, &Script::shell_session().into_bytes());
    let before = pane.screen(cx);
    pane.feed(cx, b"\x1b[?1049h\x1b[?25l\x1b[H\x1b[2J");
    pane.feed(cx, "\x1b[1;1H┌────────┐\x1b[2;1H│ editor │".as_bytes());
    assert_eq!(pane.screen(cx)[1], "│ editor │");
    pane.feed(cx, b"\x1b[?1049l\x1b[?25h");
    assert_eq!(pane.screen(cx), before);
    assert_eq!(pane.told(), vec![Told::Title("zsh".to_owned())]);
}

#[gpui_kit::test]
fn the_split_shortcut_asks_for_a_new_shell_and_reaches_no_program(cx: &mut TestAppContext) {
    let pane = open(cx, 100);
    pane.frame(cx, |window, cx| window.press(&shortcut("cmd-\\", "ctrl-shift-\\"), cx));
    assert_eq!(pane.told(), vec![Told::Split]);
    assert!(pane.sent().is_empty());
    // Ctrl+\ alone is the program's.
    pane.frame(cx, |window, cx| window.press("ctrl-\\", cx));
    assert!(pane.told().is_empty());
    assert_eq!(pane.sent(), b"\x1c".to_vec());
}

#[gpui_kit::test]
fn a_hidden_pane_draws_nothing_and_catches_up_when_shown(cx: &mut TestAppContext) {
    let pane = open(cx, 1_000);
    pane.feed(cx, b"before\r\n");
    pane.pane.update(cx, |pane, cx| pane.set_visible(false, cx));
    pane.frame(cx, |_, _| {});
    let frames = pane.pane.read_with(cx, |pane, cx| pane.view().read(cx).stats().frames);

    // The program goes on writing while nobody looks.
    for n in 0..50 {
        pane.feed(cx, format!("hidden line {n}\r\n").as_bytes());
    }
    pane.frame(cx, |window, _| assert!(window.try_find("terminal").is_none()));
    let drawn = pane.pane.read_with(cx, |pane, cx| pane.view().read(cx).stats().frames);
    assert_eq!(drawn, frames, "a hidden pane drew its terminal");
    assert_eq!(pane.screen(cx)[1], "", "a hidden pane parsed its output");

    pane.pane.update(cx, |pane, cx| pane.set_visible(true, cx));
    pane.frame(cx, |_, _| {});
    let screen = pane.screen(cx);
    let last = screen.iter().rposition(|row| !row.is_empty()).expect("the output is shown");
    assert_eq!(screen[last], "hidden line 49");
    // Typing reaches the program again.
    pane.frame(cx, |window, cx| window.press("a", cx));
    assert_eq!(pane.sent(), b"a".to_vec());
}
