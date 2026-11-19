//! Changing a machine's Humata folder from Settings (SPEC PL-1, FC-10, ST-5):
//! Change… opens a sheet, Plan shows the channels under the old root and the
//! sessions that run, and nothing moves until the person picks Move them or
//! Use for new channels only. Against the fakes.

use std::sync::Arc;

use humata2_clients::machines::{Call, FakeMachines};
use humata2_clients::model::{ChannelId, MachineId, RootPlan, SessionId};
use humata2_clients::{Clients, DaemonError, fixture};
use humata2_hosts::{Hosts, HostsAction as A, HostsState};
use tca::{TestClock, TestStore};

fn mac() -> MachineId {
    MachineId::new("this-mac")
}

fn plan(running: &[&str]) -> RootPlan {
    RootPlan {
        machine: mac(),
        token: "pl_1".to_owned(),
        old_root: "~/Humata".to_owned(),
        new_root: "~/Work".to_owned(),
        channels: vec![ChannelId::new("web"), ChannelId::new("api")],
        running: running.iter().map(|id| SessionId::new(*id)).collect(),
    }
}

/// The hosts page on the standard fakes, whose root plan is `plan`.
fn page(plan: RootPlan) -> (TestStore<Hosts>, FakeMachines) {
    let workspace = Arc::new(fixture::standard());
    let (clients, fakes) = humata2_clients::fake_clients((*workspace).clone(), Arc::new(TestClock::new()));
    fakes.machines.set_root_plan(Ok(plan));
    let state = HostsState { workspace, ..HostsState::default() };
    let store = TestStore::new(state, Hosts::new(clients)).non_exhaustive();
    (store, fakes.machines)
}

/// The sheet open on This Mac with `~/Work` typed and planned.
fn planned(plan: RootPlan) -> (TestStore<Hosts>, FakeMachines) {
    let (mut store, fake) = page(plan);
    store.send(A::OpenRoot(mac()), |_| {});
    store.send(A::RootText("~/Work".to_owned()), |_| {});
    store.send(A::PlanRoot, |_| {});
    store.receive_matching("planned", |a| matches!(a, A::RootPlanned(_, _, Ok(_))), |_| {});
    (store, fake)
}

fn changes(fake: &FakeMachines) -> Vec<(MachineId, String, bool)> {
    fake.calls()
        .into_iter()
        .filter_map(|call| match call {
            Call::ChangeRoot(machine, root, moved) => Some((machine, root, moved)),
            _ => None,
        })
        .collect()
}

fn root_shown(store: &TestStore<Hosts>) -> String {
    store.state().workspace.machine(&mac()).map(|m| m.root.clone()).unwrap_or_default()
}

#[test]
fn the_sheet_opens_with_the_current_root_and_a_plan_lists_the_channels() {
    let (mut store, fake) = page(plan(&[]));
    store.send(A::OpenRoot(mac()), |_| {});
    let sheet = store.state().root_change.clone().expect("Change… opens the sheet");
    assert_eq!(sheet.text, root_shown(&store));
    assert!(sheet.plan.is_none());

    store.send(A::RootText("~/Work".to_owned()), |_| {});
    store.send(A::PlanRoot, |_| {});
    store.receive_matching("planned", |a| matches!(a, A::RootPlanned(_, _, Ok(_))), |_| {});

    let sheet = store.state().root_change.clone().expect("the sheet stays open");
    let planned = sheet.plan.expect("the plan is shown");
    assert_eq!(planned.channels.len(), 2);
    assert!(changes(&fake).is_empty(), "planning changes nothing (FC-10)");
    assert_eq!(root_shown(&store), "~/Humata");
}

#[test]
fn typing_again_drops_a_plan_made_for_other_text() {
    let (mut store, _fake) = planned(plan(&[]));
    store.send(A::RootText("~/Else".to_owned()), |_| {});
    let sheet = store.state().root_change.clone().expect("open");
    assert!(sheet.plan.is_none(), "a plan is for the folder it was made for");
}

#[test]
fn move_them_changes_the_root_with_move_channels_on() {
    let (mut store, fake) = planned(plan(&[]));
    store.send(A::ApplyRoot { move_channels: true }, |_| {});
    store.receive_matching("changed", |a| matches!(a, A::RootApplied(_, _, Ok(()))), |_| {});

    assert_eq!(changes(&fake), vec![(mac(), "~/Work".to_owned(), true)]);
    assert_eq!(root_shown(&store), "~/Work");
    assert!(store.state().root_change.is_none(), "the sheet closes when it is done");
}

#[test]
fn use_for_new_channels_only_changes_the_root_with_move_channels_off() {
    let (mut store, fake) = planned(plan(&[]));
    store.send(A::ApplyRoot { move_channels: false }, |_| {});
    store.receive_matching("changed", |a| matches!(a, A::RootApplied(_, _, Ok(()))), |_| {});

    assert_eq!(changes(&fake), vec![(mac(), "~/Work".to_owned(), false)]);
    assert_eq!(root_shown(&store), "~/Work");
}

#[test]
fn running_sessions_hold_back_moving_but_not_new_channels_only() {
    let (mut store, fake) = planned(plan(&["ledger"]));
    store.send(A::ApplyRoot { move_channels: true }, |_| {});
    store.settle();
    assert!(changes(&fake).is_empty(), "nothing moves while a session runs there");
    assert!(store.state().root_change.is_some());
    assert_eq!(root_shown(&store), "~/Humata");

    store.send(A::ApplyRoot { move_channels: false }, |_| {});
    store.receive_matching("changed", |a| matches!(a, A::RootApplied(_, _, Ok(()))), |_| {});
    assert_eq!(changes(&fake), vec![(mac(), "~/Work".to_owned(), false)]);
}

#[test]
fn nothing_is_sent_until_a_choice_is_made() {
    let (mut store, fake) = planned(plan(&[]));
    store.settle();
    assert!(changes(&fake).is_empty());
    store.send(A::DismissRoot, |_| {});
    assert!(store.state().root_change.is_none());
    assert!(changes(&fake).is_empty());
    assert_eq!(root_shown(&store), "~/Humata");
}

#[test]
fn a_refused_change_keeps_the_sheet_open_with_the_daemons_message() {
    let (mut store, fake) = planned(plan(&[]));
    fake.fail(
        "change_root",
        DaemonError::Refused {
            what: "Changing the Humata folder".to_owned(),
            reason: "the folder can't be written; pick another".to_owned(),
        },
    );
    store.send(A::ApplyRoot { move_channels: false }, |_| {});
    store.receive_matching("failed", |a| matches!(a, A::RootApplied(_, _, Err(_))), |_| {});

    let sheet = store.state().root_change.clone().expect("the sheet stays open");
    assert!(sheet.error.is_some());
    assert!(!sheet.running);
    assert_eq!(root_shown(&store), "~/Humata", "a failed change leaves the root alone");
}

#[test]
fn a_failed_plan_shows_its_message_and_offers_no_choice() {
    let (mut store, fake) = page(plan(&[]));
    fake.fail("plan_change_root", DaemonError::NotFound { what: "this machine".to_owned() });
    store.send(A::OpenRoot(mac()), |_| {});
    store.send(A::PlanRoot, |_| {});
    store.receive_matching("planned", |a| matches!(a, A::RootPlanned(_, _, Err(_))), |_| {});
    let sheet = store.state().root_change.clone().expect("open");
    assert!(sheet.error.is_some());
    assert!(sheet.plan.is_none());
}
