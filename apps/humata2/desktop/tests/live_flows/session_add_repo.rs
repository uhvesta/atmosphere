//! Flow 02 step 3 and CH-21 on a real daemon: from a running session's menu,
//! Add repo to this session clones a repo the channel lacks in the session on
//! the session's branch, adds it to the channel, lists it among the session's
//! clones, and tells the agent nothing about it (D743).
use gpui_kit::Role;
use humata2_app::{Overlay, Route};
use humata2_clients::model::SessionId;

use super::world::World;
use super::{App, running_session, runtime};

/// What the open sheet holds, or `None` while it isn't open.
fn sheet_open(app: &App) -> bool {
    app.ui.read(|state| {
        state
            .overlays
            .iter()
            .any(|overlay| matches!(overlay, Overlay::SessionAddRepo(_)))
    })
}

/// The repos the session is cloned in, as the app lists them.
fn clones(app: &App, id: &SessionId) -> Vec<String> {
    app.ui.read(|state| {
        state
            .workspace
            .session(id)
            .map(|session| session.clones.iter().map(|clone| clone.repo.clone()).collect())
            .unwrap_or_default()
    })
}

#[test]
fn a_repo_added_to_a_running_session_is_cloned_in_it_joins_the_channel_and_is_not_told_to_the_agent() {
    let runtime = runtime();
    let (world, clients, id) = runtime.block_on(async {
        let world = World::start().await.expect("start the daemon");
        let clients = world.clients().await.expect("connect");
        let id = running_session(&world, &clients, "Fix the ledger", "before adding").await;
        world.repo("owner", "api").expect("make a second repo");
        (world, clients, id)
    });
    let mut app = App::open(&runtime, clients);
    app.wait_until("the session to load", |app| {
        app.ui.read(|state| state.workspace.session(&id).is_some())
    });
    app.show_session(&id);
    app.wait_until("the session page to show", |app| {
        app.ui.read(|state| state.route == Route::Session(id.clone()))
    });
    assert!(!clones(&app, &id).contains(&"owner/api".to_owned()));

    app.click(Role::Button, "Session actions");
    app.click(Role::Button, "Add repo to this session…");
    app.wait_until("the sheet to open", |app| sheet_open(app));
    app.click(Role::TextInput, "Repo to add");
    app.type_text("owner/api");
    app.wait_until("the typed repo to reach the sheet", |app| {
        app.ui.read(|state| {
            state.overlays.iter().any(|overlay| {
                matches!(overlay, Overlay::SessionAddRepo(sheet) if sheet.search.query == "owner/api")
            })
        })
    });
    app.click(Role::Button, "Add repo");

    app.wait_until("the repo to be among the session's clones", |app| {
        clones(app, &id).contains(&"owner/api".to_owned())
    });
    app.wait_until("the sheet to close", |app| !sheet_open(app));
    let channel_has_it = app.ui.read(|state| {
        state
            .workspace
            .channels
            .iter()
            .any(|channel| channel.repos.contains(&"owner/api".to_owned()))
    });
    assert!(channel_has_it, "the channel has the repo too");
    let cloned = app.ui.read(|state| {
        state
            .workspace
            .session(&id)
            .and_then(|session| {
                session
                    .clones
                    .iter()
                    .find(|clone| clone.repo == "owner/api")
                    .map(|clone| (clone.path.clone(), clone.branch.clone(), session.branch.clone()))
            })
    });
    let (path, branch, session_branch) = cloned.expect("the clone is listed");
    assert!(
        std::path::Path::new(&path).join(".git").exists(),
        "the clone is on disk at {path}"
    );
    assert_eq!(branch, session_branch, "the repo is on the session's branch");
    // The daemon's own test (session_add_repo) proves this with a line typed
    // after the add; here, by the time the clone is listed, nothing was.
    let told = world.conversation_text();
    assert!(!told.contains("owner/api"), "the agent was told about the repo: {told:?}");
    assert!(!told.contains(&path), "the agent was told the repo's path: {told:?}");
    drop(app);
    drop(world);
}
