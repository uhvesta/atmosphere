//! A channel's own Desk on a real daemon (CD-2, CD-4, CD-6): from the
//! channel's Desk tab, New desk session starts one; it is listed on the
//! tab and under the channel in the sidebar; Close session, once confirmed,
//! leaves neither.
use gpui_kit::Role;
use humata2_app::{AppAction, Route};
use humata2_desk::DeskAction;

use super::world::World;
use super::{App, channel_with_repo, runtime};

const TITLE: &str = "Triage flaky tests";

#[test]
fn a_channel_desk_session_starts_from_its_tab_and_closing_it_takes_it_away() {
    let runtime = runtime();
    let (world, clients, channel) = runtime.block_on(async {
        let world = World::start().await.expect("start the daemon");
        let clients = world.clients().await.expect("connect");
        let (channel, _repo) = channel_with_repo(&world, &clients, "payments").await;
        (world, clients, channel)
    });
    let mut app = App::open(&runtime, clients);
    app.wait_until("the channel and This Mac to load", |app| {
        app.ui.read(|state| {
            state.workspace.channel(&channel).is_some()
                && state.workspace.machines.iter().any(|machine| machine.online)
        })
    });
    app.cx
        .update(|cx| app.ui.send(AppAction::OpenChannelDesk(channel.clone()), cx));
    app.settle();
    app.wait_until("the channel's page to open", |app| {
        app.ui
            .read(|state| state.route == Route::Channel(channel.clone()))
    });
    assert!(
        app.ui
            .read(|state| state.desk.channel_rows(&channel).is_empty()),
        "a new channel has no Desk session"
    );

    app.click(Role::Button, "New desk session");
    app.wait_until("the New desk session dialog", |app| {
        app.ui.read(|state| state.desk.channel_new.is_some())
    });
    app.click(Role::TextInput, "Title");
    app.type_text(TITLE);
    app.wait_until("the typed title to reach the form", |app| {
        app.ui.read(|state| {
            state
                .desk
                .channel_new
                .as_ref()
                .is_some_and(|form| form.title == TITLE)
        })
    });
    app.click(Role::Button, "Start");
    app.wait_until("the session to be listed on the channel's Desk", |app| {
        app.ui
            .read(|state| state.desk.channel_rows(&channel).len() == 1)
    });
    let (id, listed) = app.ui.read(|state| {
        let rows = state.desk.channel_rows(&channel);
        (
            rows.first().map(|row| row.session.id.clone()),
            rows.first().map(|row| row.session.title.clone()),
        )
    });
    let (Some(id), Some(listed)) = (id, listed) else {
        panic!("the Desk lists the session it started")
    };
    assert_eq!(listed, TITLE);
    app.wait_until("the sidebar to show the channel's Desk row", |app| {
        app.ui.read(|state| {
            state
                .sidebar
                .channel_desks
                .get(&channel)
                .is_some_and(|rows| rows.len() == 1)
        })
    });

    // Back on the tab, closing asks first.
    app.cx
        .update(|cx| app.ui.send(AppAction::OpenChannelDesk(channel.clone()), cx));
    app.settle();
    app.cx.update(|cx| {
        app.ui
            .send(AppAction::Desk(DeskAction::AskClose(id.clone())), cx)
    });
    app.settle();
    app.wait_until("the confirmation", |app| {
        app.ui.read(|state| state.desk.channel_close.is_some())
    });
    assert_eq!(
        app.ui
            .read(|state| state.desk.channel_rows(&channel).len()),
        1,
        "it stays open until confirmed"
    );
    app.click(Role::Button, "Close session");
    app.wait_until("the session to be gone from the Desk and the sidebar", |app| {
        app.ui.read(|state| {
            state.desk.channel_rows(&channel).is_empty()
                && state
                    .sidebar
                    .channel_desks
                    .get(&channel)
                    .is_none_or(Vec::is_empty)
        })
    });
    drop(app);
    drop(world);
}
