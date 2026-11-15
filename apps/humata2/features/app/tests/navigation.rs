//! NV-3/NV-4: drilling in supports Back; lateral navigation clears its trail.
use std::sync::Arc;
use humata2_app::{App, AppAction, AppState, Route};
use humata2_clients::{Clients, fixture};
use humata2_clients::model::SessionId;
use tca::{TestClock, TestStore};

#[test]
fn switching_screens_clears_the_drill_in_trail() {
    let (clients, _) = Clients::fake(fixture::design(), Arc::new(TestClock::new()));
    let mut state = AppState::launch(&clients);
    state.route = Route::Session(SessionId::new("ledger"));
    state.session.open = Some(SessionId::new("ledger"));
    state.trail = vec![Route::Home, Route::Inbox];
    let mut store = TestStore::new(state, App::new(clients));
    store.send(AppAction::Switch(Route::Home), |s| {
        s.route = Route::Home;
        s.trail.clear();
        s.session.open = None;
    });
    store.send(AppAction::Navigate(Route::Inbox), |s| {
        s.route = Route::Inbox;
        s.trail.push(Route::Home);
    });
    store.send(AppAction::Back, |s| {
        s.route = Route::Home;
        s.trail.clear();
    });
    store.send(AppAction::Switch(Route::Settings), |s| s.route = Route::Settings);
    store.send(AppAction::Back, |_| {});
    store.finish();
}
