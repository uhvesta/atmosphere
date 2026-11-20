//! The delete sheet: the plan, the work and count gates, a plan that changed
//! under the person, and what a delete leaves behind, on the fake daemon.
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use humata2_clients::daemon::UnpushedWork;
use humata2_clients::desk::is_desk;
use humata2_clients::mass_delete::{
    DeleteScope, DeleteSessionsPlan, LeftAlone, SessionToDelete, SessionWork,
};
use humata2_clients::model::{ChannelId, ForkKind, MachineId, SessionId};
use humata2_clients::{FakeDaemon, fixture};
use humata2_mass_delete::{
    Delegate, DeleteSheet, FooterHint, Listing, MassDelete, MassDeleteAction as A,
    MassDeleteState, Row, Surface,
};
use tca::{TestClock, TestStore};

fn id(name: &str) -> SessionId {
    SessionId::new(name)
}

fn rig() -> (TestStore<MassDelete>, FakeDaemon) {
    let fake = FakeDaemon::new(fixture::standard(), Arc::new(TestClock::new()));
    let rows = fake
        .workspace()
        .sessions
        .iter()
        .map(|session| Row {
            id: session.id.clone(),
            channel: session.channel.clone(),
            machine: session.machine.clone(),
            context_fork_of: session
                .parent
                .clone()
                .filter(|_| session.fork == Some(ForkKind::Conversation)),
            archived: session.archived,
            live: !session.archived,
            desk: is_desk(session),
        })
        .collect();
    let state = MassDeleteState {
        listing: Listing { rows, ..Listing::default() },
        ..MassDeleteState::default()
    };
    let store = TestStore::new(state, MassDelete::new(Arc::new(fake.clone()))).non_exhaustive();
    (store, fake)
}

fn work(fake: &FakeDaemon, session: &str, what: &str) {
    fake.set_unpushed(&id(session), vec![("api".to_owned(), what.to_owned())]);
}

/// Selects the two scratch sessions and opens the sheet on them.
fn open_picked(store: &mut TestStore<MassDelete>) {
    store.send(A::Enter { surface: Surface::Sidebar, first: id("s-notes") }, |_| {});
    store.send(A::Toggle(id("s-old")), |_| {});
    store.send(A::OpenPicked, |_| {});
    planned(store);
}

fn planned(store: &mut TestStore<MassDelete>) {
    store.receive_matching("Planned(Ok(_))", |a| matches!(a, A::Planned(Ok(_))), |_| {});
}

fn sheet(store: &TestStore<MassDelete>) -> &DeleteSheet {
    store.state().sheet.as_ref().expect("the sheet is open")
}

#[test]
fn opening_for_the_picked_sessions_loads_their_plan() {
    let (mut store, _fake) = rig();
    open_picked(&mut store);

    let sheet = sheet(&store);
    assert!(!sheet.loading);
    assert_eq!(sheet.planned_count(), 2);
    assert!(matches!(sheet.scope, DeleteScope::Picked(_)));
    assert!(sheet.can_confirm());
    assert_eq!(sheet.hint(), Some(FooterHint::Ready));
}

#[test]
fn opening_with_nothing_picked_does_nothing() {
    let (mut store, _fake) = rig();
    store.send(A::Enter { surface: Surface::Sidebar, first: id("s-notes") }, |_| {});
    store.send(A::SelectNothing, |_| {});
    store.send(A::OpenPicked, |_| {});
    assert!(store.state().sheet.is_none());
}

#[test]
fn work_that_was_never_pushed_blocks_until_its_box_is_ticked() {
    let (mut store, fake) = rig();
    work(&fake, "s-notes", "2 commits never pushed");
    open_picked(&mut store);

    assert!(!sheet(&store).can_confirm());
    assert_eq!(sheet(&store).hint(), Some(FooterHint::TickOne(id("s-notes"))));
    store.send(A::Confirm, |_| {});
    assert!(!sheet(&store).deleting);

    store.send(A::TickWork(id("s-notes")), |_| {});
    assert!(sheet(&store).can_confirm());
    assert!(sheet(&store).delete_unpushed_work());
    // A session with nothing to lose has no box to tick.
    store.send(A::TickWork(id("s-old")), |_| {});
    assert_eq!(sheet(&store).work_ticks.len(), 1);
}

#[test]
fn delete_all_needs_the_typed_count_as_well_as_the_work_ticks() {
    let (mut store, fake) = rig();
    work(&fake, "s-notes", "2 commits never pushed");
    store.send(A::OpenAll, |_| {});
    planned(&mut store);
    let count = sheet(&store).planned_count();
    assert!(sheet(&store).needs_count());

    store.send(A::TickWork(id("s-notes")), |_| {});
    assert!(!sheet(&store).can_confirm());
    assert_eq!(sheet(&store).hint(), Some(FooterHint::Type(count)));

    // Only digits are kept.
    store.send(A::Typed(format!(" {count}x ")), |_| {});
    assert_eq!(sheet(&store).typed, count.to_string());
    assert!(sheet(&store).can_confirm());

    store.send(A::Typed(format!("{}", count + 1)), |_| {});
    assert!(!sheet(&store).can_confirm());
}

#[test]
fn the_hints_follow_what_is_still_missing() {
    let session = |name: &str, unpushed: bool| SessionToDelete {
        session: id(name),
        title: name.to_owned(),
        channel: ChannelId::new("web"),
        machine: MachineId::new("m1"),
        archived: false,
        unpushed: if unpushed {
            vec![UnpushedWork { repo: "api".to_owned(), what: "1 commit".to_owned() }]
        } else {
            Vec::new()
        },
        running_terminals: 0,
        goes_with: None,
        terminals: Vec::new(),
        exited: false,
    };
    let plan = |sessions: Vec<SessionToDelete>, left_alone: Vec<LeftAlone>| DeleteSessionsPlan {
        sessions,
        left_alone,
        forks_staying: Vec::new(),
        desk_sessions_left_out: 0,
        token: "t".to_owned(),
        needs_count: false,
    };
    let sheet = |plan| {
        let mut sheet = DeleteSheet {
            scope: DeleteScope::All,
            plan: Some(plan),
            loading: false,
            work_ticks: BTreeSet::new(),
            typed: String::new(),
            stale: None,
            deleting: false,
            error: None,
        };
        sheet.loading = false;
        sheet
    };

    let two_with_work = sheet(plan(vec![session("a", true), session("b", true), session("c", false)], Vec::new()));
    assert_eq!(
        two_with_work.hint(),
        Some(FooterHint::TickAndType { sessions: 2, count: 3 })
    );

    // Unreachable sessions are not counted.
    let unreachable = LeftAlone {
        machine: MachineId::new("gpu-box"),
        sessions: vec![SessionWork {
            session: id("z"),
            title: "z".to_owned(),
            dirty_files: 0,
            unpushed_commits: 0,
        }],
    };
    let mut counted = sheet(plan(vec![session("a", false)], vec![unreachable.clone()]));
    assert_eq!(counted.planned_count(), 1);
    counted.typed = "1".to_owned();
    assert!(counted.can_confirm());

    // Nothing reachable: delete stays off.
    let mut nothing = sheet(plan(Vec::new(), vec![unreachable]));
    nothing.typed = "0".to_owned();
    assert!(!nothing.can_confirm());
    assert_eq!(nothing.hint(), Some(FooterHint::NothingReachable));
}

#[test]
fn a_changed_plan_replaces_the_old_one_and_clears_only_the_changed_ticks() {
    let (mut store, fake) = rig();
    work(&fake, "s-notes", "2 commits never pushed");
    work(&fake, "s-old", "1 commit never pushed");
    open_picked(&mut store);
    store.send(A::TickWork(id("s-notes")), |_| {});
    store.send(A::TickWork(id("s-old")), |_| {});
    assert!(sheet(&store).can_confirm());

    // A commit arrives on s-old after the plan.
    work(&fake, "s-old", "2 commits never pushed");
    fake.change_after_plan();
    store.send(A::Confirm, |_| {});
    store.receive_matching("Deleted(Ok(_))", |a| matches!(a, A::Deleted(Ok(_))), |_| {});

    let sheet = sheet(&store);
    assert!(!sheet.deleting);
    let stale = sheet.stale.as_ref().expect("the notice shows");
    assert!(!stale.changes.is_empty());
    assert_eq!(stale.changed, BTreeSet::from([id("s-old")]));
    assert!(sheet.is_changed(&id("s-old")));
    assert_eq!(sheet.work_ticks, BTreeSet::from([id("s-notes")]));
    assert!(!sheet.can_confirm());
    assert!(fake.workspace().session(&id("s-notes")).is_some());
}

#[test]
fn a_changed_count_clears_the_typed_number() {
    let (mut store, fake) = rig();
    store.send(A::OpenAll, |_| {});
    planned(&mut store);
    let count = sheet(&store).planned_count();
    store.send(A::Typed(count.to_string()), |_| {});

    // A host drops off before the delete runs: its sessions are no longer
    // counted.
    let machine = fake
        .workspace()
        .session(&id("s-old"))
        .map(|session| session.machine.clone())
        .unwrap_or_else(|| MachineId::new("none"));
    fake.take_offline(&machine);
    store.send(A::Confirm, |_| {});
    store.receive_matching("Deleted(Ok(_))", |a| matches!(a, A::Deleted(Ok(_))), |_| {});

    assert!(sheet(&store).planned_count() < count);
    assert!(sheet(&store).typed.is_empty());
}

#[test]
fn deleting_closes_the_sheet_ends_select_mode_and_toasts() {
    let (mut store, fake) = rig();
    work(&fake, "s-notes", "2 commits never pushed");
    open_picked(&mut store);
    store.send(A::TickWork(id("s-notes")), |_| {});
    let machine = fake
        .workspace()
        .session(&id("s-notes"))
        .map(|session| session.machine.to_string())
        .unwrap_or_default();

    store.send(A::Confirm, |_| {});
    assert!(sheet(&store).deleting);
    store.receive_matching("Deleted(Ok(_))", |a| matches!(a, A::Deleted(Ok(_))), |_| {});
    let toast = Mutex::new(String::new());
    store.receive_matching(
        "Delegate(Toast(_))",
        |a| {
            if let A::Delegate(Delegate::Toast(text)) = a {
                if let Ok(mut held) = toast.lock() {
                    held.clone_from(text);
                }
                true
            } else {
                false
            }
        },
        |_| {},
    );

    assert!(store.state().sheet.is_none());
    assert!(!store.state().selecting());
    assert!(fake.workspace().session(&id("s-notes")).is_none());
    assert!(fake.workspace().session(&id("s-old")).is_none());
    let toast = toast.lock().map(|held| held.clone()).unwrap_or_default();
    assert!(toast.starts_with("Deleted 2 sessions."), "{toast}");
    assert!(toast.contains(&machine), "{toast}");
}

#[test]
fn cancelling_closes_the_sheet_and_keeps_the_selection() {
    let (mut store, _fake) = rig();
    open_picked(&mut store);
    store.send(A::Cancel, |_| {});

    assert!(store.state().sheet.is_none());
    assert!(store.state().selecting());
    assert_eq!(store.state().picked_count(), 2);
}

#[test]
fn escape_closes_the_sheet_first_and_then_leaves_select_mode() {
    let (mut store, _fake) = rig();
    open_picked(&mut store);
    store.send(A::Escape, |_| {});
    assert!(store.state().sheet.is_none());
    assert!(store.state().selecting());
    store.send(A::Escape, |_| {});
    assert!(!store.state().selecting());
}
