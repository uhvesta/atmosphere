//! UP-1, D281, D374 (frames 77–81): a Migrate item in the Inbox. It comes
//! from Humata, not a session, so it is VIP and sits in its own "Humata"
//! group. Its one answer is Migrate: no Allow, Always, Deny or reply, and
//! Allow all / Deny all leave it out. Migrate sends Approve and shows
//! Migrating; the daemon's update turns it Failed (Migrate again) or
//! Migrated, and a mismatch that went away withdraws it.
//!
//! An old local daemon can't hold a Migrate item, so the app holds its own
//! "Migrate the daemon on This Mac" until the daemon holds one (D374); its
//! Migrate runs on This Mac through the local daemon client, not as an
//! answer. Once that returns, the Inbox waits, bounded, for This Mac's
//! daemon to come back at the staged release: then it asks the app to
//! reconnect and the item ends Migrated; past the bound it fails (D376).
use std::sync::Arc;
use std::time::Duration;

use humata2_clients::model::{
    Answer, ChannelId, MachineId, MigrateCard, MigrateFailure, MigratePhase, MigrateThing,
    Preview, Release, Request, RequestId, RequestKind, SessionId,
};
use humata2_clients::{Clients, DaemonError, Fakes, Workspace, fixture};
use humata2_inbox::{
    Bulk, Delegate, Group, History, Inbox, InboxAction as A, InboxState, MigrateStatus, Offer,
    OldLocalDaemon,
};
use tca::{TestClock, TestStore};

const FROM: (&str, &str) = ("2.0.4", "9f3e2d1c0b1a29384756");
const TO: (&str, &str) = ("2.0.5", "a1b2c3d4e5f60718293a");

fn release((version, bundle): (&str, &str)) -> Release {
    Release { version: version.to_owned(), bundle: bundle.to_owned() }
}

fn card(thing: MigrateThing, machine: &str, phase: MigratePhase) -> MigrateCard {
    MigrateCard {
        thing,
        machine: MachineId::new(machine),
        from: release(FROM),
        to: release(TO),
        changes: vec!["Daemon".to_owned(), "Store".to_owned()],
        keeps_running: vec!["3 sessions".to_owned()],
        phase,
        waiting_sessions: Vec::new(),
        migrated: None,
        steps: Vec::new(),
        holders: Vec::new(),
    }
}

/// A Migrate item the daemon holds: no requester, so no session or channel.
fn migrate(id: &str, thing: MigrateThing, machine: &str, phase: MigratePhase) -> Request {
    Request {
        id: RequestId::new(id),
        session: SessionId::new(""),
        channel: ChannelId::new(""),
        kind: RequestKind::Migrate,
        title: format!("Migrate {thing:?} on {machine}"),
        reason: String::new(),
        preview: Preview::Migrate(card(thing, machine, phase)),
        doing: String::new(),
        rule: String::new(),
        // The daemon leaves undo unset for a Migrate item.
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

fn gpu_daemon() -> Request {
    migrate("m-gpu", MigrateThing::Daemon, "gpu-box", MigratePhase::Waiting)
}

/// The design workspace with only `first`, then two ordinary waiting
/// requests that can be undone (so Allow all never asks first on their
/// account). The design fixture's own requests are the frames'; the two
/// ordinary ones come from the standard fixture, which keeps them stable.
fn workspace(first: Vec<Request>) -> Workspace {
    let mut workspace = fixture::design();
    let ordinary: Vec<Request> = fixture::standard()
        .requests
        .iter()
        .filter(|r| r.id.as_str() == "r-test" || r.id.as_str() == "r-edit")
        .cloned()
        .map(|mut r| {
            r.answer = None;
            r.undoable = true;
            r
        })
        .collect();
    assert_eq!(ordinary.len(), 2, "the standard fixture has r-test and r-edit");
    workspace.requests = first.into_iter().chain(ordinary).collect();
    workspace
}

fn store(first: Vec<Request>) -> (TestStore<Inbox>, Fakes) {
    let (store, fakes, _clock) = store_with_clock(first);
    (store, fakes)
}

fn store_with_clock(first: Vec<Request>) -> (TestStore<Inbox>, Fakes, Arc<TestClock>) {
    let workspace = workspace(first);
    let clock = Arc::new(TestClock::new());
    let (clients, fakes) = Clients::fake(workspace.clone(), clock.clone());
    let mut store = TestStore::new(InboxState::default(), Inbox::new(clients)).non_exhaustive();
    store.send(A::Show(Arc::new(workspace)), |_| {});
    (store, fakes, clock)
}

/// Shows what the fake daemon holds now, as the app does after its events.
fn refresh(store: &mut TestStore<Inbox>, fakes: &Fakes) {
    store.send(A::Show(Arc::new(fakes.daemon.workspace())), |_| {});
}

fn answered(store: &mut TestStore<Inbox>, id: &RequestId) {
    let want = id.clone();
    store.receive_matching(
        "the answer's result",
        move |a| matches!(a, A::Answered(got, _) if *got == want),
        |_| {},
    );
}

fn shown_ids(state: &InboxState) -> Vec<RequestId> {
    state.shown().into_iter().map(|r| r.id.clone()).collect()
}

/// The Migrate items shown for `thing` on `machine`.
fn shown_migrates(state: &InboxState, thing: MigrateThing, machine: &str) -> Vec<Request> {
    state
        .shown()
        .into_iter()
        .filter(|r| {
            r.migrate()
                .is_some_and(|card| card.thing == thing && card.machine.as_str() == machine)
        })
        .cloned()
        .collect()
}

fn old_local_daemon() -> OldLocalDaemon {
    OldLocalDaemon {
        machine: MachineId::new("this-mac"),
        running: "2.0.4".to_owned(),
        staged: release(TO),
    }
}

// The card (frame 77).

#[test]
fn a_migrate_item_is_vip_in_the_humata_group_with_migrate_its_only_answer() {
    let (store, _fakes) = store(vec![gpu_daemon()]);
    let state = store.state();
    let id = RequestId::new("m-gpu");
    let Some(item) = state.shown().into_iter().find(|r| r.id == id).cloned() else {
        panic!("the Migrate item shows while it waits");
    };

    assert!(state.vip(&item));
    assert_eq!(state.migrate_status(&id), Some(MigrateStatus::Waiting));
    assert_eq!(state.offers(&id), vec![Offer::Migrate]);

    let groups = state.groups();
    let (first, requests) = &groups[0];
    assert_eq!(*first, Group::Humata, "Humata's group comes first");
    assert_eq!(requests.iter().map(|r| r.id.clone()).collect::<Vec<_>>(), vec![id.clone()]);
    for (group, requests) in &groups[1..] {
        assert!(requests.iter().all(|r| r.id != id), "{group:?} holds the Migrate item too");
    }
    // Agent requests stay in their channel's group, with their usual answers.
    assert!(groups.iter().any(|(group, requests)| {
        *group == Group::Channel(ChannelId::new("api"))
            && requests.iter().any(|r| r.id.as_str() == "r-test")
    }));
    let ordinary = state.offers(&RequestId::new("r-test"));
    assert!(ordinary.contains(&Offer::Allow) && ordinary.contains(&Offer::Deny), "{ordinary:?}");
    assert!(!ordinary.contains(&Offer::Migrate));
    assert_eq!(state.migrate_status(&RequestId::new("r-test")), None);
}

#[test]
fn a_migrate_item_takes_no_deny_always_reply_or_choice() {
    let (mut store, fakes) = store(vec![gpu_daemon()]);
    let id = RequestId::new("m-gpu");
    store.send(A::Answer(id.clone(), Answer::Denied { reply: None }), |_| {});
    store.send(A::Always(id.clone()), |_| {});
    store.send(A::ToggleAlways(id.clone()), |_| {});
    store.send(A::Choose(id.clone(), "Yes".to_owned()), |_| {});
    store.send(A::StartReply(id.clone()), |_| {});
    store.send(A::SendReply, |_| {});
    store.settle();
    store.skip_received();

    assert!(fakes.daemon.answers().is_empty(), "{:?}", fakes.daemon.answers());
    assert!(store.state().pending.is_empty());
    assert!(store.state().always.is_empty());
    assert_eq!(store.state().migrate_status(&id), Some(MigrateStatus::Waiting));
}

// Allow all and Deny all (UP-1).

#[test]
fn allow_all_skips_the_migrate_item_and_its_count_leaves_it_out() {
    let (mut store, fakes) = store(vec![gpu_daemon()]);
    let id = RequestId::new("m-gpu");
    // The badge counts it; Allow all n doesn't (frame 77: "8 waiting", "Allow all 7").
    assert_eq!(humata2_inbox::waiting(&store.state().workspace), 3);
    assert_eq!(store.state().bulk_count(), 2);

    // The Migrate item can't be undone, but Allow all leaves it out, so it
    // doesn't make Allow all ask first.
    store.send(A::Bulk(Bulk::ApproveAll), |_| {});
    assert_eq!(store.state().confirm, None);
    for _ in 0..2 {
        store.receive_matching("an answer", |a| matches!(a, A::Answered(_, _)), |_| {});
    }
    store.settle();
    store.skip_received();

    let mut sent: Vec<_> = fakes.daemon.answers().into_iter().map(|(id, _)| id).collect();
    sent.sort();
    assert_eq!(sent, vec![RequestId::new("r-edit"), RequestId::new("r-test")]);
    assert!(shown_ids(store.state()).contains(&id), "the Migrate item still waits");
    assert_eq!(store.state().migrate_status(&id), Some(MigrateStatus::Waiting));
}

#[test]
fn deny_all_skips_the_migrate_item() {
    let (mut store, fakes) = store(vec![gpu_daemon()]);
    store.send(A::Bulk(Bulk::DenyAll), |_| {});
    for _ in 0..2 {
        store.receive_matching("an answer", |a| matches!(a, A::Answered(_, _)), |_| {});
    }
    store.settle();
    store.skip_received();

    let answers = fakes.daemon.answers();
    assert_eq!(answers.len(), 2, "{answers:?}");
    assert!(answers.iter().all(|(id, _)| id.as_str() != "m-gpu"), "{answers:?}");
    assert!(shown_ids(store.state()).contains(&RequestId::new("m-gpu")));
}

// Migrate → Migrating… → Failed or Migrated (frames 79–81).

#[test]
fn migrate_sends_approve_for_that_item_and_shows_migrating() {
    let (mut store, fakes) = store(vec![gpu_daemon()]);
    let id = RequestId::new("m-gpu");
    store.send(A::Select(id.clone()), |_| {});

    store.send(A::Migrate(id.clone()), |_| {});
    assert_eq!(store.state().migrate_status(&id), Some(MigrateStatus::Migrating));
    assert!(store.state().offers(&id).is_empty(), "Migrating… is not a button");

    answered(&mut store, &id);
    assert_eq!(fakes.daemon.answers(), vec![(id.clone(), Answer::Approved)]);
    // The item is still pending: it stays shown, selected and Migrating
    // until the daemon says how it ended.
    assert_eq!(store.state().migrate_status(&id), Some(MigrateStatus::Migrating));
    assert_eq!(store.state().selected, Some(id.clone()));
    assert!(shown_ids(store.state()).contains(&id));

    // The fake daemon took it as the real one does: pending, migrating.
    refresh(&mut store, &fakes);
    assert_eq!(store.state().migrate_status(&id), Some(MigrateStatus::Migrating));
    assert!(shown_ids(store.state()).contains(&id));
}

#[test]
fn a_second_click_while_migrating_sends_nothing_more() {
    let (mut store, fakes) = store(vec![gpu_daemon()]);
    let id = RequestId::new("m-gpu");
    store.send(A::Migrate(id.clone()), |_| {});
    store.send(A::Migrate(id.clone()), |_| {});
    answered(&mut store, &id);
    refresh(&mut store, &fakes);
    store.send(A::Migrate(id.clone()), |_| {});
    store.settle();
    store.skip_received();
    assert_eq!(fakes.daemon.answers().len(), 1, "{:?}", fakes.daemon.answers());
}

#[test]
fn a_failed_migrate_shows_the_failure_with_migrate_again() {
    let (mut store, fakes) = store(vec![gpu_daemon()]);
    let id = RequestId::new("m-gpu");
    store.send(A::Migrate(id.clone()), |_| {});
    answered(&mut store, &id);
    refresh(&mut store, &fakes);

    let failure = MigrateFailure {
        message: "2.0.5 failed to start twice, so 2.0.4 is running again".to_owned(),
        fix: "Fix what the log names, then Migrate again".to_owned(),
    };
    let failed = migrate(
        "m-gpu",
        MigrateThing::Daemon,
        "gpu-box",
        MigratePhase::Failed(failure.clone()),
    );
    fakes.daemon.put_request(failed);
    refresh(&mut store, &fakes);

    assert_eq!(store.state().migrate_status(&id), Some(MigrateStatus::Failed(failure)));
    assert_eq!(store.state().offers(&id), vec![Offer::MigrateAgain]);
    assert!(shown_ids(store.state()).contains(&id), "a failure stays waiting");

    store.send(A::Migrate(id.clone()), |_| {});
    assert_eq!(store.state().migrate_status(&id), Some(MigrateStatus::Migrating));
    answered(&mut store, &id);
    assert_eq!(
        fakes.daemon.answers(),
        vec![(id.clone(), Answer::Approved), (id, Answer::Approved)]
    );
}

#[test]
fn an_approved_item_shows_migrated_and_moves_to_answered() {
    let (mut store, fakes) = store(vec![gpu_daemon()]);
    let id = RequestId::new("m-gpu");
    store.send(A::Migrate(id.clone()), |_| {});
    answered(&mut store, &id);

    let mut done = gpu_daemon();
    done.answer = Some(Answer::Approved);
    if let Preview::Migrate(card) = &mut done.preview {
        card.migrated = Some("2 min ago".to_owned());
    }
    fakes.daemon.put_request(done);
    refresh(&mut store, &fakes);

    let migrated = MigrateStatus::Migrated { to: release(TO), at: "2 min ago".to_owned() };
    assert_eq!(store.state().migrate_status(&id), Some(migrated));
    assert!(store.state().offers(&id).is_empty());
    assert!(!shown_ids(store.state()).contains(&id), "Waiting no longer lists it");
    assert_eq!(humata2_inbox::waiting(&store.state().workspace), 2);

    store.send(A::SetHistory(History::Answered), |_| {});
    assert!(shown_ids(store.state()).contains(&id), "Answered lists it");
}

#[test]
fn a_withdrawn_item_leaves_the_waiting_list() {
    let (mut store, fakes) = store(vec![gpu_daemon()]);
    let id = RequestId::new("m-gpu");
    store.send(A::Select(id.clone()), |_| {});

    // gpu-box came back at the host's release by itself (RESOLVED).
    fakes.daemon.withdraw_request(&id);
    refresh(&mut store, &fakes);

    assert!(!shown_ids(store.state()).contains(&id));
    assert_ne!(store.state().selected, Some(id.clone()));
    assert!(store.state().selected.is_some(), "the selection moves to what is left");
    assert_eq!(store.state().migrate_status(&id), None);
    store.send(A::SetHistory(History::All), |_| {});
    assert!(!shown_ids(store.state()).contains(&id), "a withdrawn item isn't an answer");
}

// The app's own item for an old local daemon (D374).

#[test]
fn an_old_local_daemon_with_a_staged_release_shows_the_apps_own_migrate_item() {
    let (mut store, _fakes) = store(Vec::new());
    assert!(shown_migrates(store.state(), MigrateThing::Daemon, "this-mac").is_empty());

    store.send(A::OldLocalDaemon(Some(old_local_daemon())), |_| {});

    let items = shown_migrates(store.state(), MigrateThing::Daemon, "this-mac");
    assert_eq!(items.len(), 1, "{items:?}");
    let item = &items[0];
    assert!(store.state().vip(item));
    assert_eq!(store.state().migrate_status(&item.id), Some(MigrateStatus::Waiting));
    assert_eq!(store.state().offers(&item.id), vec![Offer::Migrate]);
    assert_eq!(item.migrate().map(|card| card.to.clone()), Some(release(TO)));
    let groups = store.state().groups();
    assert_eq!(groups[0].0, Group::Humata);
    assert!(groups[0].1.iter().any(|r| r.id == item.id));
    // It is never sent to the daemon, so Allow all leaves it out as well.
    assert_eq!(store.state().bulk_count(), 2);
}

#[test]
fn the_apps_own_item_migrates_on_this_mac_not_through_an_answer() {
    let (mut store, fakes) = store(Vec::new());
    store.send(A::OldLocalDaemon(Some(old_local_daemon())), |_| {});
    let id = shown_migrates(store.state(), MigrateThing::Daemon, "this-mac")[0].id.clone();

    store.send(A::Migrate(id.clone()), |_| {});
    assert_eq!(store.state().migrate_status(&id), Some(MigrateStatus::Migrating));
    answered(&mut store, &id);

    assert_eq!(fakes.local_daemon.migrations(), vec![TO.1.to_owned()]);
    assert!(fakes.daemon.answers().is_empty(), "the old daemon can't take the answer");
    assert_eq!(store.state().migrate_status(&id), Some(MigrateStatus::Migrating));

    // The app reconnected and sees the daemon at the staged release: the
    // item ends Migrated, not withdrawn (D376).
    store.send(A::OldLocalDaemon(None), |_| {});
    assert_migrated(store.state(), &id);
}

#[test]
fn the_apps_own_item_that_fails_shows_failed_with_migrate_again() {
    let (mut store, fakes) = store(Vec::new());
    fakes.local_daemon.fail_next(DaemonError::Refused {
        what: "Migrating the daemon on This Mac".to_owned(),
        reason: "the staged files don't match the staged bundle".to_owned(),
    });
    store.send(A::OldLocalDaemon(Some(old_local_daemon())), |_| {});
    let id = shown_migrates(store.state(), MigrateThing::Daemon, "this-mac")[0].id.clone();

    store.send(A::Migrate(id.clone()), |_| {});
    answered(&mut store, &id);

    assert!(
        matches!(store.state().migrate_status(&id), Some(MigrateStatus::Failed(_))),
        "{:?}",
        store.state().migrate_status(&id)
    );
    assert_eq!(store.state().offers(&id), vec![Offer::MigrateAgain]);
    assert!(shown_ids(store.state()).contains(&id));
}

#[test]
fn a_daemon_held_item_for_the_same_daemon_replaces_the_apps_own() {
    // Another machine's daemon item doesn't stand in for This Mac's.
    let (mut store, fakes) = store(vec![gpu_daemon()]);
    store.send(A::OldLocalDaemon(Some(old_local_daemon())), |_| {});
    assert_eq!(shown_migrates(store.state(), MigrateThing::Daemon, "this-mac").len(), 1);
    assert_eq!(shown_migrates(store.state(), MigrateThing::Daemon, "gpu-box").len(), 1);

    // Nor does This Mac's CLI item.
    fakes
        .daemon
        .put_request(migrate("m-cli", MigrateThing::Cli, "this-mac", MigratePhase::Waiting));
    refresh(&mut store, &fakes);
    assert_eq!(shown_migrates(store.state(), MigrateThing::Daemon, "this-mac").len(), 1);

    // Once the daemon holds the same thing for the same machine, its row wins.
    let held = migrate("m-local", MigrateThing::Daemon, "this-mac", MigratePhase::Waiting);
    fakes.daemon.put_request(held);
    refresh(&mut store, &fakes);
    let items = shown_migrates(store.state(), MigrateThing::Daemon, "this-mac");
    assert_eq!(
        items.iter().map(|r| r.id.clone()).collect::<Vec<_>>(),
        vec![RequestId::new("m-local")]
    );

    // Its Migrate is an answer to the daemon, not the app's local migrate.
    let id = RequestId::new("m-local");
    store.send(A::Migrate(id.clone()), |_| {});
    answered(&mut store, &id);
    assert_eq!(fakes.daemon.answers(), vec![(id, Answer::Approved)]);
    assert!(fakes.local_daemon.migrations().is_empty());
}

// The app's own item after its migrate returns (D376).

/// Shows the app's own item and clicks Migrate; the local migrate returns
/// Ok and the wait for This Mac's daemon has started.
fn migrated_locally(store: &mut TestStore<Inbox>) -> RequestId {
    store.send(A::OldLocalDaemon(Some(old_local_daemon())), |_| {});
    let id = shown_migrates(store.state(), MigrateThing::Daemon, "this-mac")[0].id.clone();
    store.send(A::Migrate(id.clone()), |_| {});
    store.receive_matching(
        "the local migrate returned",
        |a| matches!(a, A::Answered(_, Ok(()))),
        |_| {},
    );
    // Let the wait and its bound start before the clock moves.
    store.settle();
    id
}

fn assert_migrated(state: &InboxState, id: &RequestId) {
    assert!(
        matches!(
            state.migrate_status(id),
            Some(MigrateStatus::Migrated { ref to, .. }) if *to == release(TO)
        ),
        "{:?}",
        state.migrate_status(id)
    );
    assert!(state.offers(id).is_empty(), "{:?}", state.offers(id));
    assert!(
        !shown_ids(state).contains(id),
        "Waiting no longer lists it"
    );
}

fn assert_failed_with_migrate_again(state: &InboxState, id: &RequestId) {
    let Some(MigrateStatus::Failed(failure)) = state.migrate_status(id) else {
        panic!("{:?}", state.migrate_status(id));
    };
    assert!(!failure.message.is_empty(), "a failure says what went wrong");
    assert!(!failure.fix.is_empty(), "a failure says how to fix it");
    assert_eq!(state.offers(id), vec![Offer::MigrateAgain]);
    assert!(shown_ids(state).contains(id), "a failure stays waiting");
}

#[test]
fn the_apps_own_item_waits_migrating_until_this_macs_daemon_is_back() {
    let (mut store, fakes, clock) = store_with_clock(Vec::new());
    let id = migrated_locally(&mut store);
    assert_eq!(fakes.local_daemon.migrations(), vec![TO.1.to_owned()]);

    // The new daemon isn't up yet, and the bound hasn't run out.
    clock.advance(Duration::from_secs(45));
    store.settle();
    assert_eq!(store.state().migrate_status(&id), Some(MigrateStatus::Migrating));
    assert!(shown_ids(store.state()).contains(&id));
    assert!(store.state().offers(&id).is_empty(), "Migrating… is not a button");
}

#[test]
fn the_apps_own_item_ends_migrated_and_asks_for_a_reconnect_once_the_daemon_is_back() {
    let (mut store, fakes, clock) = store_with_clock(Vec::new());
    let id = migrated_locally(&mut store);

    fakes.local_daemon.come_back(TO.1);
    store.receive_matching(
        "This Mac's daemon answers at the staged release",
        |a| matches!(a, A::LocalDaemonBack),
        |_| {},
    );
    store.receive_matching(
        "the app reconnects its clients",
        |a| matches!(a, A::Delegate(Delegate::Reconnect)),
        |_| {},
    );
    assert_migrated(store.state(), &id);
    assert!(fakes.daemon.answers().is_empty(), "the old daemon never took an answer");
    store.send(A::SetHistory(History::Answered), |_| {});
    assert!(shown_ids(store.state()).contains(&id), "Answered lists it");

    // The bound running out later changes nothing, and neither does the
    // app saying the old daemon is gone.
    clock.advance(Duration::from_secs(120));
    store.settle();
    store.skip_received();
    store.send(A::OldLocalDaemon(None), |_| {});
    assert!(
        matches!(store.state().migrate_status(&id), Some(MigrateStatus::Migrated { .. })),
        "{:?}",
        store.state().migrate_status(&id)
    );
}

#[test]
fn the_apps_own_item_fails_when_this_macs_daemon_does_not_come_back_in_time() {
    let (mut store, fakes, clock) = store_with_clock(Vec::new());
    let id = migrated_locally(&mut store);

    clock.advance(Duration::from_secs(45));
    store.settle();
    clock.advance(Duration::from_secs(30));
    store.receive_matching(
        "the wait ran out",
        |a| matches!(a, A::LocalDaemonLate),
        |_| {},
    );
    assert_failed_with_migrate_again(store.state(), &id);

    // Migrate again runs the local migrate again.
    store.send(A::Migrate(id.clone()), |_| {});
    assert_eq!(store.state().migrate_status(&id), Some(MigrateStatus::Migrating));
    answered(&mut store, &id);
    assert_eq!(fakes.local_daemon.migrations().len(), 2, "{:?}", fakes.local_daemon.migrations());
}

#[test]
fn a_daemon_that_comes_back_at_the_old_release_is_not_back() {
    // launchd restarted the old daemon instead of the staged one.
    let (mut store, fakes, clock) = store_with_clock(Vec::new());
    let id = migrated_locally(&mut store);

    fakes.local_daemon.come_back(FROM.1);
    store.settle();
    assert_eq!(store.state().migrate_status(&id), Some(MigrateStatus::Migrating));

    clock.advance(Duration::from_secs(75));
    store.receive_matching(
        "the wait ran out",
        |a| matches!(a, A::LocalDaemonLate),
        |_| {},
    );
    assert_failed_with_migrate_again(store.state(), &id);
}

#[test]
fn a_daemon_already_back_when_the_migrate_returns_counts() {
    let (mut store, fakes, _clock) = store_with_clock(Vec::new());
    fakes.local_daemon.come_back(TO.1);
    let id = migrated_locally(&mut store);
    store.receive_matching(
        "This Mac's daemon answers at the staged release",
        |a| matches!(a, A::LocalDaemonBack),
        |_| {},
    );
    store.receive_matching(
        "the app reconnects its clients",
        |a| matches!(a, A::Delegate(Delegate::Reconnect)),
        |_| {},
    );
    assert_migrated(store.state(), &id);
}

#[test]
fn a_local_migrate_that_fails_does_not_wait_for_the_daemon() {
    let (mut store, fakes, clock) = store_with_clock(Vec::new());
    fakes.local_daemon.fail_next(DaemonError::Refused {
        what: "Migrating the daemon on This Mac".to_owned(),
        reason: "the staged files don't match the staged bundle".to_owned(),
    });
    store.send(A::OldLocalDaemon(Some(old_local_daemon())), |_| {});
    let id = shown_migrates(store.state(), MigrateThing::Daemon, "this-mac")[0].id.clone();
    store.send(A::Migrate(id.clone()), |_| {});
    answered(&mut store, &id);
    let before = store.state().migrate_status(&id);
    assert!(matches!(before, Some(MigrateStatus::Failed(_))), "{before:?}");

    // A daemon coming back or the bound running out changes nothing: there
    // was no migrate to wait for.
    fakes.local_daemon.come_back(TO.1);
    clock.advance(Duration::from_secs(120));
    store.settle();
    store.skip_received();
    assert_eq!(store.state().migrate_status(&id), before);
}

#[test]
fn an_own_item_never_migrated_is_withdrawn_when_the_old_daemon_is_gone() {
    // Someone updated This Mac's daemon another way: there is nothing to
    // report, so the item leaves without reading Migrated.
    let (mut store, _fakes) = store(Vec::new());
    store.send(A::OldLocalDaemon(Some(old_local_daemon())), |_| {});
    let id = shown_migrates(store.state(), MigrateThing::Daemon, "this-mac")[0].id.clone();
    store.send(A::OldLocalDaemon(None), |_| {});
    assert!(shown_migrates(store.state(), MigrateThing::Daemon, "this-mac").is_empty());
    assert_eq!(store.state().migrate_status(&id), None);
    store.send(A::SetHistory(History::All), |_| {});
    assert!(!shown_ids(store.state()).contains(&id), "a withdrawn item isn't an answer");
}
