//! An agent's To-do ask never reaches the Agent inbox or "Needs you"; it
//! notifies as a To-do notice.

use std::sync::Arc;

use humata2_app::{App, AppAction as A, AppState, Load, Route, needs_you};
use humata2_clients::model::{Request, RequestId, RequestKind, ToDoAsk};
use humata2_clients::{Event, FakeNotifier, NotifyTarget, fixture};
use humata2_settings::SettingsAction;
use tca::{TestClock, TestStore};

fn setup() -> (TestStore<App>, Arc<FakeNotifier>) {
    let workspace = Arc::new(fixture::design());
    let (clients, fakes) =
        humata2_clients::fake_clients((*workspace).clone(), Arc::new(TestClock::new()));
    let notifier = fakes.notifier.clone();
    let mut state = AppState::launch(&clients);
    state.load = Load::Ready;
    state.workspace = workspace.clone();
    state.inbox.workspace = workspace.clone();
    state.session.workspace = workspace;
    state.route = Route::Home;
    (TestStore::new(state, App::new(clients)).non_exhaustive(), notifier)
}

fn to_do_ask(store: &TestStore<App>) -> Request {
    let mut request = store.state().workspace.requests[0].clone();
    request.id = RequestId::new("todo-ask");
    request.kind = RequestKind::Command;
    request.operation = "todo.create".to_owned();
    request.answer = None;
    request.to_do = Some(ToDoAsk::NewItem);
    request
}

#[test]
fn a_to_do_ask_event_adds_no_inbox_count_and_posts_a_to_do_notice() {
    let (mut store, notifier) = setup();
    store.send(A::Settings(SettingsAction::ToggleDesktopNotifications), |s| {
        s.settings.preferences.notify.enabled = true;
    });
    store.skip_received();
    store.send(A::WindowActive(false), |_| {});
    store.skip_received();

    let waiting_before = humata2_inbox::waiting(&store.state().workspace);
    let needs_before = needs_you(&store.state().workspace).rows.len();
    let ask = to_do_ask(&store);
    store.send(A::Event(Event::Request(ask)), |_| {});
    store.skip_received();

    assert_eq!(humata2_inbox::waiting(&store.state().workspace), waiting_before);
    assert_eq!(needs_you(&store.state().workspace).rows.len(), needs_before);
    let posted = notifier.posted();
    assert_eq!(posted.len(), 1, "{posted:?}");
    assert!(matches!(posted[0].target, NotifyTarget::Todo { .. }), "{posted:?}");
}

#[test]
fn ask_why_expands_inline_on_todo_without_jumping_to_inbox() {
    let (mut store, _) = setup();
    store.send(A::Switch(Route::Todo), |_| {});
    store.skip_received();

    store.send(
        A::Todo(humata2_app::TodoAction::AskWhy("req-todo-1".to_owned())),
        |_| {},
    );
    store.skip_received();

    assert_eq!(store.state().route, Route::Todo);
    assert!(store.state().todo.expanded_why.contains("req-todo-1"));
}

#[test]
fn blocked_session_with_pending_to_do_ask_does_not_post_terminal_needs_you() {
    let (mut store, notifier) = setup();
    store.send(A::Settings(SettingsAction::ToggleDesktopNotifications), |s| {
        s.settings.preferences.notify.enabled = true;
    });
    store.skip_received();
    store.send(A::WindowActive(false), |_| {});
    store.skip_received();

    let mut session = store.state().workspace.sessions[0].clone();
    session.status = humata2_clients::model::Status::Working;
    store.send(A::Event(Event::Session(session.clone())), |_| {});
    store.skip_received();

    let mut ask = to_do_ask(&store);
    ask.session = session.id.clone();
    store.send(A::Event(Event::Request(ask)), |_| {});
    store.skip_received();
    assert_eq!(notifier.posted().len(), 1);
    notifier.clear();

    // Agent blocks waiting on that To-do ask: no TerminalNeedsYou notice should fire.
    session.status = humata2_clients::model::Status::NeedsYou;
    store.send(A::Event(Event::Session(session)), |_| {});
    store.skip_received();
    assert!(notifier.posted().is_empty(), "{:?}", notifier.posted());
}
