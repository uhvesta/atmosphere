//! IN-14, IN-25, IN-11: the grant tick, question choices, and opening the asking session.
use humata2_clients::model::{Answer, Request, RequestId};
use humata2_clients::{Clients, Fakes, fixture};
use humata2_inbox::{Delegate, Inbox, InboxAction as A, InboxState};
use std::sync::Arc;
use tca::{TestClock, TestStore};

fn setup(rule: &str) -> (TestStore<Inbox>, Fakes, Request) {
    let mut workspace = fixture::design();
    let Some(request) = workspace.requests.iter_mut().find(|r| r.answer.is_none()) else {
        panic!("the design fixture has a waiting request");
    };
    request.rule = rule.to_owned();
    let request = request.clone();
    let (clients, fakes) = Clients::fake(workspace.clone(), Arc::new(TestClock::new()));
    let state = InboxState {
        workspace: Arc::new(workspace),
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

fn answer(store: &mut TestStore<Inbox>, id: &RequestId, answer: A) {
    store.send(answer, |s| {
        s.pending.insert(id.clone());
    });
    store.receive(A::Answered(id.clone(), Ok(())), |s| {
        s.pending.remove(id);
        s.always.remove(id);
    });
}

#[test]
fn a_ticked_allow_sends_always_with_the_rule() {
    let (mut store, fakes, request) = setup("git push to origin");
    let id = request.id.clone();
    store.send(A::ToggleAlways(id.clone()), |s| {
        s.always.insert(id.clone());
    });
    answer(&mut store, &id, A::Answer(id.clone(), Answer::Approved));
    assert_eq!(
        sent(&fakes, &id),
        Some(Answer::Always {
            rule: "git push to origin".to_owned()
        })
    );
    store.finish();
}

#[test]
fn an_unticked_allow_sends_approved() {
    let (mut store, fakes, request) = setup("git push to origin");
    let id = request.id.clone();
    answer(&mut store, &id, A::Answer(id.clone(), Answer::Approved));
    assert_eq!(sent(&fakes, &id), Some(Answer::Approved));
    store.finish();
}

#[test]
fn the_tick_is_ignored_without_a_rule() {
    let (mut store, fakes, request) = setup("");
    let id = request.id.clone();
    store.send(A::ToggleAlways(id.clone()), |_| {});
    assert!(store.state().always.is_empty());
    answer(&mut store, &id, A::Answer(id.clone(), Answer::Approved));
    assert_eq!(sent(&fakes, &id), Some(Answer::Approved));
    store.finish();
}

#[test]
fn choosing_sends_the_choice_as_a_reply() {
    let (mut store, fakes, request) = setup("");
    let id = request.id.clone();
    answer(&mut store, &id, A::Choose(id.clone(), "Yes".to_owned()));
    assert_eq!(
        sent(&fakes, &id),
        Some(Answer::Replied {
            text: "Yes".to_owned()
        })
    );
    store.finish();
}

#[test]
fn open_request_names_the_session_and_the_request() {
    let (mut store, _fakes, request) = setup("");
    store.send(A::OpenRequest(request.id.clone()), |_| {});
    store.receive(
        A::Delegate(Delegate::OpenRequest {
            session: request.session.clone(),
            request: request.id.clone(),
        }),
        |_| {},
    );
    store.finish();
}
