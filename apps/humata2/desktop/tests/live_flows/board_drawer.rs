//! Macro case 11 ("Planning on the board"), the drawer's edits on a real
//! `humatad2`: a card opens its drawer, the Status menu moves the item and
//! its history grows, the dependency picker leaves out the item, its links
//! and loops and adds what Enter picks, and the Sessions row adds and
//! removes one of the channel's sessions. Each edit is checked on the
//! daemon, not only in the app. The picker is typed into and driven by its
//! keys, as a person would.
use gpui_kit::Role;
use humata2_app::{AppAction, Route};
use humata2_board::Choice;
use humata2_clients::model::{Item, ItemDetails, ItemId, ItemStatus, Scope};

use humata2_flow::Action;

use super::world::World;
use super::{App, running_session, runtime, session_now};

fn shows(app: &mut App<'_>, role: Role, label: &str) -> bool {
    app.view().contains(&format!("{role:?} {label:?}"))
}

/// The card, row and picker label the board gives an item.
fn label_of(item: &Item) -> String {
    format!("{} {}", item.id.as_str(), item.title)
}

fn item_in_app(app: &App<'_>, id: &ItemId) -> Item {
    app.ui
        .read(|state| state.workspace.items.iter().find(|item| &item.id == id).cloned())
        .expect("the app lists the item")
}

fn on_daemon(app: &App<'_>, id: &ItemId) -> (Item, ItemDetails) {
    let daemon = app.daemon.clone();
    let workspace = app.runtime.block_on(daemon.load()).expect("the daemon loads");
    let details = workspace.item_details.get(id).cloned().unwrap_or_default();
    let item = workspace
        .items
        .into_iter()
        .find(|item| &item.id == id)
        .expect("the daemon lists the item");
    (item, details)
}

fn history_len(app: &App<'_>, id: &ItemId) -> usize {
    let daemon = app.daemon.clone();
    app.runtime
        .block_on(daemon.item_history(id))
        .expect("the history reads")
        .len()
}

fn press(app: &mut App<'_>, key: &str) {
    app.play(&Action::Key(key.to_owned()));
    app.settle();
}

fn choices(app: &App<'_>) -> Vec<Choice> {
    app.ui.read(|state| state.board.choices())
}

/// Steps 4, 7, 9 and 10: Schema's card opens its drawer with its status,
/// Sessions and Depends on rows; the Status menu moves it to In progress
/// on the daemon and adds to its history, and offers neither Blocked nor
/// Won't do; with Write path already waiting
/// on Schema, the dependency picker offers only Read path (not Schema, not
/// the loop), the filter keeps it, and Enter links it; the Sessions picker
/// offers the channel's session, a pick adds it to the item and its ×
/// takes it off.
#[test]
fn the_drawer_moves_links_and_assigns_an_item_on_the_daemon() {
    let runtime = runtime();
    let (world, clients, session, channel, schema, write, read) = runtime.block_on(async {
        let world = World::start().await.expect("start the daemon");
        let clients = world.clients().await.expect("connect");
        let session = running_session(&world, &clients, "Cache warmup", "warm the cache").await;
        let channel = session_now(&clients.daemon, &session)
            .await
            .expect("the daemon lists the session")
            .channel;
        let scope = Scope::Channel(channel.clone());
        let daemon = &clients.daemon;
        let schema = daemon.create_item(scope.clone(), "Schema").await.expect("create Schema");
        let write = daemon
            .create_item(scope.clone(), "Write path")
            .await
            .expect("create Write path");
        let read = daemon.create_item(scope, "Read path").await.expect("create Read path");
        daemon
            .add_dependency(&write, &schema)
            .await
            .expect("Write path waits on Schema");
        (world, clients, session, channel, schema, write, read)
    });
    let mut app = App::open(&runtime, clients);
    app.wait_until("the items and the session to load", |app| {
        app.ui.read(|state| {
            [&schema, &write, &read]
                .iter()
                .all(|id| state.workspace.items.iter().any(|item| &item.id == *id))
                && state.workspace.session(&session).is_some()
        })
    });
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
    let read_label = label_of(&item_in_app(&app, &read));
    app.wait_until("Schema's card", |app| shows(app, Role::ListItem, &schema_label));

    // Step 4: the drawer.
    let status = item_in_app(&app, &schema).status;
    assert_ne!(status, ItemStatus::InProgress, "Schema starts out of In progress");
    app.click(Role::ListItem, &schema_label);
    app.wait_until("Schema's drawer", |app| {
        app.ui.read(|state| state.board.drawer.as_ref() == Some(&schema))
            && shows(app, Role::Button, status.name())
    });
    // Each field's + shows while the pointer is over it.
    for add in ["Add session", "Add dependency"] {
        app.hover(Role::Button, add);
        app.wait_until(add, |app| shows(app, Role::Button, add));
    }

    // Step 7: the Status menu moves it, on the daemon, with history.
    let before = history_len(&app, &schema);
    app.click(Role::Button, status.name());
    app.wait_until("the Status menu", |app| {
        app.ui.read(|state| state.board.status_menu)
            && shows(app, Role::Button, ItemStatus::InProgress.name())
    });
    for column in [ItemStatus::Blocked, ItemStatus::WontDo] {
        assert!(
            !shows(&mut app, Role::Button, column.name()),
            "the menu offers no column the daemon refuses"
        );
    }
    app.click(Role::Button, ItemStatus::InProgress.name());
    app.wait_until("the daemon to move Schema", |app| {
        on_daemon(app, &schema).0.status == ItemStatus::InProgress
    });
    app.wait_until("the drawer to show the new status", |app| {
        item_in_app(app, &schema).status == ItemStatus::InProgress
            && shows(app, Role::Button, ItemStatus::InProgress.name())
    });
    assert!(history_len(&app, &schema) > before, "the history records the move");

    // Step 9: the dependency picker.
    app.hover(Role::Button, "Add dependency");
    app.click(Role::Button, "Add dependency");
    app.wait_until("the dependency picker", |app| {
        app.ui.read(|state| state.board.picker.is_some()) && shows(app, Role::Button, &read_label)
    });
    assert_eq!(
        choices(&app),
        vec![Choice::Item(read.clone())],
        "not Schema itself, and not Write path, which waits on Schema"
    );
    app.click(Role::TextInput, "Filter dependencies");
    app.type_text("read");
    app.wait_until("the picker to take what was typed", |app| {
        app.ui.read(|state| {
            state
                .board
                .picker
                .as_ref()
                .is_some_and(|picker| picker.query == "read")
        })
    });
    assert_eq!(choices(&app), vec![Choice::Item(read.clone())], "the filter keeps Read path");
    press(&mut app, "down");
    press(&mut app, "up");
    press(&mut app, "enter");
    app.wait_until("the daemon to link Read path", |app| {
        on_daemon(app, &schema).0.depends_on == vec![read.clone()]
    });
    app.wait_until("the app to hold the link", |app| {
        item_in_app(app, &schema).depends_on == vec![read.clone()]
    });
    assert!(shows(&mut app, Role::Button, &read_label), "the link shows as a chip");
    assert_eq!(
        on_daemon(&app, &write).0.depends_on,
        vec![schema.clone()],
        "Write path is untouched"
    );
    press(&mut app, "escape");
    app.wait_until("Escape to close the picker", |app| {
        app.ui.read(|state| state.board.picker.is_none())
    });

    // Step 10: a session on the item, then off it.
    app.hover(Role::Button, "Add session");
    app.click(Role::Button, "Add session");
    app.wait_until("the session picker to offer the channel's session", |app| {
        shows(app, Role::Button, "Cache warmup")
    });
    assert_eq!(choices(&app), vec![Choice::Session(session.clone())]);
    app.click(Role::Button, "Cache warmup");
    app.wait_until("the daemon to put the session on Schema", |app| {
        on_daemon(app, &schema).1.sessions.contains(&session)
    });
    app.wait_until("the drawer to list the session", |app| {
        app.ui.read(|state| {
            state
                .workspace
                .item_details
                .get(&schema)
                .is_some_and(|details| details.sessions.contains(&session))
        })
    });
    // The row's × shows while the pointer is over the row.
    app.hover(Role::Button, "Remove session Cache warmup");
    app.wait_until("the drawer to offer to take it off", |app| {
        shows(app, Role::Button, "Remove session Cache warmup")
    });
    app.click(Role::Button, "Remove session Cache warmup");
    app.wait_until("the daemon to take the session off Schema", |app| {
        !on_daemon(app, &schema).1.sessions.contains(&session)
    });
    drop(app);
    drop(world);
}
