//! The "free memory of hidden terminals" switch: on at first, toggles, and
//! is found by the palette in General.
use humata2_clients::{Clients, fixture};
use humata2_clients::daemon::TodoAutomation;
use humata2_settings::TodoAutomationAction;
use humata2_settings::{Preferences, Row, Section, Settings, SettingsAction as A, SettingsState};
use std::sync::Arc;
use tca::{TestClock, TestStore};

fn store() -> TestStore<Settings> {
    let (clients, _) = humata2_clients::fake_clients(fixture::design(), Arc::new(TestClock::new()));
    TestStore::new(SettingsState::default(), Settings::new(clients))
}

#[test]
fn hidden_terminals_are_freed_by_default() {
    assert!(Preferences::default().free_hidden_terminals);
}

#[test]
fn toggling_turns_it_off_and_back_on() {
    let mut store = store();
    store.send(A::ToggleFreeHiddenTerminals, |s| {
        s.preferences.free_hidden_terminals = false
    });
    store.send(A::ToggleFreeHiddenTerminals, |s| {
        s.preferences.free_hidden_terminals = true
    });
    store.finish();
}

#[test]
fn the_row_is_in_general_and_reachable_from_the_palette() {
    assert_eq!(Row::FreeHiddenTerminals.section(), Section::General);
    assert!(Row::ALL.contains(&Row::FreeHiddenTerminals));
    let mut store = store();
    store.send(A::Reveal(Row::FreeHiddenTerminals), |s| {
        s.section = Section::General;
        s.revealed = Some(Row::FreeHiddenTerminals);
    });
    store.receive(A::Todo(TodoAutomationAction::Loaded(Ok(TodoAutomation::default()))), |s| {
        s.todo.section = Some(TodoAutomation::default());
    });
    store.receive(
        A::WebTodo(humata2_settings::WebTodoAction::Loaded(Ok(
            humata2_clients::daemon::WebTodoSnapshot::default(),
        ))),
        |s| {
            s.web_todo.snapshot = Some(humata2_clients::daemon::WebTodoSnapshot::default());
        },
    );
    store.receive_matching(
        "general loaded",
        |a| matches!(a, A::General(humata2_settings::GeneralAction::Loaded(Ok(_)))),
        |s| {
            s.general.settings = Some(humata2_clients::daemon::FullSettings::default());
            s.general.terminal_font_draft = "Menlo".to_owned();
            s.general.scrollback_draft = "10000".to_owned();
            s.general.archive_pr_days_draft = "0".to_owned();
            s.general.archive_idle_days_draft = "0".to_owned();
        },
    );
    store.finish();
}
