//! SPEC flow 26.22 (act from the palette) and LD-23/LD-24 (find any
//! session's turn) on a real daemon, with the live history and marks
//! clients: Move session ranks first in a session and asks inline for a
//! machine, Escape steps back; a word from a real agent conversation is
//! found on the host and its hit opens the read-only Conversation tab on
//! that entry (D186); a pasted PR URL offers Review this PR first; "long"
//! opens Settings at Long lines; a pin is kept on the host and both
//! clients on it follow it.
//!
//! The target's deps hold neither the palette, find nor settings crates,
//! so commands and rows are told apart by their `Debug` names here.
use gpui_kit::Role;
use humata2_app::{Overlay, Route};
use humata2_clients::Clients;
use humata2_clients::history::TurnRole;
use humata2_clients::marks::Thing;
use humata2_flow::Action;
use humata2_session::{ConversationTarget, Tab};

use super::world::World;
use super::{App, running_session, runtime};

/// The word only the agent's conversation has.
const WORD: &str = "pangolin";

/// A prompt as Claude writes it to its conversation file. The test agent
/// keeps each line typed to it there, so typing this line makes it a
/// prompt of the session's conversation.
fn prompt(text: &str) -> String {
    serde_json::json!({
        "type": "user",
        "isSidechain": false,
        "message": {"role": "user", "content": text},
        "timestamp": "2026-11-15T09:00:00.000Z",
    })
    .to_string()
}

/// What a palette entry does, by name.
fn command(entry: &impl std::fmt::Debug) -> String {
    format!("{entry:?}")
}

/// The palette's entries, as their commands' names.
fn commands(app: &App) -> Vec<String> {
    app.ui.read(|state| {
        state
            .palette
            .entries
            .iter()
            .map(|entry| command(&entry.command))
            .collect()
    })
}

fn palette_open(app: &App) -> bool {
    app.ui.read(|state| {
        state.palette.open && state.overlays.last() == Some(&Overlay::Palette)
    })
}

/// ⌘K, then waits until the palette shows with nothing typed.
fn open_palette(app: &mut App) {
    app.play(&Action::Key("secondary-k".to_owned()));
    app.wait_until("the palette to open", |app| {
        palette_open(app) && app.ui.read(|state| state.palette.query.is_empty())
    });
}

/// Types `text` into the palette and waits until it has it.
fn type_query(app: &mut App, text: &str) {
    app.type_text(text);
    app.wait_until("the palette to take what was typed", |app| {
        app.ui.read(|state| state.palette.query == text)
    });
}

/// Escape, then waits until the palette has closed.
fn close_palette(app: &mut App) {
    app.play(&Action::Key("escape".to_owned()));
    app.wait_until("the palette to close", |app| !palette_open(app));
}

fn pinned(app: &App, thing: &Thing) -> bool {
    app.ui
        .read(|state| state.pinned.list.items.iter().any(|pin| &pin.thing == thing))
}

/// Whether a client's own load of the pins from the host has `thing`.
fn host_pins(app: &App, clients: &Clients, thing: &Thing) -> bool {
    app.runtime
        .block_on(clients.marks.pinned())
        .is_ok_and(|list| list.items.iter().any(|pin| &pin.thing == thing))
}

#[test]
fn from_the_palette_a_session_moves_a_turn_is_found_a_pr_reviews_settings_open_and_a_pin_holds()
{
    let runtime = runtime();
    let (world, clients, other, id) = runtime.block_on(async {
        let world = World::start().await.expect("start the daemon");
        let clients = world.clients().await.expect("connect");
        let id = running_session(
            &world,
            &clients,
            "Fix the ledger",
            &prompt(&format!("reconcile the {WORD} invoices")),
        )
        .await;
        // A second client on the same daemon, as a second window would be.
        let other = world.clients().await.expect("connect a second client");
        (world, clients, other, id)
    });
    let marks = clients.marks.clone();
    let mut app = App::open(&runtime, clients);
    app.wait_until("the session to load", |app| {
        app.ui
            .read(|state| state.workspace.session(&id).is_some())
    });
    app.show_session(&id);
    app.wait_until("the session page to show", |app| {
        app.ui.read(|state| state.route == Route::Session(id.clone()))
    });

    // 1. In the session, "move" puts Move session first, with its
    // shortcut. Return asks inline for a machine; Escape steps back.
    open_palette(&mut app);
    assert!(
        app.ui.read(|state| state.palette.context == Some(id.clone())),
        "the palette knows the session beneath it"
    );
    type_query(&mut app, "move");
    assert_eq!(commands(&app).first().map(String::as_str), Some("MoveSession"));
    assert!(
        app.ui
            .read(|state| state.palette.entries.first().is_some_and(|e| !e.hint.is_empty())),
        "Move session shows its shortcut"
    );
    app.play(&Action::Key("enter".to_owned()));
    app.wait_until("Move session to ask for a machine", |app| {
        app.ui.read(|state| state.palette.asking.is_some())
    });
    let here = app.ui.read(|state| {
        state
            .workspace
            .session(&id)
            .map(|session| format!("{:?}", session.machine))
            .expect("the session is loaded")
    });
    let offered = commands(&app);
    assert!(
        offered.iter().all(|c| c.starts_with("MoveTo") && !c.contains(&here)),
        "while asking, only other machines are offered: {offered:?}"
    );
    assert!(palette_open(&app), "asking keeps the palette open");
    app.play(&Action::Key("escape".to_owned()));
    app.wait_until("Escape to step back from the question", |app| {
        app.ui.read(|state| state.palette.asking.is_none())
    });
    assert!(palette_open(&app), "the first Escape only steps back");
    assert!(app.ui.read(|state| state.palette.query == "move"));
    assert_eq!(commands(&app).first().map(String::as_str), Some("MoveSession"));
    close_palette(&mut app);
    assert_eq!(app.ui.read(|state| state.route.clone()), Route::Session(id.clone()));

    // 2. A word from the agent's conversation: the host's search finds
    // it, and its hit opens the Conversation tab on that turn.
    app.wait_until("the host to index the conversation", |app| {
        app.runtime
            .block_on(other.history.search(WORD, ""))
            .is_ok_and(|(hits, _)| hits.iter().any(|hit| hit.session == id))
    });
    open_palette(&mut app);
    type_query(&mut app, WORD);
    app.wait_until("the palette to show the hit", |app| {
        app.ui.read(|state| {
            state.find.query == WORD
                && !state.find.loading
                && state.find.hits.iter().any(|hit| hit.session == id)
                && state.palette.hits == state.find.hits.len()
        })
    });
    assert!(
        commands(&app).is_empty(),
        "no command matches the word, so Return opens the first hit: {:?}",
        commands(&app)
    );
    let hit = app.ui.read(|state| state.find.hits[0].clone());
    assert_eq!(hit.session, id);
    assert_eq!(hit.agent, "fake", "the hit names the session's agent");
    assert!(hit.turn >= 1, "the hit names its turn");
    app.play(&Action::Key("enter".to_owned()));
    let target = ConversationTarget {
        turn: hit.turn,
        seq: hit.seq,
    };
    app.wait_until("the conversation to be read around the hit", |app| {
        app.ui.read(|state| {
            state
                .session
                .conversations
                .get(&id)
                .is_some_and(|c| c.loading.is_none() && c.marked().is_some())
        })
    });
    assert!(!palette_open(&app), "the palette closes when its hit opens");
    assert_eq!(app.ui.read(|state| state.route.clone()), Route::Session(id.clone()));
    let (view_target, selected, conversation) = app.ui.read(|state| {
        let view = state.session.views.get(&id).expect("the session's page");
        (
            view.conversation,
            view.selected(view.focused_group).cloned(),
            state.session.conversations.get(&id).cloned().expect("read"),
        )
    });
    assert_eq!(selected, Some(Tab::Conversation), "the Conversation tab is in front");
    assert_eq!(view_target, Some(target));
    assert_eq!(conversation.target, target);
    assert_eq!(conversation.error, None);
    let marked = conversation.marked().expect("the hit's entry is read");
    assert_eq!(marked.role, TurnRole::User);
    assert!(marked.text.contains(WORD));
    let from_host = runtime
        .block_on(other.history.read(&id, conversation.first, conversation.last))
        .expect("the host reads the same turns");
    assert_eq!(
        conversation.entries, from_host,
        "the tab shows the host's entries for the turns it read"
    );

    // 3. A pasted PR URL. D197: until the Review a pull request dialog exists,
    // Review this PR is offered only for a PR a session here reviews, and the
    // live host has none, so this one offers no dead-end Review entry.
    open_palette(&mut app);
    let url = "https://github.com/o/r/pull/123";
    // Typed, not pasted: on the headless window ⌘V doesn't reach the
    // palette's input, and the palette sees the same query either way.
    type_query(&mut app, url);
    assert!(
        commands(&app).iter().all(|c| !c.starts_with("ReviewPull")),
        "an unreviewed PR offers no Review this PR: {:?}",
        commands(&app)
    );
    close_palette(&mut app);

    // 4. "long" reaches Settings › Long lines.
    open_palette(&mut app);
    type_query(&mut app, "long");
    let long_lines = app.ui.read(|state| {
        state
            .palette
            .entries
            .iter()
            .find(|entry| command(&entry.command) == "SettingsRow(LongLines)")
            .map(|entry| entry.title.clone())
    });
    let long_lines = long_lines
        .unwrap_or_else(|| panic!("\"long\" offers Long lines: {:?}", commands(&app)));
    app.click(Role::ListBoxOption, &long_lines);
    app.wait_until("Settings to open at Long lines", |app| {
        app.ui.read(|state| {
            state.route == Route::Settings
                && format!("{:?}", state.settings.revealed) == "Some(LongLines)"
        })
    });
    assert!(!palette_open(&app));

    // 5. A pin lives on the host: the app shows it from there, and a
    // second client on the same daemon has it too. The app has no Pin
    // control yet, so the pin goes through the app's own marks client.
    let thing = Thing::Session(id.clone());
    runtime
        .block_on(marks.set(&thing, Some(true), None))
        .expect("pin the session on the host");
    app.wait_until("the app's Pinned section to show the pin", |app| {
        pinned(app, &thing)
    });
    assert!(host_pins(&app, &other, &thing), "the second client sees the pin");
    // And the other way: the second client unpins, and the app follows.
    runtime
        .block_on(other.marks.set(&thing, Some(false), None))
        .expect("unpin from the second client");
    app.wait_until("the app to follow the unpin", |app| !pinned(app, &thing));
    assert!(!host_pins(&app, &other, &thing));

    drop(app);
    drop(world);
}
