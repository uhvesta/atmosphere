//! The Hosts page reads the selected machine's health from the machines
//! client for its card (HO-4, frame 28): the machine selected when the
//! page's data first arrives, then each machine selected; what was read is
//! kept per machine, and a read that fails keeps what was there.
//!
//! Waiting for enabling: this file lives in tests/pending/ (outside the
//! BUILD glob `tests/*.rs`) until the Hosts reducer has
//!
//! - `HostsState.health: BTreeMap<MachineId, MachineDetail>`: what each
//!   machine's daemon last reported, kept when a later read fails;
//! - `HostsAction::Health(MachineId, Result<MachineDetail, DaemonError>)`:
//!   the answer to a read, named for the machine that was asked about.
//!
//! Then move it to `tests/health.rs`; the glob makes it
//! `//apps/humata2/features/hosts:health_test`.

use std::sync::Arc;
use std::time::Duration;

use humata2_clients::machines::{Call, FakeMachines, GhLogin, MachineDetail, MachineHealth, Reach};
use humata2_clients::model::{Agent, MachineId};
use humata2_clients::{Clients, DaemonError, Workspace, fixture};
use humata2_hosts::{Hosts, HostsAction as A, HostsState};
use tca::{TestClock, TestStore};

fn id(text: &str) -> MachineId {
    MachineId::new(text)
}

/// What `machine`'s daemon reports, with `gh` at this version.
fn detail(machine: &str, gh_version: &str) -> MachineDetail {
    MachineDetail {
        machine: id(machine),
        os: "Ubuntu 24.04".to_owned(),
        arch: "x86_64".to_owned(),
        address: format!("{machine}.lan"),
        reach: Reach::Ssh,
        daemon_version: "2.0.3".to_owned(),
        health: MachineHealth {
            gh: Some(GhLogin { user: "uhvesta".to_owned(), scopes: vec!["repo".to_owned()] }),
            agents: vec![(Agent::Claude, "2.4.1".to_owned()), (Agent::Codex, "0.98.0".to_owned())],
            disk_free: 11_000_000_000,
            disk_total: 500_000_000_000,
            variables: vec![],
            skills_problem: None,
            git_version: "2.43.0".to_owned(),
            gh_version: gh_version.to_owned(),
        },
        folders: vec![],
        sessions: vec![],
        rules: vec![],
    }
}

/// The Hosts page on the design workspace (this-mac first, then devbox-1),
/// with nothing shown yet.
fn page() -> (TestStore<Hosts>, FakeMachines, Arc<Workspace>) {
    let workspace = Arc::new(fixture::design());
    let (clients, fakes) = Clients::fake((*workspace).clone(), Arc::new(TestClock::new()));
    let store = TestStore::new(HostsState::default(), Hosts::new(clients))
        .with_timeout(Duration::from_secs(30))
        .non_exhaustive();
    (store, fakes.machines, workspace)
}

/// The page with its data, and this-mac's health read as `first`.
fn shown(first: &MachineDetail) -> (TestStore<Hosts>, FakeMachines, Arc<Workspace>) {
    let (mut store, fake, workspace) = page();
    fake.set_detail(Ok(first.clone()));
    store.send(A::Show(workspace.clone()), |_| {});
    store.receive(A::Health(id("this-mac"), Ok(first.clone())), |_| {});
    (store, fake, workspace)
}

/// The machines whose detail was asked for, oldest first.
fn asked(fake: &FakeMachines) -> Vec<MachineId> {
    fake.calls()
        .into_iter()
        .filter_map(|call| match call {
            Call::Detail(machine) => Some(machine),
            _ => None,
        })
        .collect()
}

#[test]
fn the_machine_selected_when_the_data_first_arrives_has_its_health_read() {
    let (mut store, fake, workspace) = page();
    let this_mac = detail("this-mac", "2.83.0");
    fake.set_detail(Ok(this_mac.clone()));

    store.send(A::Show(workspace), |_| {});
    assert_eq!(store.state().selected, Some(id("this-mac")));
    store.receive(A::Health(id("this-mac"), Ok(this_mac.clone())), |_| {});

    assert_eq!(store.state().health.get(&id("this-mac")), Some(&this_mac));
    assert_eq!(asked(&fake), [id("this-mac")]);
}

#[test]
fn selecting_a_machine_reads_that_machines_health_and_keeps_the_others() {
    let this_mac = detail("this-mac", "2.83.0");
    let (mut store, fake, _workspace) = shown(&this_mac);
    let devbox = detail("devbox-1", "2.90.1");
    fake.set_detail(Ok(devbox.clone()));

    store.send(A::Select(id("devbox-1")), |_| {});
    store.receive(A::Health(id("devbox-1"), Ok(devbox.clone())), |_| {});

    assert_eq!(store.state().health.get(&id("devbox-1")), Some(&devbox));
    assert_eq!(store.state().health.get(&id("this-mac")), Some(&this_mac));
    assert_eq!(asked(&fake), [id("this-mac"), id("devbox-1")]);
}

#[test]
fn a_read_that_fails_keeps_the_health_it_had() {
    let this_mac = detail("this-mac", "2.83.0");
    let (mut store, fake, _workspace) = shown(&this_mac);
    let devbox = detail("devbox-1", "2.90.1");
    fake.set_detail(Ok(devbox.clone()));
    store.send(A::Select(id("devbox-1")), |_| {});
    store.receive(A::Health(id("devbox-1"), Ok(devbox.clone())), |_| {});

    // The daemon stops answering: both machines are selected again.
    let offline = DaemonError::Offline { machine: id("devbox-1") };
    fake.fail("detail", offline.clone());
    store.send(A::Select(id("this-mac")), |_| {});
    store.receive(A::Health(id("this-mac"), Err(offline.clone())), |_| {});
    store.send(A::Select(id("devbox-1")), |_| {});
    store.receive(A::Health(id("devbox-1"), Err(offline)), |_| {});

    assert_eq!(store.state().health.get(&id("this-mac")), Some(&this_mac));
    assert_eq!(store.state().health.get(&id("devbox-1")), Some(&devbox));
    // A health read that failed is not a connect that failed.
    assert!(store.state().errors.is_empty(), "{:?}", store.state().errors);
}

#[test]
fn a_machine_never_read_has_no_health_after_a_read_that_fails() {
    let this_mac = detail("this-mac", "2.83.0");
    let (mut store, fake, _workspace) = shown(&this_mac);
    let offline = DaemonError::Offline { machine: id("gpu-box") };
    fake.fail("detail", offline.clone());

    store.send(A::Select(id("gpu-box")), |_| {});
    store.receive(A::Health(id("gpu-box"), Err(offline)), |_| {});

    assert_eq!(store.state().selected, Some(id("gpu-box")));
    assert_eq!(store.state().health.get(&id("gpu-box")), None);
    assert_eq!(store.state().health.get(&id("this-mac")), Some(&this_mac));
}

#[test]
fn the_same_data_again_and_a_machine_that_isnt_listed_read_nothing_more() {
    let this_mac = detail("this-mac", "2.83.0");
    let (mut store, fake, workspace) = shown(&this_mac);

    // The workspace is shown again on every change of its data.
    store.send(A::Show(workspace), |_| {});
    store.send(A::Select(id("missing")), |_| {});
    store.settle();

    assert_eq!(asked(&fake), [id("this-mac")]);
    assert_eq!(store.state().health.get(&id("this-mac")), Some(&this_mac));
}
