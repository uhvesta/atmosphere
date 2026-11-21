//! NV-3/NV-4: drilling in supports Back; lateral navigation clears its trail.
use std::sync::Arc;
use humata2_app::{App, AppAction, AppState, Route};
use humata2_clients::{Clients, fixture};
use humata2_clients::model::SessionId;
use humata2_session::{Delegate, SessionAction};
use tca::{TestClock, TestStore};

#[test]
fn switching_screens_clears_the_drill_in_trail() {
    let (clients, _) = humata2_clients::fake_clients(fixture::design(), Arc::new(TestClock::new()));
    let mut state = AppState::launch(&clients);
    state.route = Route::Session(SessionId::new("ledger"));
    state.session.open = Some(SessionId::new("ledger"));
    state.trail = vec![Route::Home, Route::Inbox];
    let mut store = TestStore::new(state, App::new(clients));
    store.send(AppAction::Switch(Route::Home), |s| {
        s.route = Route::Home;
        s.trail.clear();
        s.session.open = None;
        // NV-37: the session being left keeps its side panel's state.
        s.panel_collapsed.insert(SessionId::new("ledger"), false);
    });
    store.send(AppAction::Navigate(Route::Inbox), |s| {
        s.route = Route::Inbox;
        s.trail.push(Route::Home);
    });
    store.send(AppAction::Back, |s| {
        s.route = Route::Home;
        s.trail.clear();
        s.forward_trail.push(Route::Inbox);
    });
    store.send(AppAction::Switch(Route::Settings), |s| {
        s.route = Route::Settings;
        s.forward_trail.clear();
    });
    store.send(AppAction::Back, |_| {});
    store.finish();
}

#[test]
fn back_and_forward_walk_the_trail_and_new_navigation_clears_forward() {
    let (clients, _) = humata2_clients::fake_clients(fixture::design(), Arc::new(TestClock::new()));
    let state = AppState::launch(&clients);
    let mut store = TestStore::new(state, App::new(clients)).non_exhaustive();
    store.send(AppAction::Navigate(Route::Inbox), |s| {
        s.route = Route::Inbox;
        s.trail = vec![Route::Home];
        s.forward_trail.clear();
    });
    store.send(AppAction::Navigate(Route::Hosts), |s| {
        s.route = Route::Hosts;
        s.trail = vec![Route::Home, Route::Inbox];
        s.forward_trail.clear();
    });
    store.send(AppAction::Back, |s| {
        s.route = Route::Inbox;
        s.trail = vec![Route::Home];
        s.forward_trail = vec![Route::Hosts];
    });
    store.send(AppAction::Forward, |s| {
        s.route = Route::Hosts;
        s.trail = vec![Route::Home, Route::Inbox];
        s.forward_trail.clear();
    });
    store.send(AppAction::Back, |s| {
        s.route = Route::Inbox;
        s.trail = vec![Route::Home];
        s.forward_trail = vec![Route::Hosts];
    });
    store.send(AppAction::Navigate(Route::Files), |s| {
        s.route = Route::Files;
        s.trail = vec![Route::Home, Route::Inbox];
        s.forward_trail.clear();
    });
    store.send(AppAction::Forward, |_| {});
    store.finish();
}

fn on_ledger(trail: Vec<Route>) -> TestStore<App> {
    let (clients, _) = humata2_clients::fake_clients(fixture::design(), Arc::new(TestClock::new()));
    let mut state = AppState::launch(&clients);
    state.route = Route::Session(SessionId::new("ledger"));
    state.session.open = Some(SessionId::new("ledger"));
    state.trail = trail;
    TestStore::new(state, App::new(clients)).non_exhaustive()
}

fn delegate(delegate: Delegate) -> AppAction {
    AppAction::Session(SessionAction::Delegate(delegate))
}

#[test]
fn archiving_the_open_session_lands_on_home_with_no_way_back_to_it() {
    let ledger = Route::Session(SessionId::new("ledger"));
    let mut store = on_ledger(vec![Route::Home, ledger.clone(), Route::Inbox]);
    store.send(
        delegate(Delegate::Archived(SessionId::new("ledger"), "Ledger".into(), None)),
        |s| {
            s.route = Route::Home;
            s.trail = vec![Route::Home, Route::Inbox];
        },
    );
    store.send(AppAction::Back, |s| {
        s.route = Route::Inbox;
        s.trail = vec![Route::Home];
    });
    store.finish();
}

#[test]
fn a_session_archived_elsewhere_goes_back_to_the_last_page_that_is_not_a_session() {
    let mut store = on_ledger(vec![Route::Inbox, Route::Session(SessionId::new("refund"))]);
    store.send(delegate(Delegate::Left), |s| {
        s.route = Route::Inbox;
        s.trail = vec![];
    });
    store.send(AppAction::Back, |s| s.route = Route::Inbox);
    store.finish();
}
