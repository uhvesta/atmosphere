//! DV-4: closing a Desk session whose folder is not kept, while it holds
//! files, asks first; Save version and close saves then closes; Close closes.
use std::sync::Arc;

use humata2_clients::desk::{self, Document};
use humata2_clients::model::SessionId;
use humata2_clients::{FakeDaemon, Operation, Workspace, fixture};
use humata2_desk::{Change, Delegate, Desk, DeskAction as A, DeskState};
use tca::{TestClock, TestStore};

fn store_with(edit: impl FnOnce(&mut Workspace)) -> (TestStore<Desk>, FakeDaemon) {
    let mut workspace = fixture::design();
    fixture::desk(&mut workspace);
    edit(&mut workspace);
    let fake = FakeDaemon::new(workspace.clone(), Arc::new(TestClock::new()));
    let state = DeskState {
        workspace: Arc::new(workspace),
        ..DeskState::default()
    };
    let store = TestStore::new(state, Desk::new(Arc::new(fake.clone()))).non_exhaustive();
    (store, fake)
}

fn store() -> (TestStore<Desk>, FakeDaemon) {
    store_with(|_| {})
}

/// An unkept scratch folder holding a document.
fn plan() -> SessionId {
    SessionId::new("desk-plan-q4")
}

fn is_closed(fake: &FakeDaemon, id: &SessionId) -> bool {
    fake.workspace().desk.sessions[id].closed.is_some()
}

fn has_saved(fake: &FakeDaemon, id: &SessionId) -> bool {
    !fake.workspace().desk.sessions[id].doc_head.is_empty()
}

fn finish_close(store: &mut TestStore<Desk>, id: &SessionId) {
    store.receive_matching(
        "Changed(Close, Ok)",
        |a| {
            matches!(
                a,
                A::Changed {
                    change: Change::Close,
                    result: Ok(()),
                    ..
                }
            )
        },
        |_| {},
    );
    store.receive_matching(
        "Delegate(Toast(_))",
        |a| matches!(a, A::Delegate(Delegate::Toast(_))),
        |_| {},
    );
    assert!(!store.state().pending.contains(id));
}

#[test]
fn an_unkept_folder_holding_files_asks_before_closing() {
    let (mut store, fake) = store();
    store.send(A::AskClose(plan()), |_| {});
    let dialog = store.state().close.clone();
    assert_eq!(dialog.map(|d| d.session), Some(plan()));
    assert!(store.state().pending.is_empty());
    assert!(!is_closed(&fake, &plan()));
}

#[test]
fn a_kept_folder_closes_without_asking() {
    let (mut store, fake) = store();
    let kept = SessionId::new("desk-brin-rfc");
    store.send(A::AskClose(kept.clone()), |_| {});
    assert!(store.state().close.is_none());
    assert!(store.state().pending.contains(&kept));
    finish_close(&mut store, &kept);
    assert!(is_closed(&fake, &kept));
}

#[test]
fn an_unkept_folder_without_files_closes_without_asking() {
    let (mut store, fake) = store_with(|workspace| {
        if let Some(record) = workspace.desk.sessions.get_mut(&plan()) {
            record.holds_files = false;
        }
    });
    store.send(A::AskClose(plan()), |_| {});
    assert!(store.state().close.is_none());
    finish_close(&mut store, &plan());
    assert!(is_closed(&fake, &plan()));
}

#[test]
fn a_picked_folder_closes_without_asking() {
    let picked = SessionId::new("desk-tidy-infra");
    let (mut store, fake) = store_with(|workspace| {
        workspace.desk.documents.push(Document {
            session: SessionId::new("desk-tidy-infra"),
            path: "todo.md".to_owned(),
        });
    });
    store.send(A::AskClose(picked.clone()), |_| {});
    assert!(store.state().close.is_none());
    finish_close(&mut store, &picked);
    assert!(is_closed(&fake, &picked));
}

#[test]
fn save_version_and_close_saves_then_closes() {
    let (mut store, fake) = store();
    store.send(A::AskClose(plan()), |_| {});
    store.send(A::SaveAndClose(plan()), |_| {});
    assert!(store.state().pending.contains(&plan()));
    store.receive_matching(
        "SavedBeforeClose(Ok)",
        |a| matches!(a, A::SavedBeforeClose { result: Ok(()), .. }),
        |_| {},
    );
    assert!(store.state().close.is_none());
    finish_close(&mut store, &plan());
    assert!(has_saved(&fake, &plan()));
    assert!(is_closed(&fake, &plan()));
}

#[test]
fn close_closes_without_saving() {
    let (mut store, fake) = store();
    store.send(A::AskClose(plan()), |_| {});
    store.send(A::Close(plan()), |_| {});
    assert!(store.state().close.is_none());
    finish_close(&mut store, &plan());
    assert!(is_closed(&fake, &plan()));
    assert!(!has_saved(&fake, &plan()));
}

#[test]
fn a_failed_save_keeps_the_question_open_and_the_session_open() {
    let (mut store, fake) = store();
    fake.fail_next(Operation::Desk(desk::Operation::SaveVersion), "no luck");
    store.send(A::AskClose(plan()), |_| {});
    store.send(A::SaveAndClose(plan()), |_| {});
    store.receive_matching(
        "SavedBeforeClose(Err)",
        |a| matches!(a, A::SavedBeforeClose { result: Err(_), .. }),
        |_| {},
    );
    let dialog = store.state().close.clone();
    let Some(dialog) = dialog else {
        panic!("the question stays open");
    };
    assert!(!dialog.saving);
    assert!(dialog.error.is_some());
    assert!(store.state().pending.is_empty());
    assert!(!is_closed(&fake, &plan()));
}

#[test]
fn cancelling_the_question_closes_nothing() {
    let (mut store, fake) = store();
    store.send(A::AskClose(plan()), |_| {});
    store.send(A::CancelDialog, |_| {});
    assert!(store.state().close.is_none());
    assert!(!store.state().dialog_open());
    assert!(!is_closed(&fake, &plan()));
}
