//! The embedded terminal: keys, focus and scrollback.
use super::*;

/// SE-10/13/16: actual terminal keystrokes, find and resize survive tab switches.
#[test]
fn the_embedded_terminal_accepts_keys_and_keeps_its_scrollback() {
    let mut app = App::new(fixture::design());
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
    assert!(
        app.ui
            .read(|s| s.session.terminal_bytes[&key].ends_with(b"~/humata\r\n$ "))
    );
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
    assert!(narrow.0 < wide.0);
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
    assert!(
        app.ui
            .read(|s| s.session.terminal_bytes[&key].ends_with(b"~/humata\r\n$ "))
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
    assert!(app.ui.read(|s| {
        s.session.terminal_bytes[&(SessionId::new("ledger"), 1)].ends_with(b"~/humata\r\n$ ")
    }));
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
    assert!(app.ui.read(|s| {
        s.session.terminal_bytes[&(SessionId::new("ledger"), tab)].ends_with(b"~/humata\r\n$ ")
    }));
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
    assert!(
        app.ui
            .read(|s| s.session.terminal_bytes[&(id, 0)].ends_with(b"~/humata\r\n$ "))
    );
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
    assert!(
        app.ui
            .read(|s| s.session.terminal_bytes[&(fork.clone(), 0)].ends_with(b"~/humata\r\n$ "))
    );
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
    assert!(
        app.ui
            .read(|s| s.session.terminal_bytes[&(fork, 0)].ends_with(b"notes.md  todo.txt\r\n$ "))
    );
}

/// SE-13: focus inside either grid determines the group that receives new tabs.
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
