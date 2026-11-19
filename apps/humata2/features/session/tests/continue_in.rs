//! Continue in another channel… (CV-8) on the session page: the dialog, its
//! channel and "Continue from" choices, and what the daemon answers.
use humata2_clients::{
    Clients, FakeDaemon, Operation, Workspace, fixture,
    model::{ChannelId, Line, Speaker, SessionId},
};
use humata2_session::{Delegate, MenuItem, SessionAction as A, SessionState, Sessions};
use std::sync::Arc;
use tca::{TestClock, TestStore};

const SECOND: &str = "Now add retries to the ledger writes.";

/// The design workspace, where "ledger" (in payments-rewrite) has a second
/// turn.
fn two_turns() -> Workspace {
    let mut workspace = fixture::design();
    let ledger = workspace
        .sessions
        .iter_mut()
        .find(|session| session.id == SessionId::new("ledger"))
        .expect("the fixture has ledger");
    ledger.transcript.push(Line {
        from: Speaker::You,
        text: SECOND.to_owned(),
    });
    ledger.transcript.push(Line {
        from: Speaker::Agent,
        text: "Retries added.".to_owned(),
    });
    workspace
}

fn store_on(workspace: Workspace, open: &str) -> (TestStore<Sessions>, FakeDaemon) {
    let (clients, fakes) = humata2_clients::fake_clients(workspace, Arc::new(TestClock::new()));
    let state = SessionState {
        workspace: Arc::new(fakes.daemon.workspace()),
        open: Some(SessionId::new(open)),
        ..Default::default()
    };
    (
        TestStore::new(state, Sessions::new(clients)).non_exhaustive(),
        fakes.daemon,
    )
}

#[test]
fn continuing_before_turn_2_starts_a_new_session_in_the_other_channel_and_opens_it() {
    let (mut store, daemon) = store_on(two_turns(), "ledger");
    let ledger = SessionId::new("ledger");
    let web = ChannelId::new("web");
    let before = daemon.workspace().session(&ledger).cloned().expect("ledger");

    store.send(A::MenuPick(MenuItem::ContinueInChannel), |_| {});
    assert!(store.state().continuing.is_some(), "the menu item opens the dialog");
    store.send(A::ContinuePick(web.clone()), |_| {});
    store.send(A::ContinueTurnPicker(true), |_| {});
    store.receive_matching("turns", |a| matches!(a, A::ContinueTurns(Ok(_))), |_| {});
    let turns = store
        .state()
        .continuing
        .as_ref()
        .and_then(|draft| draft.turns.clone())
        .expect("the turns are listed");
    assert_eq!(turns.len(), 2);
    store.send(A::ContinueBeforeTurn(Some(2)), |_| {});
    store.send(A::ConfirmContinue, |_| {});
    store.receive_matching("continued", |a| matches!(a, A::Continued(Ok(_))), |_| {});
    store.receive_matching(
        "open the new session",
        |a| matches!(a, A::Delegate(Delegate::OpenSession(_))),
        |_| {},
    );
    assert_eq!(daemon.last_continue(), Some((ledger.clone(), web.clone(), Some(2))));
    let workspace = daemon.workspace();
    let opened = workspace
        .sessions
        .iter()
        .find(|session| session.channel == web && session.title == before.title)
        .cloned()
        .expect("a new session in web");
    assert_ne!(opened.id, ledger, "under a new id");
    assert_eq!(opened.parent, None, "a top-level session");
    assert!(
        !opened.transcript.iter().any(|line| line.text == SECOND),
        "the copy stops before turn 2"
    );
    assert_eq!(
        workspace.session(&ledger),
        Some(&before),
        "the session it came from is untouched"
    );
    assert!(store.state().continuing.is_none(), "the dialog closes");
}

#[test]
fn only_another_of_the_workspaces_channels_can_be_picked() {
    let (mut store, daemon) = store_on(fixture::design(), "ledger");
    store.send(A::StartContinue, |_| {});
    store.send(A::ContinuePick(ChannelId::new("payments-rewrite")), |_| {});
    store.send(A::ContinuePick(ChannelId::new("no-such-channel")), |_| {});
    store.send(A::ContinuePick(ChannelId::new("desk")), |_| {});
    assert_eq!(store.state().continuing.as_ref().and_then(|d| d.channel.clone()), None);
    store.send(A::ConfirmContinue, |_| {});
    assert_eq!(daemon.last_continue(), None, "nothing is asked without a channel");
    store.send(A::ContinuePick(ChannelId::new("infra")), |_| {});
    assert_eq!(
        store.state().continuing.as_ref().and_then(|d| d.channel.clone()),
        Some(ChannelId::new("infra"))
    );
}

#[test]
fn nothing_comes_before_the_first_turn() {
    let (mut store, _) = store_on(two_turns(), "ledger");
    store.send(A::StartContinue, |_| {});
    store.send(A::ContinueBeforeTurn(Some(1)), |_| {});
    assert_eq!(store.state().continuing.as_ref().and_then(|d| d.before_turn), None);
    store.send(A::ContinueBeforeTurn(Some(2)), |_| {});
    store.send(A::ContinueBeforeTurn(None), |_| {});
    assert_eq!(store.state().continuing.as_ref().and_then(|d| d.before_turn), None);
}

#[test]
fn a_failed_continue_keeps_the_dialog_and_says_why() {
    let (mut store, daemon) = store_on(fixture::design(), "ledger");
    daemon.fail_next(Operation::ContinueInChannel, "link unavailable");
    store.send(A::StartContinue, |_| {});
    store.send(A::ContinuePick(ChannelId::new("web")), |_| {});
    store.send(A::ConfirmContinue, |_| {});
    store.receive_matching("continued", |a| matches!(a, A::Continued(Err(_))), |_| {});
    let draft = store.state().continuing.clone().expect("still open");
    assert_eq!(draft.channel, Some(ChannelId::new("web")));
    assert!(!draft.sending);
    assert!(store.state().error.is_some());
}

#[test]
fn a_desk_session_has_no_continue() {
    let mut workspace = fixture::design();
    fixture::desk(&mut workspace);
    let (mut store, _) = store_on(workspace, "desk-plan-q4");
    store.send(A::MenuPick(MenuItem::ContinueInChannel), |_| {});
    store.send(A::StartContinue, |_| {});
    assert!(store.state().continuing.is_none());
}

#[test]
fn cancel_or_opening_another_session_closes_the_dialog() {
    let (mut store, _) = store_on(fixture::design(), "ledger");
    store.send(A::StartContinue, |_| {});
    store.send(A::CancelContinue, |_| {});
    assert!(store.state().continuing.is_none());
    store.send(A::StartContinue, |_| {});
    store.send(A::Open(SessionId::new("rate")), |_| {});
    assert!(store.state().continuing.is_none());
}
