//! IN-10, D467: an agent's cross-machine copy asks as one card with Approve,
//! Somewhere else… and Deny. Approve answers it; Somewhere else… sends the
//! user to the Files page on the machine the files would land on and leaves
//! the request waiting; Details opens and closes the real paths.
use humata2_clients::model::{Answer, CopyCard, MachineId, Preview, Request, RequestId};
use humata2_clients::{Clients, Fakes, Reveal, RevealError, fixture};
use humata2_inbox::{Delegate, Inbox, InboxAction as A, InboxState};
use std::sync::Arc;
use tca::{TestClock, TestStore};

/// A Finder that refuses every path, as `open -R` does for a path that is gone.
struct RefusingFinder;

impl Reveal for RefusingFinder {
    fn reveal(&self, path: &str) -> Result<(), RevealError> {
        Err(RevealError::Refused {
            path: path.to_owned(),
            status: "exit status: 1".to_owned(),
        })
    }
}

fn card() -> CopyCard {
    CopyCard {
        from: MachineId::new("mac"),
        to: MachineId::new("gpu-box"),
        source_path: "datasets/".to_owned(),
        destination_path: "/home/ml/Humata/shared".to_owned(),
        place: "ml shared".to_owned(),
        bytes: 2_100_000_000,
        files: 3,
    }
}

fn setup() -> (TestStore<Inbox>, Fakes, Request, Request) {
    let mut workspace = fixture::design();
    let mut copy = workspace
        .requests
        .iter()
        .find(|r| r.answer.is_none())
        .cloned()
        .expect("the design fixture has a waiting request");
    let other = copy.clone();
    copy.id = RequestId::new("rq_copy");
    copy.preview = Preview::CrossMachineCopy(card());
    workspace.requests.push(copy.clone());
    let (clients, fakes) = Clients::fake(workspace.clone(), Arc::new(TestClock::new()));
    let state = InboxState {
        workspace: Arc::new(workspace),
        ..InboxState::default()
    };
    (TestStore::new(state, Inbox::new(clients)), fakes, copy, other)
}

#[test]
fn approving_a_copy_answers_it_approved() {
    let (mut store, fakes, copy, _) = setup();
    let id = copy.id.clone();
    store.send(A::Answer(id.clone(), Answer::Approved), |s| {
        s.pending.insert(id.clone());
    });
    store.receive(A::Answered(id.clone(), Ok(())), |s| {
        s.pending.remove(&id);
    });
    let answer = fakes
        .daemon
        .workspace()
        .requests
        .iter()
        .find(|r| r.id == id)
        .and_then(|r| r.answer.clone());
    assert_eq!(answer, Some(Answer::Approved));
    store.finish();
}

#[test]
fn somewhere_else_opens_the_destination_machine_and_leaves_the_request_waiting() {
    let (mut store, fakes, copy, _) = setup();
    store.send(A::SomewhereElse(copy.id.clone()), |_| {});
    store.receive(A::Delegate(Delegate::OpenDestination(MachineId::new("gpu-box"))), |_| {});
    let answer = fakes
        .daemon
        .workspace()
        .requests
        .iter()
        .find(|r| r.id == copy.id)
        .and_then(|r| r.answer.clone());
    assert_eq!(answer, None);
    store.finish();
}

#[test]
fn somewhere_else_on_any_other_request_does_nothing() {
    let (mut store, _, _, other) = setup();
    store.send(A::SomewhereElse(other.id.clone()), |_| {});
    store.finish();
}

#[test]
fn details_open_and_close_on_a_copy_and_never_on_another_request() {
    let (mut store, _, copy, other) = setup();
    let id = copy.id.clone();
    store.send(A::ToggleDetails(id.clone()), |s| {
        s.details.insert(id.clone());
    });
    store.send(A::ToggleDetails(id.clone()), |s| {
        s.details.remove(&id);
    });
    store.send(A::ToggleDetails(other.id.clone()), |_| {});
    store.finish();
}

#[test]
fn what_is_copied_is_the_one_name_or_the_count() {
    assert_eq!(card().what(), "datasets");
    let many = CopyCard { source_path: "a/x\nb/y\nc".to_owned(), ..card() };
    assert_eq!(many.what(), "3 items");
}

#[test]
fn copy_path_puts_the_path_on_the_clipboard() {
    let (mut store, fakes, _, _) = setup();
    store.send(A::CopyPath("/home/ml/Humata/shared".to_owned()), |_| {});
    assert_eq!(fakes.clipboard.copied().as_deref(), Some("/home/ml/Humata/shared"));
    store.finish();
}

#[test]
fn reveal_on_this_mac_asks_finder_for_the_path() {
    let (mut store, fakes, _, _) = setup();
    store.send(A::Reveal(MachineId::new("this-mac"), "/Users/me/Humata/datasets".to_owned()), |_| {});
    store.skip_received();
    assert_eq!(fakes.reveal.revealed(), ["/Users/me/Humata/datasets"]);
}

#[test]
fn reveal_on_another_machine_asks_finder_for_nothing() {
    let (mut store, fakes, _, _) = setup();
    store.send(A::Reveal(MachineId::new("gpu-box"), "/home/ml/Humata/shared".to_owned()), |_| {});
    store.skip_received();
    assert!(fakes.reveal.revealed().is_empty());
}

#[test]
fn a_finder_that_refuses_a_path_shows_an_error() {
    let workspace = fixture::design();
    let (mut clients, _) = Clients::fake(workspace.clone(), Arc::new(TestClock::new()));
    let refusing: Arc<dyn Reveal> = Arc::new(RefusingFinder);
    clients.reveal = refusing;
    let state = InboxState {
        workspace: Arc::new(workspace),
        ..InboxState::default()
    };
    let mut store = TestStore::new(state, Inbox::new(clients));
    store.send(A::Reveal(MachineId::new("this-mac"), "/Users/me/gone".to_owned()), |_| {});
    store.skip_received();
    assert!(store.state().error.is_some());
}
