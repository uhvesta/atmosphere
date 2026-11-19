//! Micro case 22 (keyboard navigation) on a real daemon. Every key is
//! pressed as the app receives it (ui/src/keys.rs), on a workspace the
//! daemon holds: one channel with two running sessions.
//!
//! Proven here: ⌘L focuses the sidebar (the arrows that follow move its
//! cursor, which they do only while it has focus), ↓ ↑ → ← Home End and
//! Enter in the channel tree, ⌥⌘→ and ⌥⌘←, ⌘0 twice (the sidebar docks
//! again), ⌘L with the sidebar hidden, ⌘1 ⌘2 ⌘3, ⌘[ after a drill-in and
//! with no trail, ⌘/ and Escape, ⌘, and ⇧⌘N. ⌥⌘M is in `keys.rs`. Keys
//! that reach a terminal belong to the terminal cases.
use humata2_app::{AppState, Overlay, Route};
use humata2_clients::model::{Agent, ChannelId, SessionId, SessionOptions};
use humata2_flow::Action;

use super::world::World;
use super::{App, agent_has, app_key, channel_with_repo, runtime};

/// Headless windows start inactive; focus needs activation.
fn activate(app: &mut App<'_>) {
    app.cx
        .update_window(app.window, |_, window, _| window.activate_window())
        .expect("the window stays open");
    app.settle();
}

fn press(app: &mut App<'_>, key: &str) {
    app.play(&Action::Key(key.to_owned()));
}

/// Where the sidebar's cursor is among the rows that can be selected.
fn cursor(state: &AppState) -> Option<usize> {
    let cursor = state.sidebar.cursor.as_ref()?;
    state
        .sidebar
        .rows
        .iter()
        .filter_map(|row| row.key.as_ref())
        .position(|key| key == cursor)
}

/// Where the row whose key names `id` is among the rows that can be
/// selected, if it is drawn. The sidebar's key type is not in this
/// target's deps, so it is told by its `Debug` text.
fn row_of(state: &AppState, id: &str) -> Option<usize> {
    state
        .sidebar
        .rows
        .iter()
        .filter_map(|row| row.key.as_ref())
        .position(|key| format!("{key:?}").contains(&format!("{id:?}")))
}

fn selectable(state: &AppState) -> usize {
    state.sidebar.rows.iter().filter(|row| row.key.is_some()).count()
}

/// The depth of the row under the cursor.
fn cursor_depth(state: &AppState) -> Option<u16> {
    let cursor = state.sidebar.cursor.as_ref()?;
    state
        .sidebar
        .rows
        .iter()
        .find(|row| row.key.as_ref() == Some(cursor))
        .map(|row| row.depth)
}

fn folded(state: &AppState, channel: &ChannelId) -> bool {
    state.sidebar.folding.channels.contains(channel)
}

/// Presses `key` and waits until `done` holds of the app's state.
fn press_until(app: &mut App<'_>, key: &str, what: &str, mut done: impl FnMut(&AppState) -> bool) {
    press(app, key);
    app.wait_until(what, |app| app.ui.read(|state| done(state)));
}

/// Walks the cursor to the row `at`, from the first row.
fn walk_to(app: &mut App<'_>, at: usize) {
    press_until(app, "home", "the cursor on the first row", |s| cursor(s) == Some(0));
    for step in 1..=at {
        press_until(app, "down", "the cursor to move down", |s| cursor(s) == Some(step));
    }
}

#[test]
fn the_keyboard_walks_the_sidebar_opens_sessions_and_reaches_every_page_and_sheet() {
    let runtime = runtime();
    let (world, clients, channel, first, second) = runtime.block_on(async {
        let world = World::start().await.expect("start the daemon");
        let clients = world.clients().await.expect("connect");
        let (channel, repo) = channel_with_repo(&world, &clients, "payments").await;
        let mac = World::this_mac(&clients)
            .await
            .expect("the workspace has This Mac");
        let mut ids = Vec::new();
        for (title, message) in [("Fix the ledger", "first ledger"), ("Tidy invoices", "second invoices")] {
            let id = clients
                .daemon
                .create_session(
                    &channel,
                    Agent::Claude,
                    &mac,
                    title,
                    SessionOptions {
                        repos: vec![repo.clone()],
                        first_message: message.to_owned(),
                        ..SessionOptions::default()
                    },
                )
                .await
                .expect("create the session");
            agent_has(&world, message).await;
            ids.push(id);
        }
        let second: SessionId = ids.pop().expect("two sessions");
        let first: SessionId = ids.pop().expect("two sessions");
        (world, clients, channel, first, second)
    });
    let mut app = App::open(&runtime, clients);
    activate(&mut app);
    app.wait_until("both sessions in the sidebar", |app| {
        app.ui.read(|state| {
            row_of(state, first.as_str()).is_some() && row_of(state, second.as_str()).is_some()
        })
    });
    assert_eq!(app.ui.read(|state| state.route.clone()), Route::Home);

    // ⌘[ with an empty trail, and Escape with nothing to dismiss: nothing
    // changes.
    press(&mut app, &app_key("["));
    press(&mut app, "escape");
    app.ui.read(|state| {
        assert_eq!(state.route, Route::Home);
        assert!(state.trail.is_empty());
        assert!(state.overlays.is_empty());
    });

    // 1 and 2. ⌘L gives the sidebar focus with no cursor yet; the first ↓
    // selects the first row, the next moves one row down.
    press(&mut app, &app_key("l"));
    assert_eq!(app.ui.read(|state| state.sidebar.cursor.clone()), None);
    press_until(&mut app, "down", "the first row to be selected", |s| cursor(s) == Some(0));
    press_until(&mut app, "down", "the cursor to move down a row", |s| cursor(s) == Some(1));

    // ↑ at the first row and ↓ at the last stay put.
    press_until(&mut app, "up", "the cursor back on the first row", |s| cursor(s) == Some(0));
    press(&mut app, "up");
    assert_eq!(app.ui.read(cursor), Some(0), "↑ on the first row stays there");
    let last = app.ui.read(selectable) - 1;
    press_until(&mut app, "end", "End to reach the last row", |s| cursor(s) == Some(last));
    press(&mut app, "down");
    assert_eq!(app.ui.read(cursor), Some(last), "↓ on the last row stays there");

    // Escape with the sidebar focused and nothing to dismiss changes
    // nothing.
    press(&mut app, "escape");
    app.ui.read(|state| {
        assert_eq!(cursor(state), Some(last));
        assert_eq!(state.route, Route::Home);
        assert!(state.overlays.is_empty());
    });

    // 3. ← folds the open channel; → on the closed channel opens it and
    // keeps the cursor; → again goes to its first child; ← goes back to
    // the parent; End and Home reach the ends.
    let at = app
        .ui
        .read(|state| {
            let channel = format!("{:?}", channel.as_str());
            state
                .sidebar
                .rows
                .iter()
                .filter_map(|row| row.key.as_ref())
                .position(|key| format!("{key:?}").contains(&channel))
        })
        .expect("the channel has a row");
    walk_to(&mut app, at);
    press_until(&mut app, "left", "← to fold the channel", |s| folded(s, &channel));
    assert_eq!(app.ui.read(cursor), Some(at), "folding keeps the cursor on the channel");
    assert!(app.ui.read(|s| row_of(s, first.as_str()).is_none()), "its sessions are folded away");
    press_until(&mut app, "right", "→ to open the channel", |s| !folded(s, &channel));
    assert_eq!(app.ui.read(cursor), Some(at), "opening keeps the cursor on the channel");
    press_until(&mut app, "right", "→ to go to the first child", |s| cursor(s) == Some(at + 1));
    assert_eq!(app.ui.read(cursor_depth), Some(1), "the child is a session in the channel");
    press_until(&mut app, "left", "← to go to the parent", |s| cursor(s) == Some(at));
    assert!(!app.ui.read(|s| folded(s, &channel)), "← on a child only moves");
    let last = app.ui.read(selectable) - 1;
    press_until(&mut app, "end", "End to reach the last row", |s| cursor(s) == Some(last));
    press_until(&mut app, "home", "Home to reach the first row", |s| cursor(s) == Some(0));

    // 5. ⌥⌘← folds every channel; ⌥⌘→ opens them all again.
    press_until(&mut app, "alt-cmd-left", "every channel to fold", |s| {
        folded(s, &channel) && row_of(s, first.as_str()).is_none()
    });
    press_until(&mut app, "alt-cmd-right", "every channel to open", |s| {
        s.sidebar.folding.channels.is_empty()
            && s.sidebar.folding.sessions.is_empty()
            && row_of(s, first.as_str()).is_some()
    });

    // 4. Enter on a session row opens it; the cursor stays on the row,
    // which is the current one; a sidebar open leaves no trail.
    let row = app
        .ui
        .read(|s| row_of(s, second.as_str()))
        .expect("the second session has a row");
    walk_to(&mut app, row);
    press_until(&mut app, "enter", "Enter to open the session", |s| {
        s.route == Route::Session(second.clone())
    });
    app.ui.read(|state| {
        assert_eq!(cursor(state), Some(row), "the cursor stays on the opened row");
        assert_eq!(state.sidebar.current, state.sidebar.cursor, "the row is the current one");
        assert!(state.trail.is_empty(), "a sidebar open is a lateral move");
    });

    // ⌘[ after a sidebar open goes nowhere.
    press(&mut app, &app_key("["));
    assert_eq!(app.ui.read(|state| state.route.clone()), Route::Session(second.clone()));

    // 7. ⌘1, ⌘2 and ⌘3: Home, the Agent inbox, the To-do list,
    // each with no trail.
    for (key, route) in [
        ("1", Route::Home),
        ("2", Route::Inbox),
        ("3", Route::Todo),
        ("1", Route::Home),
    ] {
        press_until(&mut app, &app_key(key), "the key's page", |s| s.route == route);
        assert!(app.ui.read(|state| state.trail.is_empty()), "a switch leaves no trail");
    }

    // 8. A drill-in from Home, as a card does, then ⌘[ comes back.
    app.show_session(&first);
    app.wait_until("the drill-in", |app| {
        app.ui.read(|s| s.route == Route::Session(first.clone()) && s.trail == [Route::Home])
    });
    press_until(&mut app, &app_key("["), "⌘[ to go back to Home", |s| {
        s.route == Route::Home && s.trail.is_empty()
    });

    // 9 and 10. ⌘/ opens the shortcuts sheet; Escape closes it and leaves
    // the page.
    press_until(&mut app, &app_key("/"), "the shortcuts sheet", |s| {
        s.overlays.last() == Some(&Overlay::Shortcuts)
    });
    press_until(&mut app, "escape", "Escape to close the sheet", |s| s.overlays.is_empty());
    assert_eq!(app.ui.read(|state| state.route.clone()), Route::Home);

    // 11. ⌘, opens Settings with no trail.
    press_until(&mut app, &app_key(","), "Settings", |s| {
        s.route == Route::Settings && s.trail.is_empty()
    });
    press_until(&mut app, &app_key("1"), "Home", |s| s.route == Route::Home);

    // 13. ⇧⌘N opens the New channel sheet; Escape closes it.
    press_until(&mut app, "shift-cmd-n", "the New channel sheet", |s| {
        matches!(s.overlays.last(), Some(Overlay::NewChannel(_)))
    });
    press_until(&mut app, "escape", "Escape to close the sheet", |s| s.overlays.is_empty());

    // 6. ⌘0 folds the sidebar; a second ⌘0 docks it again, not over the
    // page.
    press_until(&mut app, &app_key("0"), "the sidebar to fold", |s| s.layout.sidebar.collapsed);
    press_until(&mut app, &app_key("0"), "the sidebar to dock again", |s| {
        !s.layout.sidebar.collapsed
    });
    assert!(!app.ui.read(|state| state.navigation_overlay), "it docks, not over the page");

    // ⌘L with the sidebar hidden shows it and gives it focus: ↓ moves the
    // cursor.
    press(&mut app, &app_key("l"));
    press_until(&mut app, "home", "the cursor on the first row", |s| cursor(s) == Some(0));
    press_until(&mut app, &app_key("0"), "the sidebar to fold", |s| s.layout.sidebar.collapsed);
    press_until(&mut app, &app_key("l"), "⌘L to show the sidebar", |s| {
        !s.layout.sidebar.collapsed
    });
    press_until(&mut app, "down", "↓ to reach the shown sidebar", |s| cursor(s) == Some(1));

    drop(app);
    drop(world);
}
