//! D186, LD-24: a search hit picked from the palette opens its session on a
//! read-only Conversation tab at the matching entry, in the focused tab
//! group, from the host's mirror, so it works while the machine is offline.
//! The tab and its target survive a restart and are read again.
use humata2_app::{App, AppAction as A, AppState, Overlay, Route, saved};
use humata2_clients::history::{SearchHit, TurnRole};
use humata2_clients::model::{SessionId, Status};
use humata2_clients::{Clients, Fakes, Workspace, fixture};
use humata2_find::FindAction;
use humata2_palette::PaletteAction;
use humata2_session::{ConversationTarget, SessionAction, SessionView, Tab, TabGroup};
use std::sync::Arc;
use tca::{TestClock, TestStore};

/// The turn whose text the search finds.
const HIT_TURN: u32 = 5;

fn ledger() -> SessionId {
    SessionId::new("ledger")
}

/// A ledger conversation with one piece per turn, turn 0 to 8, so a piece's
/// place in the session is its turn number; only turn 5 says "totals".
fn seed(fakes: &Fakes) {
    for turn in 0..=8 {
        let text = if turn == HIT_TURN {
            "The ledger totals reconcile now.".to_owned()
        } else {
            format!("Step {turn} of the ledger migration.")
        };
        let role = if turn % 2 == 0 { TurnRole::User } else { TurnRole::Agent };
        fakes.history.push_turn(&ledger(), turn, role, &text);
    }
}

fn store_with(workspace: Workspace, view: Option<SessionView>) -> TestStore<App> {
    let (clients, fakes) = Clients::fake(workspace.clone(), Arc::new(TestClock::new()));
    seed(&fakes);
    let mut state = AppState::launch(&clients);
    let workspace = Arc::new(workspace);
    state.workspace = workspace.clone();
    state.session.workspace = workspace;
    if let Some(view) = view {
        state.session.views.insert(ledger(), view);
    }
    TestStore::new(state, App::new(clients)).non_exhaustive()
}

/// Searches the palette for the hit and picks it; the hit it picked.
fn pick_the_hit(store: &mut TestStore<App>) -> SearchHit {
    store.send(A::OpenPalette, |_| {});
    store.send(A::Palette(PaletteAction::Query("ledger totals".to_owned())), |_| {});
    store.receive_matching(
        "the search answered",
        |a| matches!(a, A::Find(FindAction::Loaded { .. })),
        |_| {},
    );
    assert_eq!(store.state().find.hits.len(), 1, "the one matching turn is a hit");
    let hit = store.state().find.hits[0].clone();
    store.send(A::Find(FindAction::Open(0)), |_| {});
    store.receive_matching(
        "the conversation was read",
        |a| matches!(a, A::Session(SessionAction::ConversationLoaded { .. })),
        |_| {},
    );
    store.skip_received();
    hit
}

fn ledger_view(store: &TestStore<App>) -> &SessionView {
    store.state().session.views.get(&ledger()).expect("ledger's page")
}

/// The conversation landed on the hit's entry with its turn on screen.
fn assert_on_the_hit(store: &TestStore<App>, hit: &SearchHit) {
    let conversation = store
        .state()
        .session
        .conversations
        .get(&ledger())
        .expect("the session has a conversation");
    assert!(
        conversation.entries.iter().any(|entry| entry.turn == HIT_TURN),
        "the hit's turn is read"
    );
    assert_eq!(conversation.marked().map(|entry| entry.seq), Some(hit.seq), "the hit's entry is marked");
    assert_eq!(conversation.error, None);
    assert_eq!(
        ledger_view(store).conversation,
        Some(ConversationTarget { turn: HIT_TURN, seq: hit.seq })
    );
}

#[test]
fn picking_a_search_hit_opens_a_read_only_conversation_on_its_entry() {
    let mut store = store_with(fixture::design(), None);
    let before = store.state().route.clone();
    let hit = pick_the_hit(&mut store);

    assert_eq!(hit.seq, u64::from(HIT_TURN), "the hit names the entry that matched");
    assert_eq!(store.state().route, Route::Session(ledger()));
    assert!(
        !store.state().overlays.contains(&Overlay::Palette),
        "the palette closes when its hit opens"
    );
    let view = ledger_view(&store);
    assert_eq!(view.selected(view.focused_group), Some(&Tab::Conversation));
    let shells = store
        .state()
        .session
        .workspace
        .session(&ledger())
        .map_or(0, |session| session.shells.len());
    let terminals = view.groups[view.focused_group]
        .tabs
        .iter()
        .filter(|tab| matches!(tab, Tab::Terminal(_)))
        .count();
    assert_eq!(terminals, shells + 1, "the terminal tabs stay beside it");
    assert_on_the_hit(&store, &hit);

    store.send(A::Back, |_| {});
    assert_eq!(store.state().route, before, "Back returns to where the search was made");
    store.skip_in_flight();
    store.finish();
}

#[test]
fn a_hit_opens_in_the_focused_group_and_two_groups_stay_two() {
    let view = SessionView {
        groups: vec![
            TabGroup { tabs: vec![Tab::Terminal(0)], selected: 0 },
            TabGroup { tabs: vec![Tab::Terminal(1)], selected: 0 },
        ],
        focused_group: 1,
        tab: 1,
        ..Default::default()
    };
    let mut store = store_with(fixture::design(), Some(view));
    let hit = pick_the_hit(&mut store);

    let view = ledger_view(&store);
    assert_eq!(view.groups.len(), 2, "no third group, and neither group goes");
    assert_eq!(view.groups[0].tabs, vec![Tab::Terminal(0)], "the other group is untouched");
    assert_eq!(view.focused_group, 1);
    assert_eq!(view.selected(1), Some(&Tab::Conversation));
    assert!(view.groups[1].tabs.contains(&Tab::Terminal(1)));
    assert_on_the_hit(&store, &hit);
    store.skip_in_flight();
    store.finish();
}

#[test]
fn a_hit_on_an_offline_machine_opens_the_same_conversation() {
    let mut workspace = fixture::design();
    let machine = {
        let session = workspace
            .sessions
            .iter_mut()
            .find(|session| session.id == ledger())
            .expect("the design fixture has ledger");
        session.status = Status::Offline;
        session.machine.clone()
    };
    for found in workspace.machines.iter_mut().filter(|m| m.id == machine) {
        found.lost_at = Some("14:02".into());
    }
    let mut store = store_with(workspace, None);
    let hit = pick_the_hit(&mut store);

    assert_eq!(store.state().route, Route::Session(ledger()));
    let view = ledger_view(&store);
    assert_eq!(view.selected(view.focused_group), Some(&Tab::Conversation));
    assert_on_the_hit(&store, &hit);
    store.skip_in_flight();
    store.finish();
}

#[test]
fn a_saved_conversation_tab_comes_back_after_a_restart_and_reads_again() {
    let (clients, fakes) = Clients::fake(fixture::design(), Arc::new(TestClock::new()));
    seed(&fakes);
    let target = ConversationTarget { turn: HIT_TURN, seq: u64::from(HIT_TURN) };
    let view = SessionView {
        groups: vec![TabGroup {
            tabs: vec![Tab::Terminal(0), Tab::Terminal(1), Tab::Conversation],
            selected: 2,
        }],
        conversation: Some(target),
        ..Default::default()
    };
    let mut state = AppState::launch(&clients);
    state.route = Route::Session(ledger());
    state.session.views.insert(ledger(), view.clone());
    saved::Saver::new(clients.storage.clone())
        .save(1, &state.saved())
        .expect("save the page");

    let restored = AppState::launch(&clients);
    assert_eq!(
        restored.session.views.get(&ledger()),
        Some(&view),
        "the tab and its target are saved"
    );
    assert!(
        restored.session.conversations.is_empty(),
        "entries aren't saved; they are read again"
    );

    let mut store = TestStore::new(restored, App::new(clients)).non_exhaustive();
    store.send(A::Launch, |_| {});
    store.receive_matching(
        "the restored conversation was read",
        |a| matches!(a, A::Session(SessionAction::ConversationLoaded { .. })),
        |_| {},
    );
    let conversation = store
        .state()
        .session
        .conversations
        .get(&ledger())
        .expect("the restored tab has its entries");
    let turns: Vec<u32> = conversation.entries.iter().map(|entry| entry.turn).collect();
    assert_eq!(turns, (3..=7).collect::<Vec<_>>(), "read around the saved turn");
    assert_eq!(conversation.marked().map(|entry| entry.seq), Some(target.seq));
    store.skip_received();
    store.skip_in_flight();
    store.finish();
}
