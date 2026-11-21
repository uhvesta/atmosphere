//! Fast-tier unit tests for the Settings To-do web UI switch (WEBTODO, D905).

use std::sync::Arc;

use humata2_clients::daemon::{Operation, WebTodoSnapshot, WebTodoStatus};
use humata2_clients::{FakeDaemon, fixture};
use humata2_settings::{Settings, SettingsAction, SettingsState, WebTodoAction};
use tca::{TestClock, TestStore};

fn store() -> (TestStore<Settings>, FakeDaemon) {
    let (clients, fakes) =
        humata2_clients::fake_clients(fixture::design(), Arc::new(TestClock::new()));
    let store = TestStore::new(SettingsState::default(), Settings::new(clients));
    (store, fakes.daemon)
}

#[test]
fn off_by_default_and_toggling_on_shows_loopback_url() {
    let (mut store, _daemon) = store();

    store.send(SettingsAction::WebTodo(WebTodoAction::Load), |_| {});
    store.receive(
        SettingsAction::WebTodo(WebTodoAction::Loaded(Ok(WebTodoSnapshot {
            enabled: false,
            status: WebTodoStatus::Off,
        }))),
        |state| {
            state.web_todo.snapshot = Some(WebTodoSnapshot {
                enabled: false,
                status: WebTodoStatus::Off,
            });
        },
    );
    assert!(!store.state().web_todo.enabled());
    assert_eq!(store.state().web_todo.url(), None);

    store.send(
        SettingsAction::WebTodo(WebTodoAction::ToggleEnabled),
        |state| {
            if let Some(snap) = state.web_todo.snapshot.as_mut() {
                snap.enabled = true;
            }
        },
    );
    let listening = WebTodoSnapshot {
        enabled: true,
        status: WebTodoStatus::Listening {
            url: "http://127.0.0.1:17420/".to_owned(),
        },
    };
    store.receive(
        SettingsAction::WebTodo(WebTodoAction::Saved {
            before: WebTodoSnapshot {
                enabled: false,
                status: WebTodoStatus::Off,
            },
            result: Ok(listening.clone()),
        }),
        |state| {
            state.web_todo.snapshot = Some(listening.clone());
        },
    );
    assert!(store.state().web_todo.enabled());
    assert_eq!(store.state().web_todo.url(), Some("http://127.0.0.1:17420/"));
    assert_eq!(store.state().error, None);

    // Turning off clears the URL.
    store.send(
        SettingsAction::WebTodo(WebTodoAction::ToggleEnabled),
        |state| {
            if let Some(snap) = state.web_todo.snapshot.as_mut() {
                snap.enabled = false;
            }
        },
    );
    store.receive(
        SettingsAction::WebTodo(WebTodoAction::Saved {
            before: listening,
            result: Ok(WebTodoSnapshot {
                enabled: false,
                status: WebTodoStatus::Off,
            }),
        }),
        |state| {
            state.web_todo.snapshot = Some(WebTodoSnapshot {
                enabled: false,
                status: WebTodoStatus::Off,
            });
        },
    );
    assert!(!store.state().web_todo.enabled());
    assert_eq!(store.state().web_todo.url(), None);
    store.finish();
}

#[test]
fn refusal_rolls_back_the_switch_and_shows_error() {
    let (mut store, daemon) = store();

    store.send(SettingsAction::WebTodo(WebTodoAction::Load), |_| {});
    store.receive(
        SettingsAction::WebTodo(WebTodoAction::Loaded(Ok(WebTodoSnapshot::default()))),
        |state| {
            state.web_todo.snapshot = Some(WebTodoSnapshot::default());
        },
    );

    daemon.fail_next(Operation::SetWebTodoEnabled, "port busy");
    store.send(
        SettingsAction::WebTodo(WebTodoAction::ToggleEnabled),
        |state| {
            if let Some(snap) = state.web_todo.snapshot.as_mut() {
                snap.enabled = true;
            }
        },
    );
    store.receive_matching(
        "failure",
        |a| matches!(a, SettingsAction::WebTodo(WebTodoAction::Saved { result: Err(_), .. })),
        |state| {
            state.web_todo.snapshot = Some(WebTodoSnapshot::default());
            state.error = Some(
                "SetWebTodoEnabled failed: port busy. Try again; a test made this fail on purpose"
                    .to_owned(),
            );
        },
    );
    assert!(!store.state().web_todo.enabled());
    assert!(store.state().error.as_deref().unwrap_or("").contains("port busy"));
    store.finish();
}
