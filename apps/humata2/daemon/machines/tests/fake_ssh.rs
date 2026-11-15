//! Adding, reconnecting and removing a machine end to end over a fake
//! `ssh` (no network, never the real ~/.ssh): what lands on the machine and
//! in hosts.json, and that a kill of the host or the machine at any step is
//! finished or undone (PER-27).

#[path = "../testing/fake_machine.rs"]
mod fake_machine;

use std::panic::{self, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use fake_machine::{FakeMachine, FakeSshRunner, Fault};
use humata2_machines::{
    AddSsh, Builds, BuildsError, Drive, EntryState, InstalledFile, Machines, MachinesConfig, ManageError,
    Payload, Platform, Purpose, RetryPolicy, SetupError, StepEvent,
};

/// The same three files for every platform; `version` changes their bytes.
struct Files(u32);

impl Files {
    fn payloads(&self) -> Vec<Payload> {
        InstalledFile::ALL
            .iter()
            .map(|&file| {
                let body = format!("#!/bin/sh\n# {file} build {}\nexit 0\n", self.0);
                Payload::new(file, Arc::from(body.into_bytes()))
            })
            .collect()
    }
}

impl Builds for Files {
    fn payloads(&self, _platform: Platform) -> Result<Vec<Payload>, BuildsError> {
        Ok(Files::payloads(self))
    }
}

/// The host side: its own data folder.
struct Host {
    folder: tempfile::TempDir,
}

impl Host {
    fn new() -> Self {
        Self {
            folder: tempfile::tempdir().unwrap(),
        }
    }

    /// The machines as a fresh start of the daemon sees them.
    fn machines(&self) -> Machines {
        Machines::open(MachinesConfig {
            hosts_file: self.folder.path().join("hosts.json"),
            known_hosts: self.known_hosts(),
            ssh_config: self.folder.path().join("ssh_config"),
            retry: RetryPolicy {
                attempts: 2,
                first_delay: Duration::ZERO,
                max_delay: Duration::ZERO,
            },
        })
    }

    fn known_hosts(&self) -> PathBuf {
        self.folder.path().join("known_hosts")
    }

    fn pinned(&self) -> bool {
        std::fs::read_to_string(self.known_hosts())
            .map(|text| text.contains("humata2.box-1 "))
            .unwrap_or(false)
    }
}

fn request(fingerprint: Option<String>) -> AddSsh {
    AddSsh {
        ssh_host: "me@box-1.internal".to_owned(),
        name: String::new(),
        root: String::new(),
        trust_fingerprint: fingerprint,
    }
}

/// Runs `f` with a drive over `runner` and `builds`, collecting events.
fn drive<R>(
    runner: &FakeSshRunner,
    builds: &dyn Builds,
    f: impl FnOnce(&mut Drive<'_>) -> R,
) -> (R, Vec<StepEvent>) {
    let mut events = Vec::new();
    let mut push = |event| events.push(event);
    let mut drive = Drive {
        runner,
        builds,
        wait: &|_| {},
        events: &mut push,
        cancelled: &|| false,
    };
    let result = f(&mut drive);
    (result, events)
}

/// The fingerprint the first try reports, as the dialog would show it.
fn fingerprint(host: &Host, machine: &FakeMachine) -> String {
    let machines = host.machines();
    let plan = machines.plan_add(&request(None)).unwrap();
    let (result, _) = drive(&machine.runner(), &Files(1), |drive| {
        machines.add(&request(None), plan, drive)
    });
    match result {
        Err(ManageError::HostKeyUnconfirmed { fingerprint, .. }) => fingerprint,
        other => panic!("expected the host key to need confirming, got {other:?}"),
    }
}

fn add(host: &Host, machine: &FakeMachine, runner: &FakeSshRunner, builds: &dyn Builds) -> Result<(), ManageError> {
    let fingerprint = fingerprint(host, machine);
    let machines = host.machines();
    let request = request(Some(fingerprint));
    let plan = machines.plan_add(&request)?;
    drive(runner, builds, |drive| machines.add(&request, plan, drive))
        .0
        .map(|_| ())
}

fn assert_has(machine: &FakeMachine, files: &Files) {
    for payload in files.payloads() {
        assert_eq!(
            machine.installed(payload.file().file_name()).as_deref(),
            Some(&payload.bytes()[..]),
            "{} isn't the expected build",
            payload.file()
        );
    }
}

/// Nothing of Humata's is left on the machine or the host.
fn assert_clean(host: &Host, machine: &FakeMachine) {
    assert_eq!(host.machines().list().unwrap(), []);
    assert!(!host.pinned(), "the host key is still pinned");
    for file in InstalledFile::ALL {
        assert_eq!(machine.installed(file.file_name()), None, "{file} is still on the machine");
    }
    assert!(!machine.unit_installed(), "the service is still installed");
}

#[test]
fn adding_a_machine_needs_its_host_key_confirmed_and_installs_all_three_tools() {
    let host = Host::new();
    let machine = FakeMachine::new();
    let fingerprint = fingerprint(&host, &machine);
    // The first try stops at the host key and leaves nothing behind.
    assert!(fingerprint.starts_with("SHA256:"));
    assert_clean(&host, &machine);

    let runner = machine.runner();
    let machines = host.machines();
    let request = request(Some(fingerprint));
    let plan = machines.plan_add(&request).unwrap();
    let (entry, events) = drive(&runner, &Files(1), |drive| machines.add(&request, plan, drive));
    let entry = entry.unwrap();

    assert_eq!(entry.name, "box-1");
    assert_eq!(entry.state, EntryState::Ready);
    assert_eq!((entry.os.as_str(), entry.arch.as_str()), ("linux", "x86_64"));
    assert_eq!(host.machines().list().unwrap(), [entry]);
    assert_has(&machine, &Files(1));
    assert!(machine.unit_installed());
    assert!(events.contains(&StepEvent::Done(humata2_machines::OpStep::Install)));
    assert!(events.contains(&StepEvent::Skipped(humata2_machines::OpStep::FindAgents)));
}

#[test]
fn an_add_cancelled_after_any_step_stops_there_and_is_undone() {
    use humata2_machines::OpStep;
    for (after, next) in [
        (OpStep::Connect, OpStep::Check),
        (OpStep::Check, OpStep::Install),
        (OpStep::Install, OpStep::Start),
    ] {
        let host = Host::new();
        let machine = FakeMachine::new();
        let request = request(Some(fingerprint(&host, &machine)));
        let machines = host.machines();
        let plan = machines.plan_add(&request).unwrap();
        let events = std::cell::RefCell::new(Vec::new());
        let mut push = |event| events.borrow_mut().push(event);
        let cancelled = || events.borrow().contains(&StepEvent::Done(after));
        let runner = machine.runner();
        let mut drive = Drive {
            runner: &runner,
            builds: &Files(1),
            wait: &|_| {},
            events: &mut push,
            cancelled: &cancelled,
        };
        let result = machines.add(&request, plan, &mut drive);

        assert!(matches!(result, Err(ManageError::Cancelled { .. })), "after {after:?}: {result:?}");
        assert!(!events.borrow().contains(&StepEvent::Running(next)), "{next:?} ran after the cancel");
        assert_clean(&host, &machine);
    }
}

#[test]
fn a_fingerprint_that_doesnt_match_sends_nothing() {
    let host = Host::new();
    let machine = FakeMachine::new();
    let machines = host.machines();
    let request = request(Some("SHA256:somethingelse".to_owned()));
    let plan = machines.plan_add(&request).unwrap();
    let (result, _) = drive(&machine.runner(), &Files(1), |drive| machines.add(&request, plan, drive));
    assert!(matches!(result, Err(ManageError::HostKeyMismatch { .. })));
    assert_clean(&host, &machine);
}

#[test]
fn a_name_already_added_is_refused_before_anything_runs() {
    let host = Host::new();
    let machine = FakeMachine::new();
    add(&host, &machine, &machine.runner(), &Files(1)).unwrap();
    assert!(matches!(
        host.machines().plan_add(&request(None)),
        Err(ManageError::NameTaken { .. })
    ));
}

#[test]
fn reconnecting_with_the_same_build_installs_nothing() {
    let host = Host::new();
    let machine = FakeMachine::new();
    add(&host, &machine, &machine.runner(), &Files(1)).unwrap();

    let runner = machine.runner();
    let machines = host.machines();
    let (entry, _) = drive(&runner, &Files(1), |drive| machines.reconnect("box-1", drive));

    assert_eq!(entry.unwrap().state, EntryState::Ready);
    assert_eq!(runner.ran(Purpose::Install), 0);
    assert_eq!(runner.ran(Purpose::Promote), 0);
    // The daemon keeps running: nothing changed, so nothing restarts.
    assert_eq!(runner.ran(Purpose::Service), 0);
    assert_has(&machine, &Files(1));
}

#[test]
fn reconnecting_starts_the_service_again_only_when_it_stopped_or_a_file_changed() {
    let host = Host::new();
    let machine = FakeMachine::new();
    add(&host, &machine, &machine.runner(), &Files(1)).unwrap();
    let machines = host.machines();

    machine.clear("active");
    let runner = machine.runner();
    let (entry, _) = drive(&runner, &Files(1), |drive| machines.reconnect("box-1", drive));
    assert_eq!(entry.unwrap().state, EntryState::Ready);
    assert_eq!(runner.ran(Purpose::Service), 1);

    let runner = machine.runner();
    let (entry, _) = drive(&runner, &Files(2), |drive| machines.reconnect("box-1", drive));
    assert_eq!(entry.unwrap().state, EntryState::Ready);
    assert_eq!(runner.ran(Purpose::Service), 1);
    assert_has(&machine, &Files(2));
}

#[test]
fn a_new_build_waits_for_running_holders_then_goes_in() {
    let host = Host::new();
    let machine = FakeMachine::new();
    add(&host, &machine, &machine.runner(), &Files(1)).unwrap();
    let old = Files(1).payloads();
    let new = Files(2).payloads();

    machine.set("holder");
    let machines = host.machines();
    let (entry, _) = drive(&machine.runner(), &Files(2), |drive| machines.reconnect("box-1", drive));
    let entry = entry.unwrap();

    // humatad2 never waits; the holder and humata do, beside the old ones.
    assert_eq!(machine.installed("humatad2").as_deref(), Some(&new[0].bytes()[..]));
    assert_eq!(machine.installed("humata2_holder").as_deref(), Some(&old[1].bytes()[..]));
    assert_eq!(machine.installed("humata").as_deref(), Some(&old[2].bytes()[..]));
    assert_eq!(machine.installed(".humata2_holder.staged").as_deref(), Some(&new[1].bytes()[..]));
    assert_eq!(entry.staged, ["humata2_holder", "humata"]);

    machine.clear("holder");
    let runner = machine.runner();
    let (entry, _) = drive(&runner, &Files(2), |drive| machines.reconnect("box-1", drive));

    assert_eq!(entry.unwrap().staged, Vec::<String>::new());
    assert_has(&machine, &Files(2));
    assert_eq!(machine.installed(".humata2_holder.staged"), None);
    // Moved in from the staged copies, not sent again.
    assert_eq!(runner.ran(Purpose::Install), 0);
}

#[test]
fn an_unsupported_platform_fails_with_its_uname_and_leaves_nothing() {
    let host = Host::new();
    let machine = FakeMachine::new();
    machine.set_uname("FreeBSD amd64");

    let result = add(&host, &machine, &machine.runner(), &Files(1));

    match result {
        Err(ManageError::Step {
            source: SetupError::UnsupportedPlatform { uname, .. },
            ..
        }) => assert!(uname.contains("FreeBSD")),
        other => panic!("expected an unsupported platform, got {other:?}"),
    }
    assert_clean(&host, &machine);
}

#[test]
fn removing_a_machine_takes_humata_off_it_and_forgets_it() {
    let host = Host::new();
    let machine = FakeMachine::new();
    add(&host, &machine, &machine.runner(), &Files(1)).unwrap();

    let machines = host.machines();
    let (result, _) = drive(&machine.runner(), &Files(1), |drive| machines.remove("box-1", drive));

    result.unwrap();
    assert_clean(&host, &machine);
}

/// How many commands a whole add runs, scan included.
fn add_calls() -> usize {
    let host = Host::new();
    let machine = FakeMachine::new();
    let runner = machine.runner();
    add(&host, &machine, &runner, &Files(1)).unwrap();
    runner.calls()
}

fn recover(host: &Host, machine: &FakeMachine) {
    let machines = host.machines();
    let (recovered, _) = drive(&machine.runner(), &Files(1), |drive| machines.recover(drive));
    for recovered in recovered.unwrap() {
        recovered.outcome.unwrap();
    }
}

#[test]
fn an_add_cut_short_by_a_host_restart_at_any_step_is_undone() {
    let steps = add_calls();
    assert!(steps >= 4, "an add runs scan, connect, check, install and start");
    for at in 0..steps {
        let host = Host::new();
        let machine = FakeMachine::new();
        let runner = machine.runner().fault_at(at, Fault::HostKilled);
        let killed = panic::catch_unwind(AssertUnwindSafe(|| add(&host, &machine, &runner, &Files(1))));
        assert!(killed.is_err(), "call {at} wasn't reached");

        recover(&host, &machine);

        assert_clean(&host, &machine);
        // And adding again works.
        add(&host, &machine, &machine.runner(), &Files(1)).unwrap();
    }
}

#[test]
fn an_add_whose_machine_dies_at_any_step_is_undone_once_it_is_back() {
    for at in 0..add_calls() {
        let host = Host::new();
        let machine = FakeMachine::new();
        let runner = machine.runner().fault_at(at, Fault::MachineKilled);
        assert!(add(&host, &machine, &runner, &Files(1)).is_err(), "call {at}");

        machine.clear("down");
        recover(&host, &machine);

        assert_clean(&host, &machine);
    }
}

#[test]
fn a_removal_cut_short_finishes_at_the_next_start() {
    for fault in [Fault::HostKilled, Fault::MachineKilled] {
        let host = Host::new();
        let machine = FakeMachine::new();
        add(&host, &machine, &machine.runner(), &Files(1)).unwrap();

        let runner = machine.runner().fault_at(0, fault);
        let machines = host.machines();
        let cut = panic::catch_unwind(AssertUnwindSafe(|| {
            drive(&runner, &Files(1), |drive| machines.remove("box-1", drive)).0
        }));
        match fault {
            Fault::HostKilled => assert!(cut.is_err()),
            Fault::MachineKilled => {
                assert!(matches!(cut, Ok(Err(ManageError::UninstallFailed { .. }))));
                assert_eq!(host.machines().get("box-1").unwrap().state, EntryState::Removing);
            }
        }

        machine.clear("down");
        recover(&host, &machine);

        assert_clean(&host, &machine);
    }
}
