//! The embedded terminal: keys, focus and scrollback.
use super::*;

/// Whether the fake program of a tab has printed text ending in `tail`: what
/// the typed keys reached, and what the terminal then drew.
fn printed_ends_with(app: &App, key: &(SessionId, usize), tail: &[u8]) -> bool {
    app.fakes
        .daemon
        .workspace()
        .terminal_bytes
        .get(key)
        .is_some_and(|bytes| bytes.ends_with(tail))
}

/// SE-10/13/16: actual terminal keystrokes, find and resize survive tab switches.
#[test]
fn the_embedded_terminal_accepts_keys_and_keeps_its_scrollback() {
    let mut app = App::new(fixture::design());
    // A terminal sizes its program while it has focus, and headless windows
    // start inactive, which keeps focus callbacks quiet.
    app.cx
        .update_window(app.window, |_, window, _| window.activate_window())
        .unwrap();
    app.settle();
    app.open("ledger", Panel::Info);
    app.click("terminal-tab 1");
    app.click("terminal");
    app.cx
        .update_window(app.window, |_, window, cx| {
            for key in ["p", "w", "d", "enter"] {
                window.press(key, cx);
            }
        })
        .unwrap();
    app.settle();
    let key = (SessionId::new("ledger"), 1);
    assert!(printed_ends_with(&app, &key, b"~/humata\r\n$ "));
    let wide = app
        .runtime
        .block_on(app.ui.clients().daemon.load())
        .unwrap()
        .terminal_sizes[&key];
    app.click("terminal-tab 0");
    app.click("terminal-tab 1");
    app.click("terminal");
    app.cx
        .update_window(app.window, |_, window, cx| window.press("cmd-f", cx))
        .unwrap();
    app.settle();
    assert!(app.contains("find-bar"));
    app.cx
        .update_window(app.window, |_, window, cx| window.press("escape", cx))
        .unwrap();
    app.settle();
    assert!(!app.contains("find-bar"), "Escape closes the find bar");
    app.cx
        .update_window(app.window, |_, window, cx| {
            window.resize(size(px(720.), px(760.)));
            window.bounds_changed(cx);
        })
        .unwrap();
    app.settle();
    app.draw();
    app.settle();
    let narrow = app
        .runtime
        .block_on(app.ui.clients().daemon.load())
        .unwrap()
        .terminal_sizes[&key];
    assert!(
        narrow.0 < wide.0,
        "the focused tab fits a narrower window: {narrow:?}, was {wide:?}"
    );
    app.cx
        .update_window(app.window, |_, window, cx| {
            window.resize(size(px(1280.), px(760.)));
            window.bounds_changed(cx);
        })
        .unwrap();
    app.settle();
    app.draw();
    app.settle();
    let restored = app
        .runtime
        .block_on(app.ui.clients().daemon.load())
        .unwrap()
        .terminal_sizes[&key];
    assert_eq!(restored, wide);
    assert!(app.contains("terminal"));
    assert!(printed_ends_with(&app, &key, b"~/humata\r\n$ "));
}

fn terminal_size(app: &App, key: &(SessionId, usize)) -> (u16, u16) {
    app.runtime
        .block_on(app.ui.clients().daemon.load())
        .unwrap()
        .terminal_sizes[key]
}

/// D750 (reads D271 point 3): a shown terminal sizes its program to its own
/// bounds, in focus or not. A window that narrows while the palette has focus
/// narrows the shell at once, and focus coming back changes nothing. Never
/// taking back another view's size is covered in
/// terminal/client/tests/component.rs.
#[test]
fn a_terminal_fits_its_own_bounds_even_out_of_focus() {
    let mut app = App::new(fixture::design());
    app.cx
        .update_window(app.window, |_, window, _| window.activate_window())
        .unwrap();
    app.settle();
    app.open("ledger", Panel::Info);
    app.click("terminal-tab 1");
    app.click("terminal");
    let key = (SessionId::new("ledger"), 1);
    let wide = terminal_size(&app, &key);

    app.send(AppAction::OpenPalette);
    app.cx
        .update_window(app.window, |_, window, cx| {
            window.resize(size(px(720.), px(760.)));
            window.bounds_changed(cx);
        })
        .unwrap();
    app.settle();
    app.draw();
    app.settle();
    let narrow = terminal_size(&app, &key);
    assert!(narrow.0 < wide.0, "a terminal out of focus fits its own new bounds");

    press(&mut app, &["escape"]);
    app.draw();
    app.settle();
    assert_eq!(
        terminal_size(&app, &key),
        narrow,
        "focus coming back asks for no other size"
    );
}

/// SE-1/8/10: starting a session or selecting a tab immediately directs keyboard input there.
#[test]
fn new_sessions_and_selected_tabs_receive_input_without_an_extra_click() {
    use humata2_app::NewSessionAction as N;
    use humata2_clients::model::Agent;
    let mut app = App::new(fixture::design());
    // Headless windows start inactive; focus callbacks require activation.
    app.cx
        .update_window(app.window, |_, window, _| window.activate_window())
        .unwrap();
    app.settle();
    app.open("ledger", Panel::Info);
    app.click("terminal");
    app.click("terminal-tab 1");
    app.cx
        .update_window(app.window, |_, window, cx| {
            for key in ["p", "w", "d", "enter"] {
                window.press(key, cx);
            }
        })
        .unwrap();
    app.settle();
    assert!(printed_ends_with(
        &app,
        &(SessionId::new("ledger"), 1),
        b"~/humata\r\n$ "
    ));
    app.send(AppAction::Session(SessionAction::OpenTerminal {
        agent: Agent::Shell,
        split: false,
    }));
    let tab = app.ui.read(|s| s.session.view().unwrap().tab);
    app.cx
        .update_window(app.window, |_, window, cx| {
            for key in ["p", "w", "d", "enter"] {
                window.press(key, cx);
            }
        })
        .unwrap();
    app.settle();
    assert!(printed_ends_with(
        &app,
        &(SessionId::new("ledger"), tab),
        b"~/humata\r\n$ "
    ));
    app.send(AppAction::StartNewSession);
    app.send(AppAction::NewSession(N::Agent(Agent::Shell)));
    app.send(AppAction::NewSession(N::Title("Fresh terminal".into())));
    app.click("new-session-create");
    let id = app.ui.read(|s| s.session.open.clone()).unwrap();
    app.cx
        .update_window(app.window, |_, window, cx| {
            for key in ["p", "w", "d", "enter"] {
                window.press(key, cx);
            }
        })
        .unwrap();
    app.settle();
    assert!(printed_ends_with(&app, &(id, 0), b"~/humata\r\n$ "));
    app.click("session-fork");
    app.click("fork-kind-2");
    app.click("fork-create");
    let fork = app.ui.read(|s| s.session.open.clone()).unwrap();
    app.cx
        .update_window(app.window, |_, window, cx| {
            for key in ["p", "w", "d", "enter"] {
                window.press(key, cx);
            }
        })
        .unwrap();
    app.settle();
    assert!(printed_ends_with(&app, &(fork.clone(), 0), b"~/humata\r\n$ "));
    app.send(AppAction::OpenPalette);
    app.cx
        .update_window(app.window, |_, window, cx| window.press("escape", cx))
        .unwrap();
    app.settle();
    app.cx
        .update_window(app.window, |_, window, cx| {
            for key in ["l", "s", "enter"] {
                window.press(key, cx);
            }
        })
        .unwrap();
    app.settle();
    assert!(printed_ends_with(
        &app,
        &(fork, 0),
        b"notes.md  todo.txt\r\n$ "
    ));
}

/// SE-13: focus inside either grid determines the group that receives new tabs.
fn press(app: &mut App, keys: &[&str]) {
    app.cx
        .update_window(app.window, |_, window, cx| {
            for key in keys {
                window.press(key, cx);
            }
        })
        .unwrap();
    app.settle();
}

/// SE-16: a shell tab whose shell exits says so under its output, never
/// that the machine is out of reach, and what is typed after it reaches
/// nothing.
#[test]
fn a_shell_tab_whose_shell_exits_says_so_and_not_that_the_connection_was_lost() {
    let mut app = App::new(fixture::design());
    app.open("ledger", Panel::Info);
    app.click("terminal-tab 1");
    app.click("terminal");
    press(&mut app, &["e", "x", "i", "t", "enter"]);
    assert!(app.contains("terminal-exited"));
    assert!(!app.contains("disconnected-banner"));
    press(&mut app, &["l", "s", "enter"]);
    let key = (SessionId::new("ledger"), 1);
    assert!(!printed_ends_with(&app, &key, b"todo.txt\r\n$ "));
}

/// SE-16: when a tab's machine comes back, a line under its output says so
/// and how much it caught up, until the person types again.
#[test]
fn a_reconnected_tab_says_so_until_it_is_typed_into() {
    use humata2_clients::TerminalNotice;
    let mut app = App::new(fixture::design());
    app.open("ledger", Panel::Info);
    app.click("terminal-tab 1");
    app.click("terminal");
    app.fakes.daemon.terminal_notice(
        &SessionId::new("ledger"),
        1,
        TerminalNotice::Reconnected {
            caught_up_lines: 12,
        },
    );
    app.settle();
    assert!(app.contains("terminal-reconnected"));
    press(&mut app, &["a"]);
    assert!(!app.contains("terminal-reconnected"));
}

#[test]
fn clicking_a_split_terminal_focuses_that_group() {
    use humata2_clients::model::Agent;
    let mut app = App::new(fixture::design());
    // Headless windows start inactive; focus callbacks require activation.
    app.cx
        .update_window(app.window, |_, window, _| window.activate_window())
        .unwrap();
    app.settle();
    app.open("ledger", Panel::Info);
    app.send(AppAction::Session(SessionAction::OpenTerminal {
        agent: Agent::Codex,
        split: true,
    }));
    assert_eq!(app.ui.read(|s| s.session.view().unwrap().focused_group), 1);
    app.cx
        .update_window(app.window, |_, window, cx| {
            window.within("terminal-area").click("terminal", cx)
        })
        .unwrap();
    app.settle();
    assert_eq!(app.ui.read(|s| s.session.view().unwrap().focused_group), 0);
    app.click("terminal-secondary");
    assert_eq!(app.ui.read(|s| s.session.view().unwrap().focused_group), 1);
    app.send(AppAction::Session(SessionAction::OpenTerminal {
        agent: Agent::Shell,
        split: false,
    }));
    assert!(app.ui.read(|s| {
        s.session.view().unwrap().groups[1]
            .tabs
            .contains(&humata2_session::Tab::Terminal(3))
    }));
    assert_eq!(app.ui.read(|s| s.session.view().unwrap().groups.len()), 2);
}

/// SE-10: keyboard input follows focus; palette editing cannot type into the terminal.
const PASTE: &str = if cfg!(target_os = "macos") {
    "cmd-v"
} else {
    "ctrl-shift-v"
};

fn copy(app: &mut App, entry: gpui_kit::ClipboardEntry) {
    app.cx.update(|cx| {
        cx.write_to_clipboard(gpui_kit::ClipboardItem {
            entries: vec![entry],
        })
    });
}

fn toasts(app: &App) -> usize {
    app.ui.read(|state| state.toasts.len())
}

/// SE-16: an image pasted into a tab is saved on the session's machine and
/// the tab gets its path, as typed.
#[test]
fn a_pasted_image_types_the_path_the_machine_saved_it_at() {
    let mut app = App::new(fixture::design());
    app.open("ledger", Panel::Info);
    app.click("terminal-tab 1");
    app.click("terminal");
    copy(
        &mut app,
        gpui_kit::ClipboardEntry::Image(gpui_kit::Image::from_bytes(
            gpui_kit::ImageFormat::Png,
            b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec(),
        )),
    );
    press(&mut app, &[PASTE]);
    app.settle();
    let key = (SessionId::new("ledger"), 1);
    assert!(printed_ends_with(&app, &key, b".png"));
}

/// SE-16, D335: files pasted from Finder type their paths into a terminal
/// on this Mac. On another machine those paths don't exist, so the files
/// are uploaded into the session's pastes there and the paths typed are
/// the machine's.
#[test]
fn pasted_files_type_local_paths_here_and_uploaded_paths_on_another_machine() {
    let mut app = App::new(fixture::design());
    let files = || {
        gpui_kit::ClipboardEntry::ExternalPaths(gpui_kit::ExternalPaths(
            vec![
                std::path::PathBuf::from("/tmp/a b.png"),
                std::path::PathBuf::from("/tmp/notes.txt"),
            ]
            .into(),
        ))
    };
    // `refund` runs on this Mac in the design fixture.
    app.open("refund", Panel::Info);
    app.click("terminal-tab 0");
    app.click("terminal");
    copy(&mut app, files());
    press(&mut app, &[PASTE]);
    let here = (SessionId::new("refund"), 0);
    assert!(printed_ends_with(&app, &here, b"'/tmp/a b.png' /tmp/notes.txt"));

    app.open("ledger", Panel::Info);
    app.click("terminal-tab 1");
    app.click("terminal");
    copy(&mut app, files());
    press(&mut app, &[PASTE]);
    let there = (SessionId::new("ledger"), 1);
    assert!(printed_ends_with(&app, &there, b"notes.txt"));
    assert!(!printed_ends_with(&app, &there, b"/tmp/notes.txt"));
}

/// SE-16: typing into a tab that takes none says why, once, rather than
/// vanishing.
#[test]
fn typing_into_an_exited_shell_says_why_once() {
    let mut app = App::new(fixture::design());
    app.open("ledger", Panel::Info);
    app.click("terminal-tab 1");
    app.click("terminal");
    press(&mut app, &["e", "x", "i", "t", "enter"]);
    let before = toasts(&app);
    press(&mut app, &["l", "s", "enter"]);
    assert_eq!(toasts(&app), before + 1);
}

#[test]
fn an_unfocused_terminal_does_not_receive_palette_keystrokes() {
    let mut app = App::new(fixture::design());
    app.cx
        .update_window(app.window, |_, window, _| window.activate_window())
        .unwrap();
    app.settle();
    app.open("ledger", Panel::Info);
    app.click("terminal");
    let key = (SessionId::new("ledger"), 0);
    let before = app.fakes.daemon.workspace().terminal_bytes[&key].clone();
    app.send(AppAction::OpenPalette);
    app.cx
        .update_window(app.window, |_, window, cx| {
            window.input("focus stays in search", cx);
            window.press("down", cx);
        })
        .unwrap();
    app.settle();
    assert_eq!(app.fakes.daemon.workspace().terminal_bytes[&key], before);
    app.cx
        .update_window(app.window, |_, window, cx| window.press("escape", cx))
        .unwrap();
    app.settle();
    app.click("terminal");
    app.cx
        .update_window(app.window, |_, window, cx| {
            for key in ["p", "w", "d", "enter"] {
                window.press(key, cx);
            }
        })
        .unwrap();
    app.settle();
    assert_ne!(app.fakes.daemon.workspace().terminal_bytes[&key], before);
}

fn links_opened(app: &App, key: &(SessionId, usize)) -> usize {
    let call = format!("terminal link {} {}", key.0, key.1);
    app.fakes
        .daemon
        .calls()
        .iter()
        .filter(|made| **made == call)
        .count()
}

/// A Stop and Resume ends the tab's link and the app makes its terminal
/// again: the new one takes the keys the old one had, with no click.
#[test]
fn a_terminal_made_again_after_its_link_ends_takes_the_keys() {
    let mut app = App::new(fixture::design());
    app.cx
        .update_window(app.window, |_, window, _| window.activate_window())
        .unwrap();
    app.settle();
    app.open("ledger", Panel::Info);
    app.click("terminal");
    let key = (SessionId::new("ledger"), 0);
    let opened = links_opened(&app, &key);
    app.fakes.daemon.end_terminal_links(&key.0, key.1);
    app.settle();
    assert!(links_opened(&app, &key) > opened, "the tab opened a new link");
    let before = app.fakes.daemon.workspace().terminal_bytes[&key].clone();
    press(&mut app, &["p", "w", "d", "enter"]);
    assert_ne!(app.fakes.daemon.workspace().terminal_bytes[&key], before);
}

/// A press on the terminal's area, its margin around the screen included,
/// gives the program the keys.
#[test]
fn a_press_on_the_terminal_margin_gives_it_the_keys() {
    let mut app = App::new(fixture::design());
    app.cx
        .update_window(app.window, |_, window, _| window.activate_window())
        .unwrap();
    app.settle();
    app.open("ledger", Panel::Info);
    app.cx
        .update_window(app.window, |_, window, cx| {
            window.blur(cx);
            window.click_at("terminal-area", point(px(4.), px(4.)), cx);
        })
        .unwrap();
    app.settle();
    let key = (SessionId::new("ledger"), 0);
    let before = app.fakes.daemon.workspace().terminal_bytes[&key].clone();
    press(&mut app, &["p", "w", "d", "enter"]);
    assert_ne!(app.fakes.daemon.workspace().terminal_bytes[&key], before);
}

/// uxsimple 3-4: an agent opened from New agent tab beside the session's
/// own makes the tab row say how many agents are at work.
#[test]
fn an_agent_tab_beside_the_sessions_agent_shows_the_agent_count() {
    use humata2_clients::model::Agent;
    let mut app = App::new(fixture::design());
    app.open("ledger", Panel::Info);
    assert!(!app.contains("session-agent-count"), "one agent needs no count");
    app.send(AppAction::Session(SessionAction::OpenTerminal {
        agent: Agent::Copilot,
        split: false,
    }));
    app.settle();
    assert!(app.contains("session-agent-count"));
}
