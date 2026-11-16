//! D186: a search hit opens one read-only Conversation tab in its session,
//! read from the host's mirror around the hit's turn, landing on the hit's
//! entry. Earlier and later turns come five at a time; a failed read can be
//! retried; the target is kept with the session's page and read again after
//! a restore.
use std::collections::BTreeMap;
use std::sync::Arc;

use humata2_clients::history::{Call, FakeHistory, TurnRole};
use humata2_clients::{Clients, DaemonError, fixture, model::SessionId};
use humata2_session::{
    Conversation, ConversationTarget, SessionAction as A, SessionState, SessionView, Sessions, Tab,
    TabGroup,
};
use tca::{TestClock, TestStore};

/// The last turn the fake's ledger conversation has.
const LAST_TURN: u32 = 40;

fn ledger() -> SessionId {
    SessionId::new("ledger")
}

/// A ledger conversation with one piece per turn, turn 0 to 40, so a piece's
/// place in the session (`seq`) is its turn number.
fn store() -> (TestStore<Sessions>, FakeHistory) {
    let mut workspace = fixture::design();
    for s in &mut workspace.sessions {
        s.unread = 0;
        s.recovery = None;
    }
    let (clients, fakes) = Clients::fake(workspace.clone(), Arc::new(TestClock::new()));
    for turn in 0..=LAST_TURN {
        let role = if turn % 2 == 0 { TurnRole::User } else { TurnRole::Agent };
        fakes.history.push_turn(&ledger(), turn, role, &format!("Step {turn} of the ledger migration."));
    }
    let mut state = SessionState {
        workspace: Arc::new(workspace),
        ..Default::default()
    };
    state.open = Some(ledger());
    state.views.insert(
        ledger(),
        SessionView {
            groups: vec![TabGroup {
                tabs: vec![Tab::Terminal(0), Tab::Terminal(1)],
                selected: 0,
            }],
            ..Default::default()
        },
    );
    (
        TestStore::new(state, Sessions::new(clients)).non_exhaustive(),
        fakes.history,
    )
}

fn target(turn: u32) -> ConversationTarget {
    ConversationTarget { turn, seq: u64::from(turn) }
}

fn open_hit(store: &mut TestStore<Sessions>, turn: u32) {
    store.send(
        A::OpenConversation {
            session: ledger(),
            turn,
            seq: u64::from(turn),
        },
        |_| {},
    );
}

fn receive_read(store: &mut TestStore<Sessions>) {
    store.receive_matching(
        "the mirror answered",
        |a| matches!(a, A::ConversationLoaded { .. }),
        |_| {},
    );
}

fn conversation(store: &TestStore<Sessions>) -> &Conversation {
    store
        .state()
        .conversations
        .get(&ledger())
        .expect("the session has a conversation")
}

fn turns(store: &TestStore<Sessions>) -> Vec<u32> {
    conversation(store).entries.iter().map(|entry| entry.turn).collect()
}

/// The turn ranges asked of the mirror, oldest first.
fn reads(history: &FakeHistory) -> Vec<(u32, u32)> {
    history
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            Call::Read { first_turn, last_turn, .. } => Some((first_turn, last_turn)),
            Call::Search { .. } => None,
        })
        .collect()
}

fn conversation_tabs(view: &SessionView) -> usize {
    view.groups
        .iter()
        .flat_map(|group| group.tabs.iter())
        .filter(|tab| **tab == Tab::Conversation)
        .count()
}

#[test]
fn a_second_hit_in_the_same_session_retargets_its_one_conversation_tab() {
    let (mut store, _) = store();
    open_hit(&mut store, 10);
    receive_read(&mut store);
    open_hit(&mut store, 30);
    receive_read(&mut store);

    let view = store.state().view().expect("ledger's page");
    assert_eq!(conversation_tabs(view), 1, "one Conversation tab per session");
    assert_eq!(view.selected(view.focused_group), Some(&Tab::Conversation));
    assert_eq!(view.conversation, Some(target(30)), "the tab now aims at the second hit");
    assert_eq!(turns(&store), (28..=32).collect::<Vec<_>>(), "only the second hit's turns show");
    assert_eq!(conversation(&store).marked().map(|entry| entry.seq), Some(30));
}

#[test]
fn the_first_read_is_the_hit_turn_and_two_turns_either_side() {
    let (mut store, history) = store();
    open_hit(&mut store, 20);
    receive_read(&mut store);

    assert_eq!(reads(&history), vec![(18, 22)]);
    assert_eq!(turns(&store), (18..=22).collect::<Vec<_>>());
    let conversation = conversation(&store);
    assert_eq!(conversation.marked().map(|entry| entry.seq), Some(20), "the hit's entry is marked");
    assert_eq!(conversation.error, None);
    assert!(conversation.can_show_earlier());
    assert!(conversation.can_show_later());
}

#[test]
fn show_earlier_turns_adds_five_before_in_order_until_turn_zero() {
    let (mut store, history) = store();
    open_hit(&mut store, 20);
    receive_read(&mut store);

    store.send(A::ShowEarlierTurns, |_| {});
    receive_read(&mut store);
    assert_eq!(turns(&store), (13..=22).collect::<Vec<_>>(), "five earlier turns, in order");

    for _ in 0..3 {
        assert!(conversation(&store).can_show_earlier());
        store.send(A::ShowEarlierTurns, |_| {});
        receive_read(&mut store);
    }
    assert_eq!(turns(&store), (0..=22).collect::<Vec<_>>());
    assert_eq!(reads(&history), vec![(18, 22), (13, 17), (8, 12), (3, 7), (0, 2)]);
    assert!(
        !conversation(&store).can_show_earlier(),
        "nothing comes before turn 0, so Show earlier turns goes"
    );

    store.send(A::ShowEarlierTurns, |_| {});
    store.settle();
    assert_eq!(reads(&history).len(), 5, "at turn 0 it asks for nothing more");
    assert_eq!(conversation(&store).marked().map(|entry| entry.seq), Some(20), "still on the hit");
}

#[test]
fn show_later_turns_adds_five_after_until_a_read_comes_back_empty() {
    let (mut store, history) = store();
    open_hit(&mut store, 20);
    receive_read(&mut store);

    store.send(A::ShowLaterTurns, |_| {});
    receive_read(&mut store);
    assert_eq!(turns(&store), (18..=27).collect::<Vec<_>>(), "five later turns, in order");

    // 28–32, 33–37, then 38–42, which holds the last three turns.
    for _ in 0..3 {
        store.send(A::ShowLaterTurns, |_| {});
        receive_read(&mut store);
    }
    assert_eq!(turns(&store), (18..=LAST_TURN).collect::<Vec<_>>());
    assert!(
        conversation(&store).can_show_later(),
        "a short read isn't known to be the end"
    );

    store.send(A::ShowLaterTurns, |_| {});
    receive_read(&mut store);
    assert_eq!(
        reads(&history),
        vec![(18, 22), (23, 27), (28, 32), (33, 37), (38, 42), (43, 47)]
    );
    assert_eq!(turns(&store), (18..=LAST_TURN).collect::<Vec<_>>());
    assert!(
        !conversation(&store).can_show_later(),
        "an empty read means there is nothing later yet, so Show later turns goes"
    );
}

#[test]
fn a_hit_at_turn_zero_or_one_reads_from_the_start() {
    for (turn, window) in [(0, (0, 2)), (1, (0, 3))] {
        let (mut store, history) = store();
        open_hit(&mut store, turn);
        receive_read(&mut store);

        assert_eq!(reads(&history), vec![window], "turn {turn}");
        assert_eq!(turns(&store), (window.0..=window.1).collect::<Vec<_>>(), "turn {turn}");
        assert_eq!(conversation(&store).marked().map(|entry| entry.seq), Some(u64::from(turn)));
        assert!(!conversation(&store).can_show_earlier(), "turn {turn} starts at the beginning");
    }
}

#[test]
fn a_failed_read_shows_an_error_and_retry_reads_again() {
    let (mut store, history) = store();
    history.fail_next(DaemonError::Offline {
        machine: humata2_clients::model::MachineId::new("devbox-1"),
    });
    open_hit(&mut store, 20);
    receive_read(&mut store);

    let view = store.state().view().expect("ledger's page");
    assert_eq!(view.selected(view.focused_group), Some(&Tab::Conversation), "the tab opens anyway");
    assert!(conversation(&store).error.is_some(), "the failure is shown");
    assert!(conversation(&store).entries.is_empty());

    store.send(A::RetryConversation, |_| {});
    receive_read(&mut store);
    assert_eq!(reads(&history), vec![(18, 22), (18, 22)], "Retry asks for the same turns");
    assert_eq!(conversation(&store).error, None);
    assert_eq!(turns(&store), (18..=22).collect::<Vec<_>>());
    assert_eq!(conversation(&store).marked().map(|entry| entry.seq), Some(20));
}

#[test]
fn a_reply_for_a_hit_picked_before_another_is_ignored() {
    let (mut store, history) = store();
    history.hold_read();
    open_hit(&mut store, 10);
    open_hit(&mut store, 30);
    store.receive_matching(
        "the second hit's read answered",
        |a| matches!(a, A::ConversationLoaded { target: t, .. } if *t == target(30)),
        |_| {},
    );

    history.release();
    store.receive_matching(
        "the first hit's read answered late",
        |a| matches!(a, A::ConversationLoaded { target: t, .. } if *t == target(10)),
        |_| {},
    );
    assert_eq!(conversation(&store).target, target(30));
    assert_eq!(turns(&store), (28..=32).collect::<Vec<_>>(), "the late reply changed nothing");
    assert_eq!(conversation(&store).marked().map(|entry| entry.seq), Some(30));
}

#[test]
fn closing_the_conversation_tab_forgets_its_target() {
    let (mut store, _) = store();
    open_hit(&mut store, 20);
    receive_read(&mut store);
    let view = store.state().view().expect("ledger's page");
    let group = view
        .groups
        .iter()
        .position(|g| g.tabs.contains(&Tab::Conversation))
        .expect("a group holds the tab");
    let index = view.groups[group]
        .tabs
        .iter()
        .position(|tab| *tab == Tab::Conversation)
        .expect("the tab is in its group");

    store.send(A::CloseTab { group, index }, |_| {});
    let view = store.state().view().expect("ledger's page");
    assert_eq!(conversation_tabs(view), 0);
    assert_eq!(view.conversation, None, "the target isn't kept once the tab is closed");
    assert!(!store.state().conversations.contains_key(&ledger()), "its entries are let go");
    assert!(view.groups[0].tabs.contains(&Tab::Terminal(0)), "the terminals stay");
}

#[test]
fn a_restored_conversation_tab_reads_its_entries_again() {
    let (mut store, history) = store();
    let view = SessionView {
        groups: vec![TabGroup {
            tabs: vec![Tab::Terminal(0), Tab::Terminal(1), Tab::Conversation],
            selected: 2,
        }],
        conversation: Some(target(20)),
        ..Default::default()
    };
    store.send(A::Restore(BTreeMap::from([(ledger(), view)])), |_| {});
    store.send(A::Open(ledger()), |_| {});
    receive_read(&mut store);

    assert_eq!(reads(&history), vec![(18, 22)], "the restored target is read around its turn");
    assert_eq!(turns(&store), (18..=22).collect::<Vec<_>>());
    assert_eq!(conversation(&store).target, target(20));
    assert_eq!(conversation(&store).marked().map(|entry| entry.seq), Some(20));
    let view = store.state().view().expect("ledger's page");
    assert_eq!(view.selected(0), Some(&Tab::Conversation));
}
