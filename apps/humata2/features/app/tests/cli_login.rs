//! SETTINGSFIX Slice 1: pending `humata login` requests surface immediately in the app
//! and can be approved or denied from the app store.

use std::sync::Arc;

use humata2_app::{App, AppAction, AppState};
use humata2_clients::{Event, Fakes, PendingCliLogin, fixture};
use tca::{TestClock, TestStore};

fn setup() -> (TestStore<App>, Fakes) {
    let workspace = fixture::design();
    let (clients, fakes) = humata2_clients::fake_clients(workspace, Arc::new(TestClock::new()));
    let state = AppState::launch(&clients);
    let store = TestStore::new(state, App::new(clients)).non_exhaustive();
    (store, fakes)
}

fn sample_login(id: &str, terminal: &str, hours: u32) -> PendingCliLogin {
    PendingCliLogin {
        id: id.to_owned(),
        terminal: terminal.to_owned(),
        hours,
        asked_at_unix_ms: 1_700_000_000_000,
    }
}

#[test]
fn pending_cli_login_event_surfaces_and_can_be_approved_in_app() {
    let (mut store, fakes) = setup();
    let login = sample_login("cl_1", "Terminal, ttys004", 12);
    fakes.daemon.push_pending_cli_login(login.clone());

    store.send(AppAction::Event(Event::PendingCliLogin(login.clone())), |_| {});
    assert_eq!(store.state().pending_cli_logins, vec![login]);

    store.send(
        AppAction::AnswerCliLogin {
            id: "cl_1".to_owned(),
            allow: true,
        },
        |_| {},
    );
    store.receive_matching(
        "daemon answered CLI login",
        |action| {
            matches!(
                action,
                AppAction::CliLoginAnswered {
                    id,
                    allow: true,
                    result: Ok(()),
                } if id == "cl_1"
            )
        },
        |_| {},
    );

    assert!(store.state().pending_cli_logins.is_empty());
    assert!(store.state().answering_cli_logins.is_empty());
    assert_eq!(
        fakes.daemon.cli_login_answers(),
        vec![("cl_1".to_owned(), true)]
    );
    assert!(fakes.daemon.pending_cli_logins().is_empty());
}

#[test]
fn pending_cli_login_can_be_denied_and_withdrawn_events_remove_it() {
    let (mut store, fakes) = setup();
    let login_deny = sample_login("cl_deny", "iTerm2, ttys002", 4);
    let login_withdraw = sample_login("cl_withdraw", "Ghostty, ttys005", 8);
    fakes.daemon.push_pending_cli_login(login_deny.clone());
    fakes.daemon.push_pending_cli_login(login_withdraw.clone());

    store.send(AppAction::LoadPendingCliLogins, |_| {});
    store.receive_matching(
        "pending CLI logins loaded",
        |action| matches!(action, AppAction::PendingCliLoginsLoaded(Ok(list)) if list.len() == 2),
        |_| {},
    );
    assert_eq!(store.state().pending_cli_logins.len(), 2);

    store.send(
        AppAction::AnswerCliLogin {
            id: "cl_deny".to_owned(),
            allow: false,
        },
        |_| {},
    );
    store.receive_matching(
        "daemon denied CLI login",
        |action| {
            matches!(
                action,
                AppAction::CliLoginAnswered {
                    id,
                    allow: false,
                    result: Ok(()),
                } if id == "cl_deny"
            )
        },
        |_| {},
    );
    assert_eq!(
        fakes.daemon.cli_login_answers(),
        vec![("cl_deny".to_owned(), false)]
    );
    assert_eq!(store.state().pending_cli_logins, vec![login_withdraw]);

    store.send(
        AppAction::Event(Event::PendingCliLoginGone("cl_withdraw".to_owned())),
        |_| {},
    );
    assert!(store.state().pending_cli_logins.is_empty());
}
