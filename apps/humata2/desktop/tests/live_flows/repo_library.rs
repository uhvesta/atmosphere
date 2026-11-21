//! Flow 02 step 3 with the repo library (RL-1 to RL-3, RL-6): a repo added
//! once is in the host's library, so the next New channel lists it first,
//! under Library, before anything is typed and whatever GitHub answers,
//! and picking it makes the second channel with it. Against a real
//! `humatad2` whose `gh` is a fake.
use gpui_kit::Role;
use humata2_app::{NewChannel, Overlay, Route};
use humata2_flow::Action;

use super::world::World;
use super::{App, runtime};

/// What the open New channel sheet holds, or `None` while it isn't open.
fn sheet<R>(app: &App, read: impl FnOnce(&NewChannel) -> R) -> Option<R> {
    app.ui.read(|state| match state.overlays.last() {
        Some(Overlay::NewChannel(sheet)) => Some(read(sheet)),
        _ => None,
    })
}

/// Opens the sheet and waits until the name field has taken typed text.
fn open_sheet_and_name(app: &mut App, name: &str) {
    app.play(&Action::Key("shift-cmd-n".to_owned()));
    app.wait_until("the New channel sheet to open", |app| {
        sheet(app, |_| ()).is_some()
    });
    app.click(Role::TextInput, "Channel name");
    app.type_text(name);
    app.wait_until("the name to reach the sheet", |app| {
        sheet(app, |sheet| sheet.name == name).unwrap_or(false)
    });
}

/// Creates the channel the open sheet describes and waits for its page
/// and its clones to finish.
fn create_and_wait(app: &mut App, button: &str) {
    app.click(Role::Button, button);
    app.wait_until("the channel page to open", |app| {
        app.ui.read(|state| {
            matches!(state.route, Route::Channel(_))
                && !state
                    .overlays
                    .iter()
                    .any(|overlay| matches!(overlay, Overlay::NewChannel(_)))
        })
    });
    app.wait_until("the channel's clones to finish", |app| {
        app.ui.read(|state| state.workspace.setups.is_empty())
    });
}

#[test]
fn a_repo_added_once_is_listed_first_in_the_next_new_channel_even_with_github_out_of_reach() {
    let runtime = runtime();
    let (world, clients) = runtime.block_on(async {
        let world = World::start().await.expect("start the daemon");
        for name in ["api", "ledger"] {
            world.repo("owner", name).expect("make a repo");
        }
        let clients = world.clients().await.expect("connect");
        (world, clients)
    });
    let mut app = App::open(&runtime, clients);
    app.wait_until("This Mac to load", |app| {
        app.ui
            .read(|state| state.workspace.machines.iter().any(|machine| machine.online))
    });

    // The first channel adds owner/api, which puts it in the library.
    open_sheet_and_name(&mut app, "first");
    app.wait_until("the repos to be listed", |app| {
        sheet(app, |sheet| sheet.search.result.is_some()).unwrap_or(false)
    });
    app.click(Role::CheckBox, "owner/api");
    app.wait_until("owner/api to be ticked", |app| {
        sheet(app, |sheet| sheet.repos == ["owner/api"]).unwrap_or(false)
    });
    create_and_wait(&mut app, "Create channel and clone 1 repo");

    // GitHub can no longer be searched, and the library still answers.
    world.sign_gh_out().expect("sign gh out");
    open_sheet_and_name(&mut app, "second");
    app.wait_until("the library to list owner/api", |app| {
        sheet(app, |sheet| {
            sheet
                .search
                .library
                .iter()
                .any(|held| held.repo == "owner/api")
        })
        .unwrap_or(false)
    });
    app.wait_until("the search to answer", |app| {
        sheet(app, |sheet| !sheet.search.loading && sheet.search.result.is_some()).unwrap_or(false)
    });
    let view = app.view();
    assert!(view.contains("Library"), "the library group is shown:\n{view}");
    assert!(view.contains("owner/api"), "the repo is listed:\n{view}");
    // GitHub's group may still show the listing fetched a moment ago (D477
    // keeps it a minute); the library holds only what was added.
    let library = sheet(&app, |sheet| {
        sheet
            .search
            .library
            .iter()
            .map(|held| held.repo.clone())
            .collect::<Vec<_>>()
    })
    .unwrap_or_default();
    assert_eq!(
        library,
        vec!["owner/api".to_owned()],
        "the library holds only the repo added before:\n{view}"
    );

    // Pick it from the library and make the second channel.
    app.click(Role::CheckBox, "owner/api");
    app.wait_until("owner/api to be ticked", |app| {
        sheet(app, |sheet| sheet.repos == ["owner/api"]).unwrap_or(false)
    });
    create_and_wait(&mut app, "Create channel and clone 1 repo");

    let second = app
        .ui
        .read(|state| {
            state
                .workspace
                .channels
                .iter()
                .find(|channel| channel.name == "second")
                .cloned()
        })
        .expect("the second channel is listed");
    assert_eq!(second.repos, vec!["owner/api".to_owned()]);
    assert!(
        world
            .channels_folder()
            .join("second/main/api/.git")
            .exists(),
        "the second channel cloned it again"
    );
    drop(app);
    drop(world);
}
