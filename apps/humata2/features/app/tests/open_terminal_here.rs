//! PL-10: Open terminal here in the file manager shows the session whose
//! machine the folder is on, with a new shell tab started in that folder.
use std::sync::Arc;

use humata2_app::{App, AppAction, AppState, Route};
use humata2_clients::model::SessionId;
use humata2_clients::{Clients, fixture};
use humata2_files::{Delegate, FilesAction};
use humata2_session::SessionAction;
use tca::{TestClock, TestStore};

#[test]
fn the_session_opens_with_a_new_shell_tab_started_in_the_folder() {
    let (clients, fakes) = humata2_clients::fake_clients(fixture::design(), Arc::new(TestClock::new()));
    let id = SessionId::new("ledger");
    let state = AppState::launch(&clients);
    let mut store = TestStore::new(state, App::new(clients)).non_exhaustive();
    store.send(
        AppAction::Loaded(Ok(fakes.daemon.workspace())),
        |_| {},
    );
    store.settle();
    store.skip_received();
    let before = fakes.daemon.workspace().session(&id).map_or(0, |session| session.shells.len());

    store.send(
        AppAction::Files(FilesAction::Delegate(Delegate::OpenTerminal {
            session: id.clone(),
            folder: "/srv/work/api".to_owned(),
        })),
        |_| {},
    );
    store.receive_matching(
        "the terminal opened",
        |action| matches!(action, AppAction::Session(SessionAction::TerminalOpened { result: Ok(_), .. })),
        |_| {},
    );
    assert_eq!(store.state().route, Route::Session(id.clone()));
    let after = store.state().session.workspace.session(&id).map_or(0, |session| session.shells.len());
    assert_eq!(after, before + 1);
    assert!(fakes.daemon.calls().iter().any(|call| call.contains("/srv/work/api")));
}
