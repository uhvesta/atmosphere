//! D379 (frame 78): a Migrate card's "What changes" folds. A card with more
//! than 3 change lines starts folded, showing the first 3 and how many more;
//! ToggleChanges unfolds it and folds it back. The fold is per item and
//! stays as the daemon updates the item.
use std::sync::Arc;

use humata2_clients::model::{
    ChannelId, MachineId, MigrateCard, MigratePhase, MigrateThing, Preview, Release, Request,
    RequestId, RequestKind, SessionId,
};
use humata2_clients::{Clients, Fakes, fixture};
use humata2_inbox::{ChangesFold, Inbox, InboxAction as A, InboxState};
use tca::{TestClock, TestStore};

fn lines(n: usize) -> Vec<String> {
    (1..=n).map(|i| format!("change {i}")).collect()
}

fn migrate(id: &str, machine: &str, changes: Vec<String>, phase: MigratePhase) -> Request {
    Request {
        id: RequestId::new(id),
        session: SessionId::new(""),
        channel: ChannelId::new(""),
        kind: RequestKind::Migrate,
        title: format!("Migrate the daemon on {machine}"),
        reason: String::new(),
        preview: Preview::Migrate(MigrateCard {
            thing: MigrateThing::Daemon,
            machine: MachineId::new(machine),
            from: Release { version: "2.0.4".to_owned(), bundle: "9f3e2d1c0b1a".to_owned() },
            to: Release { version: "2.0.5".to_owned(), bundle: "a1b2c3d4e5f6".to_owned() },
            changes,
            keeps_running: Vec::new(),
            phase,
            waiting_sessions: Vec::new(),
            migrated: None,
            steps: Vec::new(),
            holders: Vec::new(),
        }),
        doing: String::new(),
        rule: String::new(),
        undoable: false,
        asked: "1m".to_owned(),
        answer: None,
        operation: String::new(),
        steps: Vec::new(),
        why_you: String::new(),
        asked_by: None,
        undo_note: None,
        choices: Vec::new(),
        on_screen: None,
    }
}

fn store(requests: Vec<Request>) -> (TestStore<Inbox>, Fakes) {
    let mut workspace = fixture::design();
    workspace.requests = requests;
    let (clients, fakes) = Clients::fake(workspace.clone(), Arc::new(TestClock::new()));
    let mut store = TestStore::new(InboxState::default(), Inbox::new(clients)).non_exhaustive();
    store.send(A::Show(Arc::new(workspace)), |_| {});
    (store, fakes)
}

fn refresh(store: &mut TestStore<Inbox>, fakes: &Fakes) {
    store.send(A::Show(Arc::new(fakes.daemon.workspace())), |_| {});
}

fn fold(state: &InboxState, id: &str) -> ChangesFold {
    state.changes(&RequestId::new(id)).expect("a Migrate item has a change list")
}

fn folded(n: usize) -> ChangesFold {
    ChangesFold { shown: lines(3), more: n - 3 }
}

fn open(n: usize) -> ChangesFold {
    ChangesFold { shown: lines(n), more: 0 }
}

#[test]
fn more_than_three_change_lines_start_folded_to_three_and_a_count() {
    let (store, _fakes) = store(vec![
        migrate("m-four", "gpu-box", lines(4), MigratePhase::Waiting),
        migrate("m-seven", "build-box", lines(7), MigratePhase::Waiting),
    ]);
    assert_eq!(fold(store.state(), "m-four"), folded(4));
    assert_eq!(fold(store.state(), "m-seven"), folded(7));
}

#[test]
fn three_or_fewer_change_lines_never_fold() {
    let (mut store, _fakes) = store(vec![
        migrate("m-three", "gpu-box", lines(3), MigratePhase::Waiting),
        migrate("m-one", "build-box", lines(1), MigratePhase::Waiting),
    ]);
    assert_eq!(fold(store.state(), "m-three"), open(3));
    assert_eq!(fold(store.state(), "m-one"), open(1));

    store.send(A::ToggleChanges(RequestId::new("m-three")), |_| {});
    assert_eq!(fold(store.state(), "m-three"), open(3), "there is nothing to fold");
}

#[test]
fn toggle_changes_unfolds_the_card_and_folds_it_back() {
    let (mut store, _fakes) =
        store(vec![migrate("m-five", "gpu-box", lines(5), MigratePhase::Waiting)]);
    let id = RequestId::new("m-five");

    store.send(A::ToggleChanges(id.clone()), |_| {});
    assert_eq!(fold(store.state(), "m-five"), open(5));

    store.send(A::ToggleChanges(id), |_| {});
    assert_eq!(fold(store.state(), "m-five"), folded(5));
}

#[test]
fn the_fold_is_per_item() {
    let (mut store, _fakes) = store(vec![
        migrate("m-gpu", "gpu-box", lines(5), MigratePhase::Waiting),
        migrate("m-build", "build-box", lines(6), MigratePhase::Waiting),
    ]);
    store.send(A::ToggleChanges(RequestId::new("m-gpu")), |_| {});
    assert_eq!(fold(store.state(), "m-gpu"), open(5));
    assert_eq!(fold(store.state(), "m-build"), folded(6));
}

#[test]
fn an_unfolded_card_stays_unfolded_as_the_daemon_updates_it() {
    let (mut store, fakes) =
        store(vec![migrate("m-gpu", "gpu-box", lines(5), MigratePhase::Waiting)]);
    store.send(A::ToggleChanges(RequestId::new("m-gpu")), |_| {});

    // The daemon pushes the item again: it is migrating now and lists one
    // more change.
    fakes
        .daemon
        .put_request(migrate("m-gpu", "gpu-box", lines(6), MigratePhase::Migrating));
    refresh(&mut store, &fakes);
    assert_eq!(fold(store.state(), "m-gpu"), open(6));
}

#[test]
fn a_folded_card_stays_folded_as_the_daemon_updates_it() {
    let (mut store, fakes) = store(vec![
        migrate("m-gpu", "gpu-box", lines(5), MigratePhase::Waiting),
        migrate("m-build", "build-box", lines(4), MigratePhase::Waiting),
    ]);
    // Unfolding one item and refreshing doesn't unfold the other.
    store.send(A::ToggleChanges(RequestId::new("m-build")), |_| {});
    fakes
        .daemon
        .put_request(migrate("m-gpu", "gpu-box", lines(6), MigratePhase::Migrating));
    refresh(&mut store, &fakes);
    assert_eq!(fold(store.state(), "m-gpu"), folded(6));
    assert_eq!(fold(store.state(), "m-build"), open(4));
}

#[test]
fn only_migrate_items_have_a_change_list() {
    let mut workspace = fixture::design();
    let ordinary = workspace
        .requests
        .iter()
        .find(|r| r.kind != RequestKind::Migrate)
        .cloned()
        .expect("the design fixture has an ordinary request");
    workspace.requests = vec![ordinary.clone()];
    let (clients, _fakes) = Clients::fake(workspace.clone(), Arc::new(TestClock::new()));
    let mut store = TestStore::new(InboxState::default(), Inbox::new(clients)).non_exhaustive();
    store.send(A::Show(Arc::new(workspace)), |_| {});
    assert_eq!(store.state().changes(&ordinary.id), None);
    assert_eq!(store.state().changes(&RequestId::new("no-such-item")), None);
}
