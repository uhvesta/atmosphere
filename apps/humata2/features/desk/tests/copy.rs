//! DK-13: Copy to a channel… copies a document into a channel's shared
//! folder and leaves the Desk's copy.
use std::sync::Arc;

use humata2_clients::desk::Document;
use humata2_clients::model::{ChannelId, MachineId, SessionId};
use humata2_clients::{FakeDaemon, fixture};
use humata2_desk::{Delegate, Desk, DeskAction as A, DeskState};
use tca::{TestClock, TestStore};

/// `payments-rewrite` runs on one machine, `infra` on two.
fn store() -> (TestStore<Desk>, FakeDaemon) {
    let mut workspace = fixture::design();
    fixture::desk(&mut workspace);
    for channel in &mut workspace.channels {
        match channel.id.as_str() {
            "payments-rewrite" => channel.machines = vec![MachineId::new("this-mac")],
            "infra" => {
                channel.machines = vec![MachineId::new("this-mac"), MachineId::new("devbox-1")];
            }
            _ => {}
        }
    }
    let fake = FakeDaemon::new(workspace.clone(), Arc::new(TestClock::new()));
    let state = DeskState {
        workspace: Arc::new(workspace),
        ..DeskState::default()
    };
    let store = TestStore::new(state, Desk::new(Arc::new(fake.clone()))).non_exhaustive();
    (store, fake)
}

fn document() -> Document {
    Document {
        session: SessionId::new("desk-brin-rfc"),
        path: "rfc-brin.md".to_owned(),
    }
}

fn copied_files(fake: &FakeDaemon) -> Vec<String> {
    fake.workspace()
        .files
        .iter()
        .filter(|file| file.path.ends_with("/shared/rfc-brin.md"))
        .map(|file| file.path.clone())
        .collect()
}

#[test]
fn a_channel_with_one_machine_needs_no_machine_pick() {
    let (mut store, fake) = store();
    store.send(A::StartCopy(document()), |_| {});
    store.send(A::CopyChannel(ChannelId::new("payments-rewrite")), |_| {});
    let Some(dialog) = store.state().copy.as_ref() else {
        panic!("the dialog is open");
    };
    assert_eq!(dialog.machine, Some(MachineId::new("this-mac")));

    store.send(A::ConfirmCopy, |_| {});
    store.receive(A::Copied(Ok(())), |_| {});
    store.receive_matching(
        "Delegate(Toast(_))",
        |a| matches!(a, A::Delegate(Delegate::Toast(_))),
        |_| {},
    );
    assert!(store.state().copy.is_none());
    assert_eq!(copied_files(&fake).len(), 1);
}

#[test]
fn a_channel_with_several_machines_needs_the_pick() {
    let (mut store, fake) = store();
    store.send(A::StartCopy(document()), |_| {});
    store.send(A::CopyChannel(ChannelId::new("infra")), |_| {});
    assert_eq!(store.state().copy_machines().len(), 2);
    assert_eq!(store.state().copy.as_ref().and_then(|d| d.machine.clone()), None);

    store.send(A::ConfirmCopy, |_| {});
    store.skip_received();
    let Some(dialog) = store.state().copy.as_ref() else {
        panic!("the dialog stays open");
    };
    assert!(dialog.error.is_some());
    assert!(!dialog.copying);
    assert!(copied_files(&fake).is_empty());

    // A machine that isn't the channel's is not taken.
    store.send(A::CopyMachine(MachineId::new("gpu-box")), |_| {});
    assert_eq!(store.state().copy.as_ref().and_then(|d| d.machine.clone()), None);

    store.send(A::CopyMachine(MachineId::new("devbox-1")), |_| {});
    store.send(A::ConfirmCopy, |_| {});
    store.receive(A::Copied(Ok(())), |_| {});
    store.skip_received();
    assert!(store.state().copy.is_none());
    let files = fake.workspace().files;
    assert!(files.iter().any(|file| {
        file.machine == MachineId::new("devbox-1") && file.path.ends_with("/shared/rfc-brin.md")
    }));
}

#[test]
fn confirming_without_a_channel_is_an_error() {
    let (mut store, fake) = store();
    store.send(A::StartCopy(document()), |_| {});
    store.send(A::ConfirmCopy, |_| {});
    store.skip_received();
    assert!(store.state().copy.as_ref().is_some_and(|d| d.error.is_some()));
    assert!(copied_files(&fake).is_empty());
}

#[test]
fn the_document_stays_on_the_desk() {
    let (mut store, fake) = store();
    store.send(A::StartCopy(document()), |_| {});
    store.send(A::CopyChannel(ChannelId::new("payments-rewrite")), |_| {});
    store.send(A::ConfirmCopy, |_| {});
    store.receive(A::Copied(Ok(())), |_| {});
    store.skip_received();

    store.send(A::Show(Arc::new(fake.workspace())), |_| {});
    assert!(
        store
            .state()
            .documents()
            .iter()
            .any(|(doc, _)| **doc == document())
    );
}

#[test]
fn cancel_closes_the_dialog() {
    let (mut store, _) = store();
    store.send(A::StartCopy(document()), |_| {});
    assert!(store.state().dialog_open());
    store.send(A::CancelDialog, |_| {});
    assert!(store.state().copy.is_none());
}

#[test]
fn only_one_dialog_is_open_at_a_time() {
    let (mut store, _) = store();
    store.send(A::StartCopy(document()), |_| {});
    store.send(A::OpenCleanUp, |_| {});
    assert!(store.state().clean_up.is_none());
    store.send(A::StartNew { agent: humata2_clients::model::Agent::Claude }, |_| {});
    assert!(store.state().new.is_none());
}

#[test]
fn the_dialog_closes_when_its_document_is_gone() {
    let (mut store, _) = store();
    store.send(A::StartCopy(document()), |_| {});
    let mut workspace = (*store.state().workspace).clone();
    workspace.desk.documents.retain(|doc| *doc != document());
    store.send(A::Show(Arc::new(workspace)), |_| {});
    assert!(store.state().copy.is_none());
}
