//! CH-6, CH-18, CH-19 (frame 20) on a real daemon: a repo that another
//! channel added is in the library, and "Add a GitHub repo…" on a second
//! channel's Repos and machines tab lists it, a click clones it into that
//! channel, and the channel's page lists it. Against a real `humatad2`
//! whose `gh` is a fake.
use gpui_kit::Role;
use humata2_app::{AppAction, ChannelAddRepo, Overlay, Route, channel};

use super::world::World;
use super::{App, channel_with_repo, runtime};

/// What the open Add a GitHub repo sheet holds, or `None` while it isn't open.
fn sheet<R>(app: &App, read: impl FnOnce(&ChannelAddRepo) -> R) -> Option<R> {
    app.ui.read(|state| match state.overlays.last() {
        Some(Overlay::ChannelAddRepo(sheet)) => Some(read(sheet)),
        _ => None,
    })
}

#[test]
fn a_repo_from_the_library_is_added_to_an_existing_channel_from_its_repos_tab() {
    let runtime = runtime();
    let (world, clients, second) = runtime.block_on(async {
        let world = World::start().await.expect("start the daemon");
        let clients = world.clients().await.expect("connect");
        // The first channel adds owner/ledger, which puts it in the library.
        let (_first, ledger) = channel_with_repo(&world, &clients, "first").await;
        assert_eq!(ledger, "owner/ledger");
        let mac = World::this_mac(&clients)
            .await
            .expect("the workspace has This Mac");
        let api = world.repo("owner", "api").expect("make a repo");
        let second =
            super::create_channel_ready(&clients.daemon, "second", vec![api], vec![mac]).await;
        (world, clients, second)
    });
    let mut app = App::open(&runtime, clients);
    app.wait_until("the channels to load", |app| {
        app.ui
            .read(|state| state.workspace.channel(&second).is_some())
    });
    app.cx.update(|cx| {
        app.ui
            .send(AppAction::Navigate(Route::Channel(second.clone())), cx);
        app.ui.send(
            AppAction::Channel(channel::Action::Tab(channel::Tab::Repos)),
            cx,
        );
    });
    app.settle();

    app.click(Role::Button, "Add a GitHub repo…");
    app.wait_until("the Add a GitHub repo sheet to open", |app| {
        sheet(app, |_| ()).is_some()
    });
    app.wait_until("the library to list owner/ledger", |app| {
        sheet(app, |sheet| {
            sheet
                .search
                .library
                .iter()
                .any(|held| held.repo == "owner/ledger")
        })
        .unwrap_or(false)
    });
    let view = app.view();
    assert!(view.contains("owner/ledger"), "the library repo is listed:\n{view}");
    assert!(
        !view.contains(world.channels_folder().to_string_lossy().as_ref()),
        "the sheet shows no path:\n{view}"
    );

    app.click(Role::Button, "owner/ledger");
    app.wait_until("the sheet to close", |app| sheet(app, |_| ()).is_none());
    app.wait_until("the channel to list owner/ledger", |app| {
        app.ui.read(|state| {
            state
                .workspace
                .channel(&second)
                .is_some_and(|found| found.repos.iter().any(|repo| repo == "owner/ledger"))
        })
    });
    assert!(
        world
            .channels_folder()
            .join("second/main/ledger/.git")
            .exists(),
        "the repo was cloned under the channel"
    );
    drop(app);
    drop(world);
}
