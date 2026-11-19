//! D781: a question read off its agent's screen offers its choices and
//! Dismiss, and names the terminal its detail shows while it waits.
use humata2_clients::model::{Answer, Request, RequestId, RequestKind};
use humata2_clients::{Clients, Fakes, fixture};
use humata2_inbox::{Inbox, InboxAction as A, InboxState, Offer};
use std::sync::Arc;
use tca::{TestClock, TestStore};

/// A workspace whose first waiting request is a question on tab 2 of its
/// session's screen.
fn setup() -> (TestStore<Inbox>, Fakes, Request) {
    let mut workspace = fixture::design();
    let Some(request) = workspace.requests.iter_mut().find(|r| r.answer.is_none()) else {
        panic!("the design fixture has a waiting request");
    };
    request.kind = RequestKind::Question;
    request.choices = vec!["Yes, proceed".to_owned(), "No, exit".to_owned()];
    request.on_screen = Some(2);
    let request = request.clone();
    let (clients, fakes) = humata2_clients::fake_clients(workspace.clone(), Arc::new(TestClock::new()));
    let state = InboxState {
        workspace: Arc::new(workspace),
        selected: Some(request.id.clone()),
        ..InboxState::default()
    };
    (TestStore::new(state, Inbox::new(clients)), fakes, request)
}

fn sent(fakes: &Fakes, id: &RequestId) -> Option<Answer> {
    fakes
        .daemon
        .workspace()
        .requests
        .iter()
        .find(|r| &r.id == id)
        .and_then(|r| r.answer.clone())
}

#[test]
fn a_question_on_its_screen_offers_dismiss_and_not_deny_or_reply() {
    let (store, _, request) = setup();
    assert_eq!(store.state().offers(&request.id), vec![Offer::Dismiss]);
}

#[test]
fn dismiss_sends_one_deny_with_no_reply() {
    let (store, fakes, request) = setup();
    let mut store = store.non_exhaustive();
    let id = request.id.clone();
    store.send(A::Dismiss(id.clone()), |_| {});
    // A second click while the first is on its way sends nothing more.
    store.send(A::Dismiss(id.clone()), |_| {});
    store.settle();
    store.skip_received();
    assert_eq!(sent(&fakes, &id), Some(Answer::Denied { reply: None }));
    assert_eq!(fakes.daemon.answers().len(), 1, "{:?}", fakes.daemon.answers());
}

#[test]
fn the_detail_shows_the_question_s_terminal_only_while_it_waits() {
    let (_, _, request) = setup();
    let mut state = InboxState {
        workspace: Arc::new({
            let mut workspace = fixture::design();
            for each in &mut workspace.requests {
                if each.id == request.id {
                    *each = request.clone();
                }
            }
            workspace
        }),
        selected: Some(request.id.clone()),
        ..InboxState::default()
    };
    assert_eq!(state.terminal(), Some((request.session.clone(), 2)));

    let mut answered = (*state.workspace).clone();
    for each in &mut answered.requests {
        if each.id == request.id {
            each.answer = Some(Answer::Denied { reply: None });
        }
    }
    state.workspace = Arc::new(answered);
    state.show_answered = true;
    assert_eq!(state.terminal(), None, "an answered item shows no terminal");
}

#[test]
fn an_ordinary_request_shows_no_terminal() {
    let workspace = fixture::design();
    let Some(request) = workspace.requests.iter().find(|r| r.answer.is_none()).cloned() else {
        panic!("the design fixture has a waiting request");
    };
    let state = InboxState {
        workspace: Arc::new(workspace),
        selected: Some(request.id.clone()),
        ..InboxState::default()
    };
    assert_eq!(state.terminal(), None);
}

#[test]
fn a_notice_that_the_agent_waits_in_its_terminal_offers_only_dismiss() {
    let (_, _, request) = setup();
    let mut notice = request;
    notice.kind = RequestKind::Command;
    notice.choices = Vec::new();
    notice.on_screen = None;
    notice.dismiss_only = true;
    let mut workspace = fixture::design();
    workspace.requests = vec![notice.clone()];
    let state = InboxState {
        workspace: Arc::new(workspace),
        selected: Some(notice.id.clone()),
        ..InboxState::default()
    };
    assert_eq!(state.offers(&notice.id), vec![Offer::Dismiss]);
    assert!(!notice.answered_in_bulk());
}
