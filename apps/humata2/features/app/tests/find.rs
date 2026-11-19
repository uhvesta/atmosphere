//! LD-23, LD-24, NV-24: a hit picked from the palette's search opens its
//! session, and a pin made on another client shows in the Pinned section.
use humata2_app::{App, AppAction as A, AppState, Overlay, Route};
use humata2_clients::history::TurnRole;
use humata2_clients::marks::{MarksClient, Thing};
use humata2_clients::{Clients, fixture, model::SessionId};
use humata2_find::{FindAction, PinnedAction};
use humata2_palette::PaletteAction;
use std::sync::Arc;
use tca::{TestClock, TestStore};

#[test]
fn picking_a_search_hit_opens_its_session_and_closes_the_palette() {
    let workspace = fixture::design();
    let (clients, fakes) = humata2_clients::fake_clients(workspace.clone(), Arc::new(TestClock::new()));
    let ledger = SessionId::new("ledger");
    fakes.history.push_turn(&ledger, 3, TurnRole::Agent, "The ledger totals reconcile now.");
    let mut state = AppState::launch(&clients);
    state.workspace = Arc::new(workspace);
    let mut store = TestStore::new(state, App::new(clients)).non_exhaustive();

    store.send(A::OpenPalette, |_| {});
    store.send(A::Palette(PaletteAction::Query("ledger totals".to_owned())), |_| {});
    store.receive_matching(
        "the search answered",
        |a| matches!(a, A::Find(FindAction::Loaded { .. })),
        |_| {},
    );
    assert_eq!(store.state().find.hits.len(), 1, "the one matching turn is a hit");

    store.send(A::Find(FindAction::Open(0)), |_| {});
    store.receive_matching(
        "the hit asks to open",
        |a| matches!(a, A::Find(FindAction::Delegate(_))),
        |_| {},
    );
    store.skip_received();
    assert_eq!(store.state().route, Route::Session(ledger));
    assert!(
        !store.state().overlays.contains(&Overlay::Palette),
        "the palette closes when its hit opens"
    );
    store.skip_in_flight();
    store.finish();
}

#[test]
fn a_pin_made_on_another_client_shows_in_the_pinned_section() {
    let (clients, fakes) = humata2_clients::fake_clients(fixture::design(), Arc::new(TestClock::new()));
    let state = AppState::launch(&clients);
    let mut store = TestStore::new(state, App::new(clients)).non_exhaustive();
    store.send(A::Launch, |_| {});
    store.receive_matching(
        "the pins loaded",
        |a| matches!(a, A::Pinned(PinnedAction::Loaded(Ok(_)))),
        |_| {},
    );
    // Lets the watch start before the other client changes anything.
    store.settle();

    let ledger = Thing::Session(SessionId::new("ledger"));
    fakes.marks.set_title(&ledger, "Ledger");
    // Clones of the fake share its marks: this one is another app.
    let other = fakes.marks.clone();
    let runtime = tokio::runtime::Builder::new_current_thread().build().unwrap();
    runtime.block_on(other.set(&ledger, Some(true), None)).unwrap();

    store.receive_matching(
        "the watch told this app",
        |a| matches!(a, A::Pinned(PinnedAction::Changed(_))),
        |_| {},
    );
    let items = &store.state().pinned.list.items;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].thing, ledger);
    assert_eq!(items[0].title, "Ledger");
    store.skip_received();
    store.skip_in_flight();
    store.finish();
}
