//! Optional permission alerts never change the underlying approval request.
use humata2_app::{App, AppAction as A, AppState, saved};
use humata2_clients::{
    Clients, Event, fixture,
    model::{Answer, Request, RequestId, RequestKind},
};
use humata2_settings::SettingsAction;
use std::sync::Arc;
use tca::{TestClock, TestStore};

fn setup() -> (TestStore<App>, Clients, Request) {
    let workspace = Arc::new(fixture::design());
    let mut request = workspace.requests[0].clone();
    request.id = RequestId::new("new-permission-alert");
    request.kind = RequestKind::Command;
    request.answer = None;
    let (clients, _) = humata2_clients::fake_clients((*workspace).clone(), Arc::new(TestClock::new()));
    let mut state = AppState::launch(&clients);
    state.workspace = workspace.clone();
    state.inbox.workspace = workspace.clone();
    state.session.workspace = workspace;
    (
        TestStore::new(state, App::new(clients.clone())).non_exhaustive(),
        clients,
        request,
    )
}

fn receive(store: &mut TestStore<App>, request: Request) {
    store.send(A::Event(Event::Request(request)), |_| {});
    store.skip_received();
}

#[test]
fn disabled_alerts_keep_the_permission_in_the_inbox() {
    let (mut store, _, request) = setup();
    assert!(!store.state().settings.preferences.permission_request_alerts);
    receive(&mut store, request.clone());
    assert!(
        store
            .state()
            .workspace
            .waiting()
            .any(|saved| saved.id == request.id)
    );
    assert!(store.state().toasts.is_empty());
}

#[test]
fn enabled_alerts_notify_new_pending_permissions_once_and_can_be_disabled() {
    let (mut store, _, request) = setup();
    store.send(
        A::Settings(SettingsAction::TogglePermissionRequestAlerts),
        |_| {},
    );
    store.skip_received();
    receive(&mut store, request.clone());
    assert_eq!(store.state().toasts.len(), 1);
    receive(&mut store, request.clone());
    assert_eq!(store.state().toasts.len(), 1);
    let mut question = request.clone();
    question.id = RequestId::new("question-without-permission-alert");
    question.kind = RequestKind::Question;
    receive(&mut store, question);
    let mut answered = request.clone();
    answered.id = RequestId::new("already-answered-permission");
    answered.answer = Some(Answer::Approved);
    receive(&mut store, answered);
    assert_eq!(store.state().toasts.len(), 1);
    store.send(
        A::Settings(SettingsAction::TogglePermissionRequestAlerts),
        |_| {},
    );
    store.skip_received();
    let mut next = request;
    next.id = RequestId::new("permission-after-disabling-alerts");
    receive(&mut store, next.clone());
    assert_eq!(store.state().toasts.len(), 1);
    assert!(
        store
            .state()
            .workspace
            .waiting()
            .any(|saved| saved.id == next.id)
    );
}

#[test]
fn alert_preference_survives_restart_and_old_saves_default_to_off() {
    let (mut store, clients, _) = setup();
    store.send(
        A::Settings(SettingsAction::TogglePermissionRequestAlerts),
        |_| {},
    );
    store.skip_received();
    saved::Saver::new(clients.storage.clone())
        .save(1, &store.state().saved())
        .unwrap();
    assert!(
        AppState::launch(&clients)
            .settings
            .preferences
            .permission_request_alerts
    );
    clients
        .storage
        .save(saved::KEY, br#"{"preferences":{"font_size":16}}"#)
        .unwrap();
    let restored = AppState::launch(&clients);
    assert!(!restored.settings.preferences.permission_request_alerts);
    assert_eq!(restored.settings.preferences.font_size, 16);
}
