//! NV-20 through the app: the palette opens knowing the host's pins and
//! the channel beneath it, and its Pin and VIP commands change the marks on
//! the host, which the Pinned section then shows.
use std::sync::Arc;

use humata2_app::{App, AppAction as A, AppState, Route};
use humata2_clients::marks::{Call, Thing};
use humata2_clients::model::{ChannelId, SessionId};
use humata2_clients::{Clients, Fakes, fixture};
use humata2_find::PinnedAction;
use humata2_palette::{Command, Delegate, Marked, PaletteAction};
use tca::{TestClock, TestStore};

fn launched() -> (TestStore<App>, Fakes) {
    let (clients, fakes) = Clients::fake(fixture::design(), Arc::new(TestClock::new()));
    let state = AppState::launch(&clients);
    let mut store = TestStore::new(state, App::new(clients)).non_exhaustive();
    store.send(A::Launch, |_| {});
    store.receive_matching(
        "the pins loaded",
        |a| matches!(a, A::Pinned(PinnedAction::Loaded(Ok(_)))),
        |_| {},
    );
    store.settle();
    (store, fakes)
}

fn run(store: &mut TestStore<App>, command: Command) {
    store.send(
        A::Palette(PaletteAction::Delegate(Delegate::Run(command))),
        |_| {},
    );
    // Pin and VIP answer with the host's list; wait for it so the Pinned section shows it.
    store.receive_matching(
        "the host's pins after the change",
        |a| matches!(a, A::Pinned(PinnedAction::Loaded(_))),
        |_| {},
    );
}

#[test]
fn the_palette_opens_with_the_hosts_pins_and_the_channel_beneath_it() {
    let (mut store, _fakes) = launched();
    let channel = ChannelId::new("payments-rewrite");
    store.send(A::Switch(Route::Channel(channel.clone())), |_| {});
    store.settle();
    store.send(A::OpenPalette, |_| {});
    let state = store.state();
    assert_eq!(state.palette.channel, Some(channel));
    assert_eq!(state.palette.marks, state.pinned.list);
    store.skip_in_flight();
}

#[test]
fn pin_from_the_palette_pins_the_session_on_the_host() {
    let (mut store, fakes) = launched();
    let rate = SessionId::new("rate");
    run(
        &mut store,
        Command::Pin {
            thing: Marked::Session(rate.clone()),
            on: true,
        },
    );
    assert!(
        fakes
            .marks
            .calls()
            .contains(&Call::Set(Thing::Session(rate.clone()), Some(true), None)),
        "the host was asked to pin it: {:?}",
        fakes.marks.calls()
    );
    assert!(
        store
            .state()
            .pinned
            .list
            .items
            .iter()
            .any(|p| p.thing == Thing::Session(rate.clone())),
        "the Pinned section shows it"
    );
    store.skip_in_flight();
}

#[test]
fn vip_and_channel_pins_from_the_palette_reach_the_host() {
    let (mut store, fakes) = launched();
    let rate = SessionId::new("rate");
    let channel = ChannelId::new("infra");
    run(
        &mut store,
        Command::Vip {
            thing: Marked::Session(rate.clone()),
            on: true,
        },
    );
    run(
        &mut store,
        Command::Pin {
            thing: Marked::Channel(channel.clone()),
            on: true,
        },
    );
    let calls = fakes.marks.calls();
    assert!(calls.contains(&Call::Set(Thing::Session(rate), None, Some(true))), "{calls:?}");
    assert!(calls.contains(&Call::Set(Thing::Channel(channel), Some(true), None)), "{calls:?}");
    store.skip_in_flight();
}
