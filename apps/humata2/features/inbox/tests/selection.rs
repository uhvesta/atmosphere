//! IN-3/7: default selection and Waiting/Answered/All are scoped to visible requests.
use humata2_clients::{Clients, fixture, model::Answer};
use humata2_inbox::{History, Inbox, InboxAction as A, InboxState};
use std::sync::Arc;
use tca::{TestClock, TestStore};

#[test]
fn filters_select_the_first_visible_request_and_a_refresh_advances_it() {
    let mut workspace = fixture::design();
    // The last request still waiting: the design workspace ends with two answered ones.
    let answered = workspace
        .requests
        .iter_mut()
        .rfind(|r| r.answer.is_none())
        .unwrap();
    answered.answer = Some(Answer::Approved);
    let last = answered.id.clone();
    let first = workspace.requests[0].id.clone();
    let (clients, _) = Clients::fake(workspace.clone(), Arc::new(TestClock::new()));
    let mut store = TestStore::new(InboxState::default(), Inbox::new(clients));
    let workspace = Arc::new(workspace);
    store.send(A::Show(workspace.clone()), |s| {
        s.workspace = workspace.clone();
        s.selected = Some(first.clone());
    });
    assert!(store.state().shown().iter().all(|r| r.answer.is_none()));
    store.send(A::SetHistory(History::Answered), |s| {
        s.show_answered = true;
        s.answered_only = true;
        s.selected = Some(last.clone());
    });
    assert_eq!(
        store.state().shown().len(),
        workspace
            .requests
            .iter()
            .filter(|r| r.answer.is_some())
            .count()
    );
    store.send(A::SetHistory(History::All), |s| s.answered_only = false);
    assert_eq!(store.state().shown().len(), workspace.requests.len());
    store.send(A::SetHistory(History::Waiting), |s| {
        s.show_answered = false;
        s.selected = Some(first.clone());
    });
    let mut next = (*workspace).clone();
    next.requests[0].answer = Some(Answer::Approved);
    let second = next.requests[1].id.clone();
    let next = Arc::new(next);
    store.send(A::Show(next.clone()), |s| {
        s.workspace = next;
        s.selected = Some(second);
    });
    assert_ne!(store.state().selected, Some(first));
    store.finish();
}
