//! A channel's own Desk on a real daemon (CD-2, CD-4, CD-6, CHANNELDESK):
//! opening the channel's Desk tab starts its persistent primary Desk session;
//! it is listed on the tab and under the channel in the sidebar; Close Desk,
//! once confirmed, leaves neither.
use gpui_kit::Role;
use humata2_app::{AppAction, Route};

use super::world::World;
use super::{App, channel_with_repo, runtime};

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
    assert!(
        app.ui
            .read(|state| state.desk.channel_rows(&channel).is_empty()),
        "a new channel has no Desk session before its Desk tab opens"
    );

    // Opening the channel's Desk tab ensures its persistent primary Desk.
    app.cx
        .update(|cx| app.ui.send(AppAction::OpenChannelDesk(channel.clone()), cx));
    app.settle();
    app.wait_until("the channel's page to open", |app| {
        app.ui
            .read(|state| state.route == Route::Channel(channel.clone()))
    });
    app.wait_until("the primary Desk session to start for the channel", |app| {
        app.ui
            .read(|state| state.desk.channel_rows(&channel).len() == 1)
    });
    let listed = app.ui.read(|state| {
        state
            .desk
            .channel_rows(&channel)
            .first()
            .map(|row| row.session.title.clone())
    });
    assert_eq!(listed.as_deref(), Some("#payments Desk"));
    app.wait_until("the sidebar to show the channel's Desk row", |app| {
        app.ui.read(|state| {
            state
                .sidebar
                .channel_desks
                .get(&channel)
                .is_some_and(|rows| rows.len() == 1)
        })
    });

    // Closing from the Desk tab asks first.
    app.click(Role::Button, "Close Desk");
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
