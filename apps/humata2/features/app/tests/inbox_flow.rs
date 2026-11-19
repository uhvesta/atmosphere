//! Flow 26.4, IN-11, IN-12, IN-14: open the requester from the Inbox, allow
//! there, with or without "Always", on the fakes.
use humata2_app::{App, AppAction as A, AppState, Route};
use humata2_clients::{
    Clients, Event, Fakes, fixture,
    model::{Answer, Capability, ChannelId, RequestId, SessionId},
};
use humata2_inbox::{Delegate as I, InboxAction};
use humata2_session::Panel;
use std::sync::Arc;
use tca::{TestClock, TestStore};

fn setup(
    edit: impl FnOnce(&mut humata2_clients::Workspace),
) -> (TestStore<App>, Fakes, Arc<humata2_clients::Workspace>) {
    let mut workspace = fixture::design();
    edit(&mut workspace);
    let workspace = Arc::new(workspace);
    let (clients, fakes) = humata2_clients::fake_clients((*workspace).clone(), Arc::new(TestClock::new()));
    let mut state = AppState::launch(&clients);
    state.workspace = workspace.clone();
    state.inbox.workspace = workspace.clone();
    state.session.workspace = workspace.clone();
    state.route = Route::Inbox;
    let store = TestStore::new(state, App::new(clients)).non_exhaustive();
    (store, fakes, workspace)
}

fn open_requester(store: &mut TestStore<App>, id: &RequestId) {
    store.send(A::Inbox(InboxAction::Select(id.clone())), |_| {});
    store.send(A::Inbox(InboxAction::OpenRequest(id.clone())), |_| {});
    store.receive_matching(
        "open the requester",
        |a| matches!(a, A::Inbox(InboxAction::Delegate(I::OpenRequest { .. }))),
        |_| {},
    );
    store.skip_received();
}

/// Sends the answer, waits for the daemon to take it, and feeds the app the
/// request as the daemon now holds it, as its event stream would.
fn allow(store: &mut TestStore<App>, fakes: &Fakes, id: &RequestId) {
    store.send(
        A::Inbox(InboxAction::Answer(id.clone(), Answer::Approved)),
        |_| {},
    );
    store.receive_matching(
        "daemon took the answer",
        |a| matches!(a, A::Inbox(InboxAction::Answered(_, Ok(())))),
        |_| {},
    );
    let request = fakes
        .daemon
        .workspace()
        .requests
        .into_iter()
        .find(|r| &r.id == id)
        .expect("the request is still in the daemon");
    store.send(A::Event(Event::Request(request)), |_| {});
    store.skip_received();
}

#[test]
fn open_session_lands_on_the_requesters_inbox_panel_with_the_request_selected() {
    let (mut store, _fakes, _) = setup(|_| {});
    let id = RequestId::new("benchmark-command");
    open_requester(&mut store, &id);
    let state = store.state();
    assert_eq!(state.route, Route::Session(SessionId::new("benchmark")));
    assert_eq!(state.panel(), Some(Panel::Inbox));
    assert_eq!(state.inbox.selected, Some(id));
}

#[test]
fn allowing_after_opening_answers_it_and_types_nothing_into_the_terminal() {
    let (mut store, fakes, _) = setup(|_| {});
    let id = RequestId::new("benchmark-command");
    let waiting_before = store.state().workspace.waiting().count();
    open_requester(&mut store, &id);
    let calls_before = fakes.daemon.calls();
    allow(&mut store, &fakes, &id);

    let answered = fakes
        .daemon
        .workspace()
        .requests
        .into_iter()
        .find(|r| r.id == id)
        .and_then(|r| r.answer);
    assert!(matches!(answered, Some(Answer::Approved)));
    let state = store.state();
    assert_eq!(state.workspace.waiting().count(), waiting_before - 1);
    assert!(state.workspace.waiting().all(|r| r.id != id));
    // IN-12: the only new daemon call is the answer; no terminal or session input.
    let new: Vec<_> = fakes.daemon.calls()[calls_before.len()..].to_vec();
    assert!(
        new.iter().all(|call| {
            !call.starts_with("write ")
                && !call.starts_with("terminal input ")
                && !call.starts_with("send ")
        }),
        "unexpected input calls: {new:?}"
    );
}

#[test]
fn allowing_with_always_ticked_records_a_grant_for_the_requesting_session() {
    let channel = ChannelId::new("payments-rewrite");
    let capability = Capability::new("channel_board_edit_others");
    let (mut store, fakes, _) = setup(|workspace| {
        let request = workspace
            .requests
            .iter_mut()
            .find(|r| r.id.as_str() == "move-pay-3")
            .expect("the design fixture has the board move request");
        request.asked_by = Some(humata2_clients::model::AskedBy {
            capability: Capability::new("channel_board_edit_others"),
            label: "Channel board: edit, move or delete anyone else's items".to_owned(),
            setting: humata2_clients::model::RuleSetting::Ask,
        });
    });
    let id = RequestId::new("move-pay-3");
    let grants = |rules: &std::collections::BTreeMap<ChannelId, Vec<humata2_clients::model::AccessRule>>| {
        rules[&channel]
            .iter()
            .find(|rule| rule.capability == capability)
            .map(|rule| rule.grants.clone())
            .unwrap_or_default()
    };
    let before = grants(&store.state().workspace.rules).len();

    open_requester(&mut store, &id);
    store.send(A::Inbox(InboxAction::ToggleAlways(id.clone())), |_| {});
    assert!(store.state().inbox.always.contains(&id));
    store.send(
        A::Inbox(InboxAction::Answer(id.clone(), Answer::Approved)),
        |_| {},
    );
    store.receive_matching(
        "daemon took the answer",
        |a| matches!(a, A::Inbox(InboxAction::Answered(_, Ok(())))),
        |_| {},
    );
    let held = fakes.daemon.workspace();
    let answered = held
        .requests
        .iter()
        .find(|r| r.id == id)
        .and_then(|r| r.answer.clone());
    assert!(matches!(answered, Some(Answer::Always { .. })));

    store.send(
        A::Event(Event::Rules {
            channel: channel.clone(),
            rules: held.rules[&channel].clone(),
        }),
        |_| {},
    );
    store.skip_received();
    let after = grants(&store.state().workspace.rules);
    assert_eq!(after.len(), before + 1);
    assert!(after.iter().any(|g| g.session == SessionId::new("ledger")));
}
