//! The Agent inbox: Ask why, the channel chips, History, and No with a note.
use humata2_clients::model::{Answer, AskedWhy, Request, RequestId};
use humata2_clients::{Clients, Fakes, fixture};
use humata2_inbox::{Filter, Inbox, InboxAction as A, InboxState};
use std::sync::Arc;
use tca::{TestClock, TestStore};

fn setup() -> (TestStore<Inbox>, Fakes, Request) {
    let workspace = fixture::design();
    let request = workspace
        .requests
        .iter()
        .find(|r| r.answer.is_none() && r.asked_why.is_none() && r.kind != humata2_clients::model::RequestKind::Question)
        .cloned()
        .unwrap_or_else(|| panic!("the design fixture has a waiting request"));
    let (clients, fakes): (Clients, Fakes) =
        humata2_clients::fake_clients(workspace.clone(), Arc::new(TestClock::new()));
    let state = InboxState {
        workspace: Arc::new(workspace),
        ..InboxState::default()
    };
    (TestStore::new(state, Inbox::new(clients)), fakes, request)
}

#[test]
fn ask_why_updates_the_card_and_keeps_it_pending() {
    let (mut store, fakes, request) = setup();
    let id = request.id.clone();
    store.send(A::AskWhy(id.clone()), |s| {
        s.asking.insert(id.clone());
    });
    let mut asked = request.clone();
    asked.asked_why = Some(AskedWhy {
        asked: "now".to_owned(),
        reply: None,
        replied: None,
    });
    let held = asked.clone();
    store.receive(A::AskedWhy(id.clone(), Ok(asked)), |s| {
        s.asking.remove(&id);
        let workspace = Arc::make_mut(&mut s.workspace);
        if let Some(found) = workspace.requests.iter_mut().find(|r| r.id == id) {
            *found = held.clone();
        }
    });
    let shown = store.state().workspace.requests.iter().find(|r| r.id == id).cloned();
    let shown = shown.unwrap_or_else(|| panic!("the request is still there"));
    assert!(shown.asked_why.is_some());
    assert!(shown.answer.is_none());
    assert_eq!(fakes.daemon.asked_why(), vec![id]);
    store.finish();
}

#[test]
fn a_channel_chip_narrows_the_cards() {
    let (mut store, _, request) = setup();
    let channel = request.channel.clone();
    store.send(A::SetFilter(Filter::Channel(channel.clone())), |s| {
        s.filter = Filter::Channel(channel.clone());
        s.selected = s.shown().first().map(|r| r.id.clone());
    });
    let shown = store.state().shown();
    assert!(!shown.is_empty());
    assert!(shown.iter().all(|r| r.channel == channel && r.answer.is_none()));
    let chips = store.state().chips();
    assert!(chips.iter().any(|(id, count)| *id == channel && *count > 0));
    store.finish();
}

#[test]
fn history_shows_the_answered_cards_and_back() {
    let (mut store, _, _) = setup();
    store.send(A::ToggleHistory, |s| {
        s.show_answered = true;
        s.answered_only = true;
        s.selected = s.shown().first().map(|r| r.id.clone());
    });
    let shown = store.state().shown();
    assert!(!shown.is_empty());
    assert!(shown.iter().all(|r| r.answer.is_some()));
    store.send(A::ToggleHistory, |s| {
        s.show_answered = false;
        s.answered_only = false;
        s.selected = s.shown().first().map(|r| r.id.clone());
    });
    assert!(store.state().shown().iter().all(|r| r.answer.is_none()));
    store.finish();
}

#[test]
fn no_with_a_note_sends_the_note() {
    let (mut store, fakes, request) = setup();
    let id: RequestId = request.id.clone();
    store.send(A::ReplyDraft(id.clone(), "use the replica".to_owned()), |s| {
        s.reply = Some((id.clone(), "use the replica".to_owned()));
    });
    store.send(A::SendReply, |s| {
        s.reply = None;
        s.pending.insert(id.clone());
    });
    store.receive(A::Answered(id.clone(), Ok(())), |s| {
        s.pending.remove(&id);
    });
    let answered = fakes
        .daemon
        .answers()
        .into_iter()
        .find(|(found, _)| *found == id)
        .map(|(_, answer)| answer);
    assert_eq!(answered, Some(Answer::Denied { reply: Some("use the replica".to_owned()) }));
    store.finish();
}

#[test]
fn today_summary_omits_zero_rule_count_and_pluralizes_nonzero_counts() {
    assert_eq!(humata2_inbox::today_summary(2, 0), "Today: 2 answered ·");
    assert_eq!(
        humata2_inbox::today_summary(0, 1),
        "Today: 0 answered · 1 action followed your rules without asking ·"
    );
    assert_eq!(
        humata2_inbox::today_summary(2, 31),
        "Today: 2 answered · 31 actions followed your rules without asking ·"
    );
}
