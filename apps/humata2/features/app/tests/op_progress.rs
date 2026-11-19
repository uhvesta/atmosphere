//! PROGRESS: the dialogs that wait on an operation keep how far it is. They
//! latch onto the first update of their kind, ignore other operations, and
//! forget it when they stop waiting.
use std::sync::Arc;

use humata2_app::{
    App, AppAction as A, AppState, Overlay, Route, SessionAddRepo,
    SessionAddRepoAction as Add,
};
use humata2_clients::model::{ChannelId, ForkKind, SessionId};
use humata2_clients::progress::{OpKind, OpProgress, OpUpdate};
use humata2_clients::{Clients, DaemonError, Event, fixture};
use humata2_session::{ContinueDraft, ForkDraft, SessionAction, SessionView};
use tca::{TestClock, TestStore};

fn step(title: &str, fraction: f32) -> OpProgress {
    OpProgress {
        title: title.to_owned(),
        fraction: Some(fraction),
        label: format!("{}%", (fraction * 100.0) as u32),
        started_ms: 0,
    }
}

fn update(id: &str, kind: OpKind, progress: Option<OpProgress>) -> A {
    A::Event(Event::Operation(OpUpdate { id: id.to_owned(), kind, progress }))
}

fn refused() -> DaemonError {
    DaemonError::Refused { what: "The step".into(), reason: "try again".into() }
}

/// A store on the "ledger" session; `prepare` shapes the state first.
fn store(prepare: impl FnOnce(&mut AppState)) -> TestStore<App> {
    let workspace = fixture::design();
    let (clients, _) = Clients::fake(workspace.clone(), Arc::new(TestClock::new()));
    let id = SessionId::new("ledger");
    let workspace = Arc::new(workspace);
    let mut state = AppState::launch(&clients);
    state.workspace = workspace.clone();
    state.session.workspace = workspace;
    state.session.open = Some(id.clone());
    state.session.views.insert(id.clone(), SessionView::default());
    state.route = Route::Session(id);
    prepare(&mut state);
    TestStore::new(state, App::new(clients)).non_exhaustive()
}

fn new_session(waiting: bool) -> TestStore<App> {
    let mut opener = store(|state| state.route = Route::Channel(ChannelId::new("payments-rewrite")));
    opener.send(A::StartNewSession, |_| {});
    let mut state = opener.state().clone();
    if let Some(Overlay::NewSession(sheet)) = state.overlays.last_mut() {
        sheet.creating = waiting;
    }
    let (clients, _) = Clients::fake(fixture::design(), Arc::new(TestClock::new()));
    TestStore::new(state, App::new(clients)).non_exhaustive()
}

fn sheet_progress(store: &TestStore<App>) -> (Option<OpProgress>, Option<String>) {
    match store.state().overlay() {
        Some(Overlay::NewSession(sheet)) => (sheet.progress.clone(), sheet.op.clone()),
        _ => panic!("the New session sheet is open"),
    }
}

#[test]
fn a_waiting_new_session_sheet_takes_the_first_updates_progress() {
    let mut store = new_session(true);
    store.send(update("op-1", OpKind::StartSession, Some(step("Cloning", 0.4))), |_| {});
    assert_eq!(sheet_progress(&store), (Some(step("Cloning", 0.4)), Some("op-1".into())));
    store.send(update("op-1", OpKind::StartSession, Some(step("Cloning", 0.8))), |_| {});
    assert_eq!(sheet_progress(&store).0, Some(step("Cloning", 0.8)));
}

#[test]
fn a_second_operation_and_other_kinds_are_ignored() {
    let mut store = new_session(true);
    store.send(update("op-1", OpKind::StartSession, Some(step("Cloning", 0.4))), |_| {});
    store.send(update("op-2", OpKind::StartSession, Some(step("Other", 0.9))), |_| {});
    store.send(update("op-1", OpKind::Fork, Some(step("Fork", 0.9))), |_| {});
    assert_eq!(sheet_progress(&store), (Some(step("Cloning", 0.4)), Some("op-1".into())));
}

#[test]
fn a_kind_mismatch_never_latches() {
    let mut store = new_session(true);
    store.send(update("op-9", OpKind::AddRepo, Some(step("Adding", 0.5))), |_| {});
    assert_eq!(sheet_progress(&store), (None, None));
}

#[test]
fn a_sheet_that_is_not_waiting_ignores_updates() {
    let mut store = new_session(false);
    store.send(update("op-1", OpKind::StartSession, Some(step("Cloning", 0.4))), |_| {});
    assert_eq!(sheet_progress(&store), (None, None));
}

#[test]
fn a_failed_create_clears_progress_so_a_retry_latches_fresh() {
    let mut store = new_session(true);
    store.send(update("op-1", OpKind::StartSession, Some(step("Cloning", 0.4))), |_| {});
    store.send(A::SessionCreated(Err(refused())), |_| {});
    assert_eq!(sheet_progress(&store), (None, None));
    // Waiting again, a new attempt latches onto its own operation.
    if let Some(Overlay::NewSession(sheet)) = store.state().overlay() {
        assert!(!sheet.creating);
    }
}

#[test]
fn a_fork_dialog_follows_its_operation_and_forgets_it_on_failure() {
    let mut store = store(|state| {
        state.session.forking = true;
        state.session.fork = Some(ForkDraft {
            kind: ForkKind::Shared,
            title: "Fork".into(),
            before_turn: None,
            agent: None,
            machine: None,
            branch: "fork".into(),
            turns: None,
            picker: None,
            progress: None,
            op: None,
        });
    });
    store.send(update("op-1", OpKind::Fork, Some(step("Cloning", 0.3))), |_| {});
    store.send(update("op-2", OpKind::Fork, Some(step("Other", 0.9))), |_| {});
    let fork = store.state().session.fork.clone();
    assert_eq!(fork.as_ref().and_then(|f| f.progress.clone()), Some(step("Cloning", 0.3)));
    store.send(A::Session(SessionAction::Forked(Err(refused()))), |_| {});
    let fork = store.state().session.fork.clone();
    assert!(fork.is_some_and(|f| f.progress.is_none() && f.op.is_none()));
}

#[test]
fn a_continue_dialog_follows_its_operation_and_forgets_it_on_failure() {
    let mut store = store(|state| {
        state.session.continuing = Some(ContinueDraft { sending: true, ..ContinueDraft::default() });
    });
    store.send(update("op-1", OpKind::ContinueInChannel, Some(step("Copying", 0.6))), |_| {});
    store.send(update("op-2", OpKind::ContinueInChannel, Some(step("Other", 0.1))), |_| {});
    let draft = store.state().session.continuing.clone();
    assert_eq!(draft.as_ref().and_then(|d| d.progress.clone()), Some(step("Copying", 0.6)));
    store.send(A::Session(SessionAction::Continued(Err(refused()))), |_| {});
    let draft = store.state().session.continuing.clone();
    assert!(draft.is_some_and(|d| !d.sending && d.progress.is_none() && d.op.is_none()));
}

#[test]
fn a_waiting_add_repo_sheet_follows_its_operation_and_forgets_it_on_failure() {
    let id = SessionId::new("ledger");
    let mut store = store(|_| {});
    store.send(A::SessionAddRepo(Add::Open(id)), |_| {});
    store.skip_received();
    let mut state = store.state().clone();
    for overlay in &mut state.overlays {
        if let Overlay::SessionAddRepo(sheet) = overlay {
            sheet.adding = Some("acme/web".into());
        }
    }
    let (clients, _) = Clients::fake(fixture::design(), Arc::new(TestClock::new()));
    let mut store = TestStore::new(state, App::new(clients)).non_exhaustive();
    let sheet = |store: &TestStore<App>| -> SessionAddRepo {
        store
            .state()
            .overlays
            .iter()
            .find_map(|o| match o {
                Overlay::SessionAddRepo(s) => Some(s.clone()),
                _ => None,
            })
            .expect("the sheet is open")
    };
    store.send(update("op-1", OpKind::AddRepo, Some(step("Cloning", 0.5))), |_| {});
    store.send(update("op-2", OpKind::AddRepo, Some(step("Other", 0.9))), |_| {});
    assert_eq!(sheet(&store).progress, Some(step("Cloning", 0.5)));
    store.send(
        A::SessionAddRepo(Add::Added { repo: "acme/web".into(), result: Err(refused()) }),
        |_| {},
    );
    let after = sheet(&store);
    assert!(after.progress.is_none() && after.op.is_none() && after.adding.is_none());
}
