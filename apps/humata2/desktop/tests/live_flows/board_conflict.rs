//! Macro case 05 ("Two agents edit the same item"): an item changes on the
//! daemon behind an open drawer, the drawer's save is refused, the drawer says
//! so and shows the daemon's current item, and a second save works.
//!
//! The refusal here is a dependency that would close a loop: the stale-view
//! refusal of a drawer edit, as the fake-side test
//! `features/board/tests/drawer.rs::a_refused_edit_shows_why_refreshes_and_keeps_the_picker`
//! covers. A move from an old copy of the item is refused by its revision
//! (D383); `board_revision.rs` proves that with an agent's move.
use gpui_kit::Role;
use humata2_app::{AppAction, Route};
use humata2_board::{BoardAction, Choice};
use humata2_clients::model::{ChannelId, Item, ItemId, Scope};

use super::world::World;
use super::{App, runtime};

/// How the view writes an element's line: its role, then its label.
fn heading(role: Role, label: &str) -> String {
    format!("{role:?} {label:?}")
}

fn shows(app: &mut App, role: Role, label: &str) -> bool {
    app.view().contains(&heading(role, label))
}

/// The card, row and picker label the board gives an item.
fn label_of(item: &Item) -> String {
    format!("{} {}", item.id.as_str(), item.title)
}

fn item_in_app(app: &App, id: &ItemId) -> Item {
    app.ui
        .read(|state| state.workspace.items.iter().find(|item| &item.id == id).cloned())
        .expect("the app lists the item")
}

fn item_on_daemon(app: &App, id: &ItemId) -> Item {
    let daemon = app.daemon.clone();
    app.runtime
        .block_on(daemon.load())
        .expect("the daemon loads")
        .items
        .into_iter()
        .find(|item| &item.id == id)
        .expect("the daemon lists the item")
}

/// Macro case 05, "Two agents edit the same item": with Schema's drawer open
/// and its dependency picker offering Write path, Write path comes to wait on
/// Schema behind the app's back. Picking Write path from the drawer would now
/// close a loop: the daemon refuses, the drawer shows an error, the picker
/// stays open, the app refreshes to the daemon's items (Write path waits on
/// Schema, Schema on nothing; the refused edge is not written), and picking
/// another item afterwards is saved.
#[test]
fn a_drawer_edit_refused_after_the_item_changed_on_the_daemon_refreshes_and_keeps_the_picker() {
    let runtime = runtime();
    let (world, clients, channel, schema, write, read) = runtime.block_on(async {
        let world = World::start().await.expect("start the daemon");
        let clients = world.clients().await.expect("connect");
        let mac = World::this_mac(&clients)
            .await
            .expect("the workspace has This Mac");
        let repo = world.repo("owner", "ledger").expect("make a repo");
        let channel: ChannelId = clients
            .daemon
            .create_channel("payments", vec![repo], vec![mac])
            .await
            .expect("create the channel");
        let scope = Scope::Channel(channel.clone());
        let schema = clients
            .daemon
            .create_item(scope.clone(), "Schema")
            .await
            .expect("create Schema");
        let write = clients
            .daemon
            .create_item(scope.clone(), "Write path")
            .await
            .expect("create Write path");
        let read = clients
            .daemon
            .create_item(scope, "Read path")
            .await
            .expect("create Read path");
        (world, clients, channel, schema, write, read)
    });
    let daemon = clients.daemon.clone();
    let mut app = App::open(&runtime, clients);
    app.wait_until("the three items to load", |app| {
        app.ui.read(|state| {
            [&schema, &write, &read]
                .iter()
                .all(|id| state.workspace.items.iter().any(|item| &item.id == *id))
        })
    });
    // The channel's page is reached by the navigation a sidebar row sends.
    let page = channel.clone();
    app.cx
        .update(|cx| app.ui.send(AppAction::Navigate(Route::Channel(page)), cx));
    app.settle();
    app.wait_until("the channel's tabs", |app| shows(app, Role::Tab, "To-do"));
    // A channel's page opens on the tab it showed last. Already on Board,
    // the board's own view switch shows a Tab "Board" too.
    if !shows(&mut app, Role::Button, "New item") {
        app.click(Role::Tab, "To-do");
    }
    let schema_label = label_of(&item_in_app(&app, &schema));
    let write_label = label_of(&item_in_app(&app, &write));
    let read_label = label_of(&item_in_app(&app, &read));
    app.wait_until("the cards", |app| {
        shows(app, Role::ListItem, &schema_label) && shows(app, Role::ListItem, &write_label)
    });

    app.click(Role::ListItem, &schema_label);
    app.wait_until("Schema's drawer", |app| {
        app.ui
            .read(|state| state.board.drawer.as_ref() == Some(&schema))
    });
    app.hover(Role::Button, "Add dependency");
    app.click(Role::Button, "Add dependency");
    app.wait_until("the picker to offer the other items", |app| {
        shows(app, Role::Button, &write_label) && shows(app, Role::Button, &read_label)
    });

    // Behind the open drawer, Write path comes to wait on Schema.
    runtime
        .block_on(daemon.add_dependency(&write, &schema))
        .expect("the change on the daemon");
    // The drawer's own view is stale until the app next hears of it. The
    // pick is sent as a row's click sends it, without letting the app catch
    // up first: the daemon refuses whatever the app thinks it offers.
    app.cx.update(|cx| {
        app.ui.send(
            AppAction::Board(BoardAction::Pick(Choice::Item(write.clone()))),
            cx,
        )
    });
    app.wait_until("the drawer to show the refusal", |app| {
        app.ui.read(|state| state.board.error.is_some())
    });
    // The app refreshes to what the daemon holds.
    app.wait_until("the app to hold Write path's new edge", |app| {
        item_in_app(app, &write).depends_on == vec![schema.clone()]
    });
    assert!(
        app.ui
            .read(|state| state.board.drawer.as_ref() == Some(&schema) && state.board.picker.is_some()),
        "the drawer and its picker stay open"
    );
    assert_eq!(
        item_on_daemon(&app, &schema).depends_on,
        Vec::<ItemId>::new(),
        "the refused edge was not written"
    );
    assert_eq!(item_on_daemon(&app, &write).depends_on, vec![schema.clone()]);
    app.wait_until("the picker to stop offering Write path", |app| {
        shows(app, Role::Button, &read_label) && !shows(app, Role::Button, &write_label)
    });

    // A save after the refresh works.
    app.click(Role::Button, &read_label);
    app.wait_until("the daemon to hold Schema's edge", |app| {
        item_on_daemon(app, &schema).depends_on == vec![read.clone()]
    });
    app.wait_until("the error to clear", |app| {
        app.ui.read(|state| state.board.error.is_none())
    });
    assert_eq!(item_on_daemon(&app, &write).depends_on, vec![schema.clone()]);
    drop(app);
    drop(world);
}
