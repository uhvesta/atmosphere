//! A failed archive keeps the session open and never reports success.
use humata2_clients::{Clients, DaemonError, Operation, fixture, model::SessionId};
use humata2_session::{SessionAction as A, SessionState, Sessions};
use std::sync::Arc;
use tca::{TestClock, TestStore};
#[test]
fn failed_archive_keeps_the_open_session() {
    let workspace = Arc::new(fixture::design());
    let (clients, fakes) = humata2_clients::fake_clients((*workspace).clone(), Arc::new(TestClock::new()));
    fakes
        .daemon
        .fail_next(Operation::Archive, "link unavailable");
    let id = SessionId::new("ledger");
    let state = SessionState {
        workspace,
        open: Some(id.clone()),
        ..Default::default()
    };
    let mut store = TestStore::new(state, Sessions::new(clients));
    store.send(A::Archive, |_| {});
    store.receive_matching(
        "the plan",
        |a| matches!(a, A::ArchivePlanned { result: Ok(_), .. }),
        |_| {},
    );
    let error = DaemonError::Scripted {
        operation: Operation::Archive,
        message: "link unavailable".into(),
    };
    store.receive(
        A::Archived {
            session: id.clone(),
            title: "Ledger schema migration".into(),
            result: Err(error.clone()),
        },
        |s| s.error = Some(error.to_string()),
    );
    assert_eq!(store.state().open, Some(id.clone()));
    assert!(!fakes.daemon.workspace().session(&id).unwrap().archived);
    store.finish();
}

/// PR-17: nothing is unpushed, but a review has drafts never submitted, so
/// Archive asks first instead of going ahead.
#[test]
fn archive_asks_first_when_a_review_has_unsubmitted_drafts() {
    let workspace = Arc::new(fixture::design());
    let (clients, fakes) = humata2_clients::fake_clients((*workspace).clone(), Arc::new(TestClock::new()));
    let id = SessionId::new("ledger");
    fakes.daemon.set_unsubmitted_drafts(&id, 2);
    let state = SessionState {
        workspace,
        open: Some(id.clone()),
        ..Default::default()
    };
    // The confirmation's contents are checked below, not as a state diff.
    let mut store = TestStore::new(state, Sessions::new(clients)).non_exhaustive();
    store.send(A::Archive, |_| {});
    store.receive_matching(
        "the plan",
        |a| matches!(a, A::ArchivePlanned { result: Ok(_), .. }),
        |_| {},
    );
    let confirm = store.state().archiving.as_ref().map(|confirm| confirm.plan.unsubmitted_drafts);
    assert_eq!(confirm, Some(2));
    assert!(!fakes.daemon.workspace().session(&id).unwrap().archived);
}

/// With no drafts and nothing unpushed, Archive goes straight ahead.
#[test]
fn archive_goes_ahead_when_a_review_has_no_unsubmitted_drafts() {
    let workspace = Arc::new(fixture::design());
    let (clients, _fakes) = humata2_clients::fake_clients((*workspace).clone(), Arc::new(TestClock::new()));
    let state = SessionState {
        workspace,
        open: Some(SessionId::new("ledger")),
        ..Default::default()
    };
    let mut store = TestStore::new(state, Sessions::new(clients)).non_exhaustive();
    store.send(A::Archive, |_| {});
    store.receive_matching(
        "the plan",
        |a| matches!(a, A::ArchivePlanned { result: Ok(_), .. }),
        |_| {},
    );
    assert!(store.state().archiving.is_none());
}

/// D784: the Archive confirm names the agent it ends and where it runs, as
/// the close dialog does: "Copilot on This Mac".
#[test]
fn the_archive_confirm_names_the_agent_and_machine_it_ends() {
    let mut workspace = fixture::design();
    let id = SessionId::new("ledger");
    for session in &mut workspace.sessions {
        if session.id == id {
            session.agent = humata2_clients::model::Agent::Copilot;
            session.machine = humata2_clients::model::MachineId::new("this-mac");
        }
    }
    let (clients, fakes) = humata2_clients::fake_clients(workspace.clone(), Arc::new(TestClock::new()));
    fakes.daemon.set_unsubmitted_drafts(&id, 1);
    let state = SessionState {
        workspace: Arc::new(workspace),
        open: Some(id),
        ..Default::default()
    };
    let mut store = TestStore::new(state, Sessions::new(clients)).non_exhaustive();
    store.send(A::Archive, |_| {});
    store.receive_matching(
        "the plan",
        |a| matches!(a, A::ArchivePlanned { result: Ok(_), .. }),
        |_| {},
    );
    let named = store.state().archiving.as_ref().map(|confirm| (confirm.agent.clone(), confirm.machine.clone()));
    assert_eq!(named, Some(("Copilot".to_owned(), "This Mac".to_owned())));
}
