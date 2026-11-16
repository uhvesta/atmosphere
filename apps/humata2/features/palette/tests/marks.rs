//! NV-20 from the palette (and the session's ⋯, which opens it): the
//! session or channel beneath the palette can be pinned or unpinned and
//! marked VIP or not. Each entry says what it will do, from the marks the
//! app hands the palette when it opens, and running it hands the change to
//! the app.
use std::sync::Arc;

use humata2_clients::fixture;
use humata2_clients::marks::{Pinned, PinnedList, Thing};
use humata2_clients::model::{ChannelId, SessionId};
use humata2_palette::{Command, Delegate, Marked, Palette, PaletteAction as A, PaletteState};
use std::time::Duration;

use tca::{Reducer, TestStore};

fn rate() -> SessionId {
    SessionId::new("rate")
}

fn payments() -> ChannelId {
    ChannelId::new("payments-rewrite")
}

fn pinned(thing: Thing, vip: bool) -> PinnedList {
    PinnedList {
        items: vec![Pinned {
            thing,
            title: "Pinned".to_owned(),
            event: String::new(),
            vip,
        }],
        revision: 1,
    }
}

/// A palette opened over `session` or, without one, over `channel`'s page.
fn opened(session: Option<SessionId>, channel: Option<ChannelId>, marks: PinnedList) -> PaletteState {
    let mut workspace = fixture::design();
    for session in &mut workspace.sessions {
        session.vip = false;
        session.pinned = false;
    }
    let in_session = session.is_some();
    let mut state = PaletteState {
        workspace: Arc::new(workspace),
        context: session,
        channel,
        marks,
        ..Default::default()
    };
    let _ = Palette.reduce(&mut state, A::Open { in_session });
    state
}

fn offered(state: &PaletteState, command: &Command) -> Option<String> {
    state
        .entries
        .iter()
        .find(|entry| &entry.command == command)
        .map(|entry| entry.title.clone())
}

fn mark_commands(state: &PaletteState) -> Vec<Command> {
    state
        .entries
        .iter()
        .filter(|e| matches!(e.command, Command::Pin { .. } | Command::Vip { .. }))
        .map(|e| e.command.clone())
        .collect()
}

#[test]
fn an_unpinned_session_offers_pin_and_vip() {
    let state = opened(Some(rate()), None, PinnedList::default());
    let pin = Command::Pin {
        thing: Marked::Session(rate()),
        on: true,
    };
    let vip = Command::Vip {
        thing: Marked::Session(rate()),
        on: true,
    };
    assert!(offered(&state, &pin).is_some(), "Pin is offered: {:?}", state.entries);
    assert!(offered(&state, &vip).is_some(), "Mark VIP is offered");
}

#[test]
fn a_pinned_vip_session_offers_unpin_and_clearing_vip() {
    let state = opened(Some(rate()), None, pinned(Thing::Session(rate()), true));
    let unpin = Command::Pin {
        thing: Marked::Session(rate()),
        on: false,
    };
    let clear = Command::Vip {
        thing: Marked::Session(rate()),
        on: false,
    };
    assert!(offered(&state, &unpin).is_some(), "Unpin is offered");
    assert!(offered(&state, &clear).is_some(), "clearing VIP is offered");
    assert_eq!(mark_commands(&state).len(), 2, "never both Pin and Unpin");
}

#[test]
fn a_session_marked_vip_by_the_daemon_offers_clearing_it() {
    let mut state = opened(Some(rate()), None, PinnedList::default());
    let mut workspace = (*state.workspace).clone();
    for session in workspace.sessions.iter_mut().filter(|s| s.id == rate()) {
        session.vip = true;
    }
    state.workspace = Arc::new(workspace);
    let _ = Palette.reduce(&mut state, A::Open { in_session: true });
    let clear = Command::Vip {
        thing: Marked::Session(rate()),
        on: false,
    };
    assert!(offered(&state, &clear).is_some(), "clearing VIP is offered");
}

#[test]
fn a_channel_page_offers_pinning_its_channel() {
    let state = opened(None, Some(payments()), PinnedList::default());
    let pin = Command::Pin {
        thing: Marked::Channel(payments()),
        on: true,
    };
    assert!(offered(&state, &pin).is_some(), "Pin this channel is offered");

    let state = opened(None, Some(payments()), pinned(Thing::Channel(payments()), false));
    let unpin = Command::Pin {
        thing: Marked::Channel(payments()),
        on: false,
    };
    assert!(offered(&state, &unpin).is_some(), "Unpin this channel is offered");
}

#[test]
fn nothing_to_mark_outside_a_session_or_channel() {
    let state = opened(None, None, PinnedList::default());
    assert!(mark_commands(&state).is_empty(), "{:?}", mark_commands(&state));
}

#[test]
fn typing_finds_pin_and_running_it_hands_the_change_to_the_app() {
    let mut state = opened(Some(rate()), None, PinnedList::default());
    let pin = Command::Pin {
        thing: Marked::Session(rate()),
        on: true,
    };
    let Some(title) = offered(&state, &pin) else {
        panic!("Pin is offered");
    };
    let _ = Palette.reduce(&mut state, A::Query(title.clone()));
    let Some(index) = state.entries.iter().position(|e| e.command == pin) else {
        panic!("typing “{title}” finds Pin");
    };
    let mut store = TestStore::new(state, Palette)
        .non_exhaustive()
        .with_timeout(Duration::from_millis(100));
    store.send(A::Pick(index), |s| s.open = false);
    store.receive(A::Delegate(Delegate::Closed), |_| {});
    store.receive(A::Delegate(Delegate::Run(pin.clone())), |_| {});
    let state = store.state();
    assert!(!state.recent.contains(&pin), "a mark is not a recent pick");
}
