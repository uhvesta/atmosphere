//! Micro cases 23 (session tabs) and 21 (command palette) on a real
//! daemon, in one channel with two running sessions whose clone of
//! `owner/ledger` holds a README.
//!
//! Micro 23: the **+** menu opens and closes and starts nothing; a file
//! read from the session's clone opens big as a tab, is left for the
//! terminal and picked again without a second tab, and its ✕ brings the
//! terminal back; ⌥⌘0 opens Changes when no panel shows, then hides and
//! shows the panel, and each session keeps its own; ⇧⌘↩ and the Expand
//! terminal button enter and leave full screen. The tab row's clicks are
//! sent as the actions they send where several share a label.
//!
//! Micro 21: the palette's file row opens the file from the session's
//! clone; a query nothing matches lists nothing and Enter runs nothing;
//! the palette opened over the shortcuts sheet closes first.
//!
//! Splits, a new shell and typing into a terminal are the terminal cases'.
use gpui_kit::Role;
use humata2_app::layout::FullScreen;
use humata2_app::{AppAction, AppState, Overlay, Route};
use humata2_clients::model::{Agent, SessionId, SessionOptions};
use humata2_flow::Action;
use humata2_session::{Panel, SessionAction, Tab};

use super::world::World;
use super::{App, agent_has, app_key, channel_with_repo, runtime};

/// The repo `channel_with_repo` makes, as the app names it, and its file.
const REPO: &str = "owner/ledger";
const README: &str = "README.md";
/// What `World::repo` writes in the README.
const README_TEXT: &str = "owner/ledger\n";

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

fn send(app: &mut App<'_>, action: AppAction) {
    app.cx.update(|cx| app.ui.send(action, cx));
    app.settle();
}

/// Presses `key` and waits until `done` holds of the app's state.
fn press_until(app: &mut App<'_>, key: &str, what: &str, mut done: impl FnMut(&AppState) -> bool) {
    press(app, key);
    app.wait_until(what, |app| app.ui.read(|state| done(state)));
}

/// One channel with two sessions, each with its agent running.
fn two_sessions(runtime: &tokio::runtime::Runtime) -> (World, humata2_clients::Clients, SessionId, SessionId) {
    runtime.block_on(async {
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
        let second = ids.pop().expect("two sessions");
        let first = ids.pop().expect("two sessions");
        (world, clients, first, second)
    })
}

fn open_session(app: &mut App<'_>, id: &SessionId) {
    app.show_session(id);
    app.wait_until("the session page to show", |app| {
        app.ui.read(|state| {
            state.route == Route::Session(id.clone()) && state.session.view().is_some()
        })
    });
}

fn shells(state: &AppState, id: &SessionId) -> usize {
    state.workspace.session(id).map_or(0, |session| session.shells.len())
}

fn file_tabs(state: &AppState) -> usize {
    state.session.view().map_or(0, |view| {
        view.groups
            .iter()
            .flat_map(|group| group.tabs.iter())
            .filter(|tab| matches!(tab, Tab::File { .. }))
            .count()
    })
}

fn readme_selected(state: &AppState) -> bool {
    state.session.view().is_some_and(|view| {
        matches!(view.selected(view.focused_group), Some(Tab::File { repo, path }) if repo == REPO && path == README)
    })
}

fn terminal_selected(state: &AppState) -> bool {
    state.session.view().is_some_and(|view| {
        matches!(view.selected(view.focused_group), Some(Tab::Terminal(_)))
    })
}

/// The README as read from `id`'s clone, once the host has answered.
fn readme_read(state: &AppState, id: &SessionId) -> Option<String> {
    state
        .session
        .file_contents
        .get(&(id.clone(), REPO.to_owned(), README.to_owned()))
        .and_then(|read| read.as_ref().ok())
        .cloned()
}

#[test]
fn the_tab_row_opens_and_closes_files_and_the_panel_and_full_screen_follow_their_keys() {
    let runtime = runtime();
    let (world, clients, first, second) = two_sessions(&runtime);
    let mut app = App::open(&runtime, clients);
    activate(&mut app);
    app.wait_until("both sessions to load", |app| {
        app.ui.read(|state| {
            state.workspace.session(&first).is_some() && state.workspace.session(&second).is_some()
        })
    });
    open_session(&mut app, &first);
    let before = app.ui.read(|state| shells(state, &first));

    // 1. + opens the menu for the first group; its Cancel closes it, and
    // nothing was started. (Its shell item is also named New terminal, so
    // the + is not clicked a second time.)
    app.click(Role::Button, "New terminal");
    app.wait_until("the new terminal menu", |app| {
        app.ui.read(|state| state.session.terminal_menu == Some(0))
    });
    app.click(Role::Button, "Cancel");
    app.wait_until("the menu to close", |app| {
        app.ui.read(|state| state.session.terminal_menu.is_none())
    });
    assert_eq!(app.ui.read(|state| shells(state, &first)), before, "the menu starts nothing");

    // 3. The README, picked in the Files panel, opens big as a selected
    // tab, read from the session's clone on the host.
    send(&mut app, AppAction::Session(SessionAction::SetPanel(Some(Panel::Files))));
    // A file is picked from the list, so the list has it first.
    app.wait_until("the README listed in the Files panel", |app| {
        app.ui.read(|state| {
            state.workspace.session_files.get(&first).is_some_and(|files| {
                files.iter().any(|file| file.repo == REPO && file.path == README)
            })
        })
    });
    send(
        &mut app,
        AppAction::Session(SessionAction::SelectFile {
            repo: REPO.to_owned(),
            path: README.to_owned(),
        }),
    );
    app.click(Role::Button, "Open big");
    app.wait_until("the README tab, read from the clone", |app| {
        app.ui.read(|state| readme_selected(state) && readme_read(state, &first).is_some())
    });
    assert_eq!(app.ui.read(|state| readme_read(state, &first)).as_deref(), Some(README_TEXT));
    assert_eq!(app.ui.read(file_tabs), 1);

    // A file left for the terminal and picked again keeps its one tab.
    send(&mut app, AppAction::Session(SessionAction::SelectGroupTab { group: 0, index: 0 }));
    app.wait_until("the terminal tab", |app| app.ui.read(terminal_selected));
    app.click(Role::Tab, README);
    app.wait_until("the README again", |app| app.ui.read(readme_selected));
    assert_eq!(app.ui.read(file_tabs), 1, "picking it again adds no tab");

    // 7, and the only tab of its group: ✕ closes the file tab and the
    // terminal shows again; nothing stopped.
    app.click(Role::Button, &format!("Close {README}"));
    app.wait_until("the file tab to close", |app| {
        app.ui.read(|state| file_tabs(state) == 0 && terminal_selected(state))
    });
    app.ui.read(|state| {
        assert_eq!(state.session.view().map(|view| view.groups.len()), Some(1));
        assert_eq!(shells(state, &first), before, "closing a view stops nothing");
    });

    // 8, and ⌥⌘0 with no panel: it opens Changes, then hides the panel,
    // then shows it again.
    send(&mut app, AppAction::Session(SessionAction::SetPanel(None)));
    app.wait_until("no panel", |app| app.ui.read(|state| state.panel().is_none()));
    press_until(&mut app, "alt-cmd-0", "⌥⌘0 to open Changes", |s| {
        s.panel() == Some(Panel::Changes) && !s.layout.panel.collapsed
    });
    press_until(&mut app, "alt-cmd-0", "⌥⌘0 to hide the panel", |s| s.layout.panel.collapsed);
    press_until(&mut app, "alt-cmd-0", "⌥⌘0 to show it again", |s| {
        !s.layout.panel.collapsed && s.panel() == Some(Panel::Changes)
    });

    // Each session keeps its own panel: hidden in the first, the second
    // still shows its own, and the first is hidden again on return.
    press_until(&mut app, "alt-cmd-0", "⌥⌘0 to hide the panel", |s| s.layout.panel.collapsed);
    open_session(&mut app, &second);
    send(&mut app, AppAction::Session(SessionAction::SetPanel(Some(Panel::Changes))));
    app.wait_until("the second session's panel to show", |app| {
        app.ui.read(|state| state.panel() == Some(Panel::Changes) && !state.layout.panel.collapsed)
    });
    open_session(&mut app, &first);
    assert!(app.ui.read(|state| state.layout.panel.collapsed), "the first keeps its panel hidden");

    // ⇧⌘↩ enters terminal full screen and leaves it; so do the button
    // and its other label.
    press_until(&mut app, "shift-cmd-enter", "terminal full screen", |s| {
        s.full_screen() == Some(FullScreen::Terminal)
    });
    press_until(&mut app, "shift-cmd-enter", "back to the columns", |s| s.full_screen().is_none());
    app.click(Role::Button, "Expand terminal");
    app.wait_until("terminal full screen", |app| {
        app.ui.read(|state| state.full_screen() == Some(FullScreen::Terminal))
    });
    app.click(Role::Button, "Return to columns");
    app.wait_until("back to the columns", |app| app.ui.read(|state| state.full_screen().is_none()));

    drop(app);
    drop(world);
}

fn palette_open(state: &AppState) -> bool {
    state.palette.open && state.overlays.last() == Some(&Overlay::Palette)
}

/// What each palette entry does, by its `Debug` name: the palette crate is
/// not in this target's deps.
fn commands(state: &AppState) -> Vec<String> {
    state
        .palette
        .entries
        .iter()
        .map(|entry| format!("{:?}", entry.command))
        .collect()
}

fn type_query(app: &mut App<'_>, text: &str) {
    app.type_text(text);
    app.wait_until("the palette to take what was typed", |app| {
        app.ui.read(|state| state.palette.query == text)
    });
}

#[test]
fn the_palette_opens_a_clone_file_lists_nothing_for_gibberish_and_stacks_over_a_sheet() {
    let runtime = runtime();
    let (world, clients, first, _second) = two_sessions(&runtime);
    let mut app = App::open(&runtime, clients);
    activate(&mut app);
    app.wait_until("the session to load", |app| {
        app.ui.read(|state| state.workspace.session(&first).is_some())
    });

    // The palette over the shortcuts sheet: a second ⌘K adds nothing, and
    // each Escape closes the top one.
    press_until(&mut app, &app_key("/"), "the shortcuts sheet", |s| {
        s.overlays == [Overlay::Shortcuts]
    });
    press_until(&mut app, &app_key("k"), "the palette over the sheet", |s| {
        s.overlays == [Overlay::Shortcuts, Overlay::Palette] && s.palette.open
    });
    press(&mut app, &app_key("k"));
    assert_eq!(
        app.ui.read(|state| state.overlays.clone()),
        [Overlay::Shortcuts, Overlay::Palette],
        "a second ⌘K opens nothing more"
    );
    press_until(&mut app, "escape", "Escape to close the palette", |s| {
        s.overlays == [Overlay::Shortcuts] && !s.palette.open
    });
    press_until(&mut app, "escape", "Escape to close the sheet", |s| s.overlays.is_empty());

    // Gibberish: nothing is listed, the search finds nothing, and Enter
    // runs nothing and leaves the palette open.
    open_session(&mut app, &first);
    press_until(&mut app, &app_key("k"), "the palette", |s| palette_open(s) && s.palette.query.is_empty());
    type_query(&mut app, "zqxvw");
    app.wait_until("nothing to match", |app| {
        app.ui.read(|state| {
            state.palette.entries.is_empty()
                && state.find.query == "zqxvw"
                && !state.find.loading
                && state.find.hits.is_empty()
        })
    });
    press(&mut app, "enter");
    app.ui.read(|state| {
        assert!(palette_open(state), "Enter on nothing keeps the palette open");
        assert_eq!(state.route, Route::Session(first.clone()));
        assert!(!readme_selected(state));
    });

    // 10. The README's row is this session's, from its clone; Enter on it
    // opens the file in the session.
    for _ in 0.."zqxvw".len() {
        press(&mut app, "backspace");
    }
    app.wait_until("the query to clear", |app| app.ui.read(|state| state.palette.query.is_empty()));
    type_query(&mut app, "README");
    let id = format!("{first:?}");
    app.wait_until("the README's file row", |app| {
        app.ui.read(|state| {
            commands(state)
                .iter()
                .any(|c| c.starts_with("File") && c.contains(&id) && c.contains(README))
        })
    });
    let row = app
        .ui
        .read(|state| {
            commands(state)
                .iter()
                .position(|c| c.starts_with("File") && c.contains(&id) && c.contains(README))
        })
        .expect("the README is listed");
    for step in 0..row {
        press_until(&mut app, "down", "the palette cursor to move", |s| s.palette.cursor == step + 1);
    }
    press(&mut app, "enter");
    app.wait_until("the README open in the session, read from its clone", |app| {
        app.ui.read(|state| {
            !palette_open(state)
                && state.route == Route::Session(first.clone())
                && readme_selected(state)
                && readme_read(state, &first).as_deref() == Some(README_TEXT)
        })
    });

    drop(app);
    drop(world);
}
