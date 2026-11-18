//! D781: a question read off its agent's screen shows that live terminal
//! in the wide Inbox. It never takes focus by itself, keeps every key once
//! it has focus (Escape too), never sizes the tab, and gives focus back to
//! the list when the item goes.
use super::*;
use humata2_clients::Event;
use humata2_clients::model::{RequestId, RequestKind};

/// Copilot's folder-trust box at the bottom of a 100x24 screen.
const TRUST_BOX: &[u8] = b"\x1b[2J\x1b[20;1H Do you trust the files in this folder?\r\n \x1b[7m> 1. Yes, proceed\x1b[0m\r\n   2. No, exit\r\n";

/// The design workspace with every waiting request a question on its
/// session's agent tab, whose screen is the trust box.
fn workspace() -> Workspace {
    let mut workspace = fixture::design();
    let mut sessions = Vec::new();
    for request in workspace.requests.iter_mut().filter(|r| r.answer.is_none()) {
        request.kind = RequestKind::Question;
        request.choices = vec!["Yes, proceed".to_owned(), "No, exit".to_owned()];
        request.on_screen = Some(0);
        sessions.push(request.session.clone());
    }
    for session in sessions {
        workspace.terminal_sizes.insert((session.clone(), 0), (100, 24));
        workspace.terminal_bytes.insert((session, 0), TRUST_BOX.to_vec());
    }
    workspace
}

/// The Inbox open on its first item, in an active window, with the item's
/// terminal key and id.
fn inbox() -> (App, (SessionId, usize), RequestId) {
    let mut app = App::new(workspace());
    app.cx
        .update_window(app.window, |_, window, _| window.activate_window())
        .unwrap();
    app.settle();
    app.send(AppAction::Navigate(Route::Inbox));
    app.draw();
    app.settle();
    let Some(key) = app.ui.read(|state| state.inbox.terminal()) else {
        panic!("the open item is a question on its screen");
    };
    let Some(id) = app.ui.read(|state| {
        state
            .inbox
            .shown()
            .into_iter()
            .find(|r| r.session == key.0 && r.on_screen == Some(key.1))
            .map(|r| r.id.clone())
    }) else {
        panic!("the open item is shown");
    };
    (app, key, id)
}

fn printed(app: &App, key: &(SessionId, usize)) -> Vec<u8> {
    app.fakes
        .daemon
        .workspace()
        .terminal_bytes
        .get(key)
        .cloned()
        .unwrap_or_default()
}

fn press(app: &mut App, key: &str) {
    app.cx
        .update_window(app.window, |_, window, cx| window.press(key, cx))
        .unwrap();
    app.settle();
}

#[test]
fn the_open_item_shows_its_screen_and_leaves_focus_in_the_list() {
    let (mut app, key, _) = inbox();
    assert!(app.contains("inbox-terminal"));
    assert!(super::editor::text(&mut app).contains("Do you trust the files in this folder?"));
    let before = printed(&app, &key);
    press(&mut app, "x");
    assert_eq!(printed(&app, &key), before, "nothing typed reaches the program unasked");
}

#[test]
fn a_focused_terminal_keeps_escape_and_never_sizes_the_tab() {
    let (mut app, key, _) = inbox();
    app.click("inbox-terminal");
    press(&mut app, "escape");
    assert_eq!(app.ui.read(|s| s.route.clone()), Route::Inbox);
    assert!(app.contains("inbox-terminal"), "Escape went to the program");
    press(&mut app, "x");
    assert!(printed(&app, &key).ends_with(b"x"));
    let size = app
        .runtime
        .block_on(app.ui.clients().daemon.load())
        .unwrap()
        .terminal_sizes[&key];
    assert_eq!(size, (100, 24), "the Inbox draws the tab at its own size");
}

#[test]
fn tab_from_the_list_focuses_the_terminal() {
    let (mut app, key, _) = inbox();
    press(&mut app, "tab");
    press(&mut app, "x");
    assert!(printed(&app, &key).ends_with(b"x"));
}

#[test]
fn a_withdrawn_item_takes_its_terminal_and_gives_focus_back() {
    let (mut app, _, id) = inbox();
    app.click("inbox-terminal");
    app.fakes.daemon.emit(Event::RequestGone(id));
    app.settle();
    app.draw();
    app.settle();
    let shown = app.ui.read(|state| state.inbox.terminal());
    let focused = app
        .cx
        .update_window(app.window, |_, window, cx| window.focused(cx).is_some())
        .unwrap();
    assert!(focused, "the list has the keys again");
    match shown {
        None => assert!(!app.contains("inbox-terminal")),
        // The next item's terminal shows, but the keys stay in the list.
        Some(next) => {
            let before = printed(&app, &next);
            press(&mut app, "x");
            assert_eq!(printed(&app, &next), before);
        }
    }
}
