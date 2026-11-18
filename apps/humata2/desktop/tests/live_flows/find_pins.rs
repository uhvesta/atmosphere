//! Macro case 13 ("Finding anything") on a real daemon, with the live
//! history and marks clients: one character in the palette searches
//! nothing and two words find the turn again; the palette's Pin pins the
//! session on the host at the end of the Pinned list; archiving the
//! pinned session keeps its pin.
//!
//! The target's deps hold neither the palette nor find crates, so the
//! Pin entry is told apart by its command's `Debug` name.
use humata2_app::{Overlay, Route};
use humata2_clients::Clients;
use humata2_clients::marks::Thing;
use humata2_flow::Action;

use super::world::World;
use super::{App, app_key, running_session, runtime};

/// The words only the agent's conversation has.
const WORDS: &str = "pangolin invoices";

/// A prompt as Claude writes it to its conversation file (as in
/// `find.rs`): the test agent keeps each typed line there.
fn prompt(text: &str) -> String {
    serde_json::json!({
        "type": "user",
        "isSidechain": false,
        "message": {"role": "user", "content": text},
        "timestamp": "2026-11-15T09:00:00.000Z",
    })
    .to_string()
}

fn palette_open(app: &App<'_>) -> bool {
    app.ui.read(|state| {
        state.palette.open && state.overlays.last() == Some(&Overlay::Palette)
    })
}

fn open_palette(app: &mut App<'_>) {
    app.play(&Action::Key(app_key("k")));
    app.wait_until("the palette to open", |app| {
        palette_open(app) && app.ui.read(|state| state.palette.query.is_empty())
    });
}

fn type_query(app: &mut App<'_>, text: &str) {
    app.type_text(text);
    app.wait_until("the palette to take what was typed", |app| {
        app.ui.read(|state| state.palette.query == text)
    });
}

/// Backspace until the palette's query is empty.
fn clear_query(app: &mut App<'_>) {
    let typed = app.ui.read(|state| state.palette.query.chars().count());
    for _ in 0..typed {
        app.play(&Action::Key("backspace".to_owned()));
    }
    app.wait_until("the query to empty", |app| {
        app.ui.read(|state| state.palette.query.is_empty())
    });
}

/// The pins in the app's Pinned section, first to last.
fn app_pins(app: &App<'_>) -> Vec<Thing> {
    app.ui.read(|state| {
        state
            .pinned
            .list
            .items
            .iter()
            .map(|pin| pin.thing.clone())
            .collect()
    })
}

/// The pins a client loads from the host, first to last.
fn host_pins(app: &App<'_>, clients: &Clients) -> Vec<Thing> {
    app.runtime
        .block_on(clients.marks.pinned())
        .expect("the host lists the pins")
        .items
        .into_iter()
        .map(|pin| pin.thing)
        .collect()
}

/// Step 3: one character clears the hits and the search is not loading;
/// after it is deleted, the two words find the turn again.
#[test]
fn one_character_searches_nothing_and_two_words_find_the_turn_again() {
    let runtime = runtime();
    let (world, clients, id) = runtime.block_on(async {
        let world = World::start().await.expect("start the daemon");
        let clients = world.clients().await.expect("connect");
        let id = running_session(
            &world,
            &clients,
            "Fix the ledger",
            &prompt(&format!("reconcile the {WORDS} by Friday")),
        )
        .await;
        (world, clients, id)
    });
    let mut app = App::open(&runtime, clients);
    app.wait_until("the session to load", |app| {
        app.ui.read(|state| state.workspace.session(&id).is_some())
    });
    app.show_session(&id);
    open_palette(&mut app);

    type_query(&mut app, WORDS);
    app.wait_until("the host's hits", |app| {
        app.ui.read(|state| !state.find.loading && !state.find.hits.is_empty())
    });
    clear_query(&mut app);

    type_query(&mut app, "p");
    app.settle();
    assert!(
        app.ui.read(|state| {
            state.find.hits.is_empty() && !state.find.loading && state.find.error.is_none()
        }),
        "one character shows no hits and searches nothing"
    );
    clear_query(&mut app);

    type_query(&mut app, WORDS);
    app.wait_until("the hits to come back", |app| {
        app.ui.read(|state| !state.find.loading && !state.find.hits.is_empty())
    });
    drop(app);
    drop(world);
}

/// Step 8 and branch "Archiving a pinned session": with a channel pinned
/// first, the palette's Pin on the session pins it on the host after the
/// channel, the palette closes and the Pinned section lists it last; a
/// second client loads the same order; archiving the session keeps the
/// pin on the host and in the app.
#[test]
fn the_palettes_pin_adds_the_session_last_on_the_host_and_an_archive_keeps_it() {
    let runtime = runtime();
    let (world, clients, other, id) = runtime.block_on(async {
        let world = World::start().await.expect("start the daemon");
        let clients = world.clients().await.expect("connect");
        let id = running_session(&world, &clients, "Fix the ledger", "reconcile the ledger").await;
        let other = world.clients().await.expect("connect a second client");
        (world, clients, other, id)
    });
    let marks = clients.marks.clone();
    let daemon = clients.daemon.clone();
    let mut app = App::open(&runtime, clients);
    app.wait_until("the session to load", |app| {
        app.ui.read(|state| state.workspace.session(&id).is_some())
    });
    let channel = app
        .ui
        .read(|state| state.workspace.session(&id).map(|session| session.channel.clone()))
        .expect("the session has its channel");
    let first = Thing::Channel(channel);
    let session = Thing::Session(id.clone());
    runtime
        .block_on(marks.set(&first, Some(true), None))
        .expect("pin the channel on the host");
    app.wait_until("the channel's pin", |app| app_pins(app) == vec![first.clone()]);

    app.show_session(&id);
    app.wait_until("the session page", |app| {
        app.ui.read(|state| state.route == Route::Session(id.clone()))
    });
    open_palette(&mut app);
    type_query(&mut app, "pin");
    let at = app.ui.read(|state| {
        state
            .palette
            .entries
            .iter()
            .position(|entry| format!("{:?}", entry.command).starts_with("Pin {"))
    });
    let at = at.expect("the palette offers Pin for the session");
    while app.ui.read(|state| state.palette.cursor) < at {
        let was = app.ui.read(|state| state.palette.cursor);
        app.play(&Action::Key("down".to_owned()));
        app.wait_until("the cursor to move down", |app| {
            app.ui.read(|state| state.palette.cursor) > was
        });
    }
    app.play(&Action::Key("enter".to_owned()));
    app.wait_until("the palette to close", |app| !palette_open(app));
    app.wait_until("the Pinned section to list the session last", |app| {
        app_pins(app) == vec![first.clone(), session.clone()]
    });
    assert_eq!(
        host_pins(&app, &other),
        vec![first.clone(), session.clone()],
        "the host keeps the order"
    );

    // Archiving keeps the pin (NV-26).
    runtime
        .block_on(daemon.archive(&id, false))
        .expect("archive the session");
    app.wait_until("the app to hear of the archive", |app| {
        app.ui.read(|state| {
            state
                .workspace
                .session(&id)
                .is_none_or(|session| session.archived)
        })
    });
    app.settle();
    assert_eq!(
        host_pins(&app, &other),
        vec![first.clone(), session.clone()],
        "the archive keeps the pin on the host"
    );
    assert_eq!(app_pins(&app), vec![first, session], "and in the Pinned section");
    drop(app);
    drop(world);
}
