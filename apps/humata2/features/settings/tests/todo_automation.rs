//! Settings › General › To-do (TODO2WIRE, D902): the section loads from the
//! daemon, each change shows at once and is saved as the whole section, a
//! refusal puts it back, and unset values read as Ask, Ask, off, off, 3 days.
use humata2_clients::daemon::{TodoAutomation, TodoMode};
use humata2_clients::{FakeDaemon, Operation, fixture};
use humata2_settings::{
    Section, Settings, SettingsAction as A, SettingsState, TodoAutomationAction as T,
};
use std::sync::Arc;
use tca::{TestClock, TestStore};

fn setup() -> (TestStore<Settings>, FakeDaemon) {
    let (clients, fakes) = humata2_clients::fake_clients(fixture::design(), Arc::new(TestClock::new()));
    let store = TestStore::new(SettingsState::default(), Settings::new(clients));
    (store, fakes.daemon)
}

/// A store that has loaded the daemon's defaults.
fn loaded() -> (TestStore<Settings>, FakeDaemon) {
    let (mut store, daemon) = setup();
    store.send(A::Todo(T::Load), |_| {});
    store.receive(A::Todo(T::Loaded(Ok(TodoAutomation::default()))), |s| {
        s.todo.section = Some(TodoAutomation::default());
    });
    (store, daemon)
}

/// Saves `action`, expecting `after` to be shown and held.
fn change(store: &mut TestStore<Settings>, action: T, after: TodoAutomation) {
    store.send(A::Todo(action), move |s| s.todo.section = Some(after));
    store.receive(
        A::Todo(T::Saved { before: TodoAutomation::default(), result: Ok(after) }),
        |_| {},
    );
}

#[test]
fn the_defaults_are_ask_ask_off_off_and_three_days() {
    let section = TodoAutomation::default();
    assert_eq!(section.on_pr_merged, TodoMode::Ask);
    assert_eq!(section.on_todo_closed_archive_sessions, TodoMode::Ask);
    assert!(!section.auto_wake_on_ci_fail);
    assert!(!section.auto_wake_on_review_comments);
    assert_eq!(section.idle_nudge_days, 3);
}

#[test]
fn opening_general_loads_the_section() {
    let (mut store, _) = setup();
    store.send(A::Section(Section::General), |s| s.section = Section::General);
    store.receive(A::Todo(T::Loaded(Ok(TodoAutomation::default()))), |s| {
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

#[test]
fn zero_days_read_as_three() {
    let (mut store, _) = setup();
    let stored = TodoAutomation { idle_nudge_days: 0, ..TodoAutomation::default() };
    store.send(A::Todo(T::Loaded(Ok(stored))), |s| {
        s.todo.section = Some(TodoAutomation::default());
    });
    assert_eq!(store.state().todo.section.map(|s| s.idle_nudge_days), Some(3));
    store.finish();
}

#[test]
fn choosing_auto_for_a_merged_pull_request_saves_the_whole_section() {
    let (mut store, _) = loaded();
    let after = TodoAutomation { on_pr_merged: TodoMode::Auto, ..TodoAutomation::default() };
    change(&mut store, T::SetOnPrMerged(TodoMode::Auto), after);
    store.send(A::Todo(T::Load), |_| {});
    store.receive(A::Todo(T::Loaded(Ok(after))), |_| {});
    store.finish();
}

#[test]
fn choosing_auto_for_archiving_changes_only_that_field() {
    let (mut store, _) = loaded();
    let after = TodoAutomation {
        on_todo_closed_archive_sessions: TodoMode::Auto,
        ..TodoAutomation::default()
    };
    change(&mut store, T::SetOnClosedArchive(TodoMode::Auto), after);
    store.finish();
}

#[test]
fn toggling_wake_on_failing_checks_flips_it() {
    let (mut store, _) = loaded();
    let after = TodoAutomation { auto_wake_on_ci_fail: true, ..TodoAutomation::default() };
    change(&mut store, T::ToggleWakeOnCiFail, after);
    store.send(A::Todo(T::ToggleWakeOnCiFail), |s| s.todo.section = Some(TodoAutomation::default()));
    store.receive(
        A::Todo(T::Saved { before: after, result: Ok(TodoAutomation::default()) }),
        |_| {},
    );
    store.finish();
}

#[test]
fn toggling_wake_on_review_comments_flips_it() {
    let (mut store, _) = loaded();
    let after = TodoAutomation { auto_wake_on_review_comments: true, ..TodoAutomation::default() };
    change(&mut store, T::ToggleWakeOnReviewComments, after);
    store.finish();
}

#[test]
fn picking_seven_days_sets_seven() {
    let (mut store, _) = loaded();
    let after = TodoAutomation { idle_nudge_days: 7, ..TodoAutomation::default() };
    change(&mut store, T::SetIdleNudgeDays(7), after);
    store.finish();
}

#[test]
fn a_refused_change_is_put_back() {
    let (store, daemon) = loaded();
    let mut store = store.non_exhaustive();
    daemon.fail_next(Operation::SetTodoAutomation, "link unavailable");
    let tried = TodoAutomation { on_pr_merged: TodoMode::Auto, ..TodoAutomation::default() };
    store.send(A::Todo(T::SetOnPrMerged(TodoMode::Auto)), move |s| s.todo.section = Some(tried));
    store.receive_matching(
        "failure",
        |a| matches!(a, A::Todo(T::Saved { result: Err(_), .. })),
        |_| {},
    );
    assert_eq!(store.state().todo.section, Some(TodoAutomation::default()));
    assert!(store.state().error.is_some());
    store.finish();
}

#[test]
fn a_change_before_the_daemon_has_said_does_nothing() {
    let (mut store, _) = setup();
    store.send(A::Todo(T::SetIdleNudgeDays(7)), |_| {});
    store.finish();
}

#[test]
fn picking_a_day_count_closes_the_list() {
    let (mut store, _) = loaded();
    store.send(A::Todo(T::IdleMenu), |s| s.todo.idle_menu = true);
    let after = TodoAutomation { idle_nudge_days: 14, ..TodoAutomation::default() };
    store.send(A::Todo(T::SetIdleNudgeDays(14)), move |s| {
        s.todo.idle_menu = false;
        s.todo.section = Some(after);
    });
    store.receive(
        A::Todo(T::Saved { before: TodoAutomation::default(), result: Ok(after) }),
        |_| {},
    );
    store.finish();
}
