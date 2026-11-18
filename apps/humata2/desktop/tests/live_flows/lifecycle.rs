//! Flows 2 step 14 and 5 steps 8 and 14 on a real daemon: archive a session
//! from its menu and undo it from the toast, and fork a session from the
//! Fork dialog.
use gpui_kit::Role;
use humata2_app::{AppAction, Route};
use humata2_session::SessionAction;

use super::world::World;
use super::{App, running_session, runtime};

/// A prompt as Claude writes it to its conversation file, so the session
/// has a conversation a fork can carry.
fn prompt(text: &str) -> String {
    serde_json::json!({
        "type": "user",
        "isSidechain": false,
        "message": {"role": "user", "content": text},
        "timestamp": "2026-11-15T09:00:00.000Z",
    })
    .to_string()
}

/// The view without the toast's wording of the title, so a title left in
/// the view is one the sidebar or page still shows.
fn view_without_quoted(app: &mut App, title: &str) -> String {
    app.view().replace(&format!("“{title}”"), "")
}

/// Flow 2 step 14 and flow 5 step 14: pick Archive session from the
/// session's menu, confirm if the dialog asks, see the session leave the
/// sidebar and the toast offer Undo; Undo brings it back.
#[test]
fn archiving_a_session_from_its_menu_and_undoing_brings_it_back() {
    let runtime = runtime();
    let (world, clients, id) = runtime.block_on(async {
        let world = World::start().await.expect("start the daemon");
        let clients = world.clients().await.expect("connect");
        let id = running_session(&world, &clients, "Fix the ledger", "before archiving").await;
        (world, clients, id)
    });
    let daemon = clients.daemon.clone();
    let mut app = App::open(&runtime, clients);
    app.wait_until("the session to load", |app| {
        app.ui.read(|state| state.workspace.session(&id).is_some())
    });
    app.show_session(&id);
    app.wait_until("the session page to show", |app| {
        app.ui.read(|state| state.route == Route::Session(id.clone()))
    });
    assert!(app.view().contains("Fix the ledger"));

    // The ellipsis menu, then Archive session…
    app.click(Role::Button, "Session actions");
    app.click(Role::Button, "Archive session…");
    // A session with unpushed work gets the dialog first; either way the
    // person ends on the same result. "Archive and delete the work" is the
    // choice that needs no remote.
    app.wait_until("the session to be archived or the dialog to ask", |app| {
        app.ui.read(|state| {
            state.session.archiving.is_some()
                || state
                    .workspace
                    .session(&id)
                    .is_none_or(|session| session.archived)
        })
    });
    if app.ui.read(|state| state.session.archiving.is_some()) {
        app.click(Role::Button, "Archive and delete the work");
    }
    app.wait_until("the session to be archived with its Undo toast", |app| {
        app.ui.read(|state| {
            state
                .workspace
                .session(&id)
                // A live load leaves archived sessions out, so gone counts.
                .is_none_or(|session| session.archived)
                && state.toasts.iter().any(|toast| toast.action.is_some())
        })
    });
    app.wait_until("the session to leave the sidebar", |app| {
        !view_without_quoted(app, "Fix the ledger").contains("Fix the ledger")
    });
    assert!(app.view().contains("Undo"), "the toast offers Undo");
    let listed = runtime
        .block_on(daemon.load())
        .expect("the daemon loads")
        .sessions
        .iter()
        .any(|session| session.id == id && !session.archived);
    assert!(!listed, "the daemon no longer lists it as active");

    app.click(Role::Button, "Undo");
    app.wait_until("the session to be back", |app| {
        app.ui.read(|state| {
            state
                .workspace
                .session(&id)
                .is_some_and(|session| !session.archived)
        })
    });
    app.wait_until("the session in the sidebar again", |app| {
        app.view().contains("Fix the ledger")
    });
    let back = runtime
        .block_on(daemon.load())
        .expect("the daemon loads")
        .sessions
        .into_iter()
        .find(|session| session.id == id)
        .expect("the daemon lists the restored session");
    assert!(!back.archived, "the daemon holds it as not archived");
    drop(app);
    drop(world);
}

/// Micro-24: a session archived from another window, here a second client
/// of the same daemon, closes its page in this one.
#[test]
fn a_session_archived_from_another_window_closes_its_page_here() {
    let runtime = runtime();
    let (world, clients, elsewhere, id) = runtime.block_on(async {
        let world = World::start().await.expect("start the daemon");
        let clients = world.clients().await.expect("connect");
        let id = running_session(&world, &clients, "Fix the ledger", "before archiving").await;
        let elsewhere = world.clients().await.expect("connect a second window");
        (world, clients, elsewhere, id)
    });
    let mut app = App::open(&runtime, clients);
    app.wait_until("the session to load", |app| {
        app.ui.read(|state| state.workspace.session(&id).is_some())
    });
    app.show_session(&id);
    app.wait_until("the session page to show", |app| {
        app.ui.read(|state| state.route == Route::Session(id.clone()))
    });

    runtime
        .block_on(elsewhere.daemon.archive(&id, false))
        .expect("the other window archives the session");
    app.wait_until("the session's page to close here", |app| {
        app.ui.read(|state| state.route != Route::Session(id.clone()))
    });
    drop(app);
    drop(world);
}

/// Flow 5 step 8: Fork on the session page opens the dialog on Code +
/// context, from Now; Fork starts a child that opens and nests under its
/// parent, and the daemon lists it with the original as its parent.
#[test]
fn forking_from_the_fork_dialog_starts_a_child_session_on_the_daemon() {
    let runtime = runtime();
    let (world, clients, id) = runtime.block_on(async {
        let world = World::start().await.expect("start the daemon");
        let clients = world.clients().await.expect("connect");
        let id = running_session(
            &world,
            &clients,
            "Write path",
            &prompt("write the ledger path"),
        )
        .await;
        (world, clients, id)
    });
    let daemon = clients.daemon.clone();
    let mut app = App::open(&runtime, clients);
    app.wait_until("the session to load", |app| {
        app.ui.read(|state| state.workspace.session(&id).is_some())
    });
    app.show_session(&id);
    app.wait_until("the session page to show", |app| {
        app.ui.read(|state| state.route == Route::Session(id.clone()))
    });

    app.click(Role::Button, "Fork");
    assert!(
        app.ui.read(|state| state.session.fork.is_some()),
        "Fork opens the dialog"
    );
    // The heading's words aren't in the accessibility tree; the dialog is,
    // and it forks the session the page has open.
    assert!(app.view().contains("Dialog \"Fork session\""), "the fork dialog shows");
    assert_eq!(app.ui.read(|state| state.session.open.clone()), Some(id.clone()));
    // The dialog's own Fork button shares its label with the page's, so the
    // driver can't pick one; the confirm is sent as the button sends it.
    app.cx.update(|cx| {
        app.ui
            .send(AppAction::Session(SessionAction::ConfirmFork), cx)
    });
    app.settle();

    app.wait_until("the child session's page to open", |app| {
        app.ui.read(|state| match &state.route {
            Route::Session(open) => {
                open != &id
                    && state
                        .workspace
                        .session(open)
                        .is_some_and(|child| child.parent.as_ref() == Some(&id))
            }
            _ => false,
        })
    });
    let child = app.ui.read(|state| match &state.route {
        Route::Session(open) => open.clone(),
        _ => id.clone(),
    });
    assert!(
        app.ui.read(|state| state.session.fork.is_none()),
        "the dialog closes once the fork starts"
    );
    let title = app
        .ui
        .read(|state| state.workspace.session(&child).map(|s| s.title.clone()))
        .expect("the child is in the app");
    assert!(app.view().contains(&title), "the sidebar and page show the child");

    let listed = runtime
        .block_on(daemon.load())
        .expect("the daemon loads")
        .sessions
        .into_iter()
        .find(|session| session.id == child)
        .expect("the daemon lists the child");
    assert_eq!(listed.parent.as_ref(), Some(&id));
    drop(app);
    drop(world);
}
