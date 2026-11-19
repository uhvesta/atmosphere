//! D781: a question read off its agent's screen shows that live terminal
//! in the wide Inbox. It never takes focus by itself, keeps every key once
//! it has focus (Escape too), never sizes the tab, and gives focus back to
//! the list when the item goes.
use super::*;
use gpui_kit::{Pixels, SharedString};
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

/// The window `height` points tall, redrawn.
fn resize(app: &mut App, height: f32) {
    app.cx
        .update_window(app.window, |_, window, cx| {
            window.resize(size(px(1280.), px(height)));
            window.bounds_changed(cx);
        })
        .expect("resize");
    app.settle();
    app.draw();
    app.settle();
    app.draw();
}

/// At the last paint: the terminal box, the screen inside it, one row's
/// height, Dismiss, and the bottom of the window, where the detail ends.
struct Layout {
    frame: Bounds<Pixels>,
    screen: Bounds<Pixels>,
    line: f32,
    dismiss: Bounds<Pixels>,
    window_bottom: f32,
}

fn layout(app: &mut App, id: &RequestId) -> Layout {
    let dismiss = SharedString::from(format!("dismiss {id}"));
    app.cx
        .update_window(app.window, |_, window, _| {
            // The grid is the tab's 24 rows and a pixel.
            let grid = window.find("terminal").bounds();
            Layout {
                frame: window.find("inbox-terminal").bounds(),
                screen: window.find("terminal-replica-pan").bounds(),
                line: (f32::from(grid.size.height) - 1.) / 24.,
                dismiss: window.find(dismiss).bounds(),
                window_bottom: f32::from(window.viewport_size().height),
            }
        })
        .unwrap()
}

fn bottom(bounds: Bounds<Pixels>) -> f32 {
    f32::from(bounds.origin.y + bounds.size.height)
}

#[test]
fn the_box_shows_whole_rows_inside_its_inset_and_dismiss_stays_in_view() {
    let (mut app, _, id) = inbox();
    for height in [760., 600.] {
        resize(&mut app, height);
        let at = layout(&mut app, &id);
        assert!(f32::from(at.frame.size.height) <= 420.5, "{height}: at most 420 pt");
        let rows = (f32::from(at.screen.size.height) - 1.) / at.line;
        assert!((rows - rows.round()).abs() < 0.05, "{height}: whole rows, not {rows}");
        assert!(rows.round() >= 8., "{height}: {rows} rows");
        for (inner, outer) in [
            (at.screen.origin.x, at.frame.origin.x),
            (at.screen.origin.y, at.frame.origin.y),
        ] {
            assert!(f32::from(inner - outer) >= 8.5, "{height}: the inset shows");
        }
        assert!(bottom(at.frame) - bottom(at.screen) >= 8.5, "{height}: the inset shows under the screen");
        assert!(
            f32::from(at.frame.origin.x + at.frame.size.width - at.screen.origin.x - at.screen.size.width) >= 8.5,
            "{height}: the inset shows beside the screen"
        );
        assert!(bottom(at.dismiss) <= at.window_bottom - 19.5, "{height}: Dismiss is in view");
    }
}

#[test]
fn a_short_window_keeps_eight_rows_and_the_detail_scrolls() {
    let (mut app, _, id) = inbox();
    resize(&mut app, 380.);
    let at = layout(&mut app, &id);
    let rows = (f32::from(at.screen.size.height) - 1.) / at.line;
    assert!(rows.round() >= 8., "{rows} rows");
    assert!(bottom(at.dismiss) > at.window_bottom, "Dismiss is under the fold");
}

/// The design workspace whose waiting questions are on a tab their session
/// doesn't have, so no terminal can be shown.
fn workspace_without_screens() -> Workspace {
    let mut workspace = workspace();
    for request in workspace.requests.iter_mut().filter(|r| r.answer.is_none()) {
        request.on_screen = Some(9);
    }
    workspace
}

fn hint(app: &mut App) -> String {
    app.draw();
    app.cx
        .update_window(app.window, |_, window, _| {
            window.find("inbox-screen-hint").label().unwrap_or_default().to_owned()
        })
        .unwrap()
}

#[test]
fn the_hint_sends_you_to_the_terminal_only_while_it_shows() {
    let (mut app, _, _) = inbox();
    assert!(hint(&mut app).contains("terminal"), "{}", hint(&mut app));
    let mut app = App::new(workspace_without_screens());
    app.settle();
    app.send(AppAction::Navigate(Route::Inbox));
    app.draw();
    app.settle();
    assert!(!app.contains("terminal-replica"), "no screen to show");
    assert!(!hint(&mut app).contains("terminal"), "{}", hint(&mut app));
}

/// How far below its card's top a request's "Open session" link sits. The
/// link shares the title row and is centred in it, so a title that wraps
/// pushes it down.
fn title_drop(app: &mut App, id: &RequestId) -> f32 {
    let card = SharedString::from(format!("request {id}"));
    let link = SharedString::from(format!("open session {id}"));
    app.draw();
    app.cx
        .update_window(app.window, |_, window, _| {
            f32::from(window.find(link).bounds().top()) - f32::from(window.find(card).bounds().top())
        })
        .unwrap()
}

// The card's title is a plain label, not in the view snapshots, so the test
// measures the title row through the link beside it. A long title wraps, but
// to two lines at most, so the link drops by about half a line, not more.
#[test]
fn a_long_title_wraps_to_two_lines_at_most() {
    let mut workspace = workspace();
    let Some(long) = workspace.requests.iter_mut().find(|r| r.answer.is_none()) else {
        panic!("the design workspace has a waiting request");
    };
    long.title = "Copilot asks: Do you trust the files in this folder? ".repeat(6);
    let long = long.id.clone();
    let mut app = App::new(workspace);
    app.send(AppAction::Navigate(Route::Inbox));
    let others: Vec<RequestId> = app.ui.read(|state| {
        state.inbox.shown().into_iter().map(|r| r.id.clone()).filter(|id| *id != long).collect()
    });
    let Some(other) = others.first() else {
        panic!("the Agent inbox shows another request");
    };
    let (long_drop, other_drop) = (title_drop(&mut app, &long), title_drop(&mut app, other));
    let extra = long_drop - other_drop;
    assert!(extra > 1., "a long title wraps instead of being cut ({extra})");
    assert!(extra < 14., "a long title stops at two lines ({extra})");
}

/// A waiting request with a very long title wraps to two lines and then
/// ends in an ellipsis on its Agent inbox card.
#[test]
fn inbox_long_title_frame() {
    let mut workspace = fixture::design();
    for s in &mut workspace.sessions {
        s.vip = s.id.as_str() == "ledger";
        s.pinned = false;
    }
    let Some(long) = workspace.requests.iter_mut().find(|r| r.answer.is_none()) else {
        panic!("the design workspace has a waiting request");
    };
    long.title = "Copilot asks: Do you trust the files in this folder? ".repeat(6);
    let mut app = App::new(workspace);
    app.cx.update(|cx| Theme::change(ThemeMode::Light, None, cx));
    app.send(AppAction::Switch(Route::Inbox));
    for id in ["pr-reviews", "web"] {
        app.send(AppAction::Sidebar(SidebarAction::Toggle(RowKey::Channel(
            humata2_clients::model::ChannelId::new(id),
        ))));
    }
    app.snapshot("inbox-long-title");
}
