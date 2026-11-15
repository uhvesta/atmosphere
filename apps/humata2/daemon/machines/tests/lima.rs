//! End to end with the built-in Lima provider: create the VM
//! `humata2-machines-e2e`, set it up with fake humatad2 files, remove
//! humatad2 and set it up again, check its status, stop it and delete it. Needs `limactl` on PATH and network for
//! the image. Never touches a VM whose name doesn't start with `humata2-`.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use humata2_machines::{
    parse_status, Action, Builds, BuildsError, InstalledFile, Invocation, MachineName, MachineStatus, Output,
    Payload, Platform, Progress, Provider, Purpose, Ready, RunError, Runner, Setup, SetupError,
    SystemRunner,
};

const MACHINE: &str = "machines-e2e";

struct Fakes;

impl Builds for Fakes {
    fn payloads(&self, _platform: Platform) -> Result<Vec<Payload>, BuildsError> {
        Ok(vec![
            // Stays running as `humatad2 serve --role machine`, so the
            // service step sees it start and removing it can find it.
            Payload::new(
                InstalledFile::Humatad,
                Arc::from(&b"#!/bin/sh\nwhile :; do sleep 1; done\n"[..]),
            ),
            Payload::new(InstalledFile::Holder, Arc::from(&b"#!/bin/sh\nexit 0\n"[..])),
            Payload::new(InstalledFile::Cli, Arc::from(&b"#!/bin/sh\nexit 0\n"[..])),
        ])
    }
}

/// Deletes the VM however the test ends.
struct Cleanup<'a> {
    provider: &'a Provider,
    machine: &'a MachineName,
}

impl Drop for Cleanup<'_> {
    fn drop(&mut self) {
        if let Ok(Some(delete)) = self
            .provider
            .invocation(Action::Delete, self.machine, &BTreeMap::new())
        {
            let _ = SystemRunner::new(&()).run(&delete);
        }
    }
}

/// The system runner, counting the files it installs.
struct Counting {
    inner: SystemRunner<'static>,
    installs: AtomicUsize,
}

impl Runner for Counting {
    fn run(&self, invocation: &Invocation) -> Result<Output, RunError> {
        if invocation.purpose == Purpose::Install {
            self.installs.fetch_add(1, Ordering::SeqCst);
        }
        self.inner.run(invocation)
    }
}

/// Steps setup until the machine is ready. Some images don't allow
/// lingering, or don't start the user's manager, without an admin; setup
/// offers setsid then, and that is a pass too.
fn drive(setup: &mut Setup, runner: &dyn Runner) -> Ready {
    let mut used_setsid = false;
    loop {
        match setup.step(runner, &Fakes) {
            Progress::Continue => {}
            Progress::RetryAfter { delay, .. } => std::thread::sleep(delay),
            Progress::Ready(ready) => return ready,
            Progress::Failed(failure)
                if matches!(
                    failure.error,
                    SetupError::LingerUnavailable { .. } | SetupError::UserManagerUnavailable { .. }
                ) && !used_setsid =>
            {
                used_setsid = true;
                setup.use_setsid();
            }
            other => panic!("setup stopped: {other:?}"),
        }
    }
}

fn status(provider: &Provider, machine: &MachineName) -> MachineStatus {
    let invocation = provider
        .invocation(Action::Status, machine, &BTreeMap::new())
        .unwrap()
        .unwrap();
    let output = SystemRunner::new(&()).run(&invocation).unwrap();
    parse_status(&output.stdout)
}

#[test]
#[ignore = "e2e: creates, sets up and deletes the Lima VM humata2-machines-e2e on this Mac (minutes, downloads an image); run with --ignored in the e2e batch"]
fn a_lima_vm_is_created_set_up_stopped_and_deleted() {
    let provider = Provider::lima().unwrap();
    let machine = MachineName::new(MACHINE).unwrap();
    let known_hosts = tempfile::tempdir().unwrap();
    let _cleanup = Cleanup {
        provider: &provider,
        machine: &machine,
    };
    let values = BTreeMap::from([
        ("cpus".to_owned(), "2".to_owned()),
        ("memory".to_owned(), "2".to_owned()),
        ("disk".to_owned(), "10".to_owned()),
    ]);
    let mut setup = Setup::new(
        provider.clone(),
        machine.clone(),
        values,
        &known_hosts.path().join("known_hosts"),
    )
    .unwrap();
    let runner = Counting {
        inner: SystemRunner::new(&()),
        installs: AtomicUsize::new(0),
    };

    let ready = drive(&mut setup, &runner);

    assert_eq!(ready.link.machine(), &machine);
    assert_eq!(status(&provider, &machine), MachineStatus::Running);

    // Removing humatad2 takes its files, so setting up again installs both.
    let removed = runner.run(&ready.link.remove_daemon().unwrap()).unwrap();
    assert!(removed.exit.success(), "{}", removed.tail(40));
    let again = runner.run(&ready.link.remove_daemon().unwrap()).unwrap();
    assert!(again.exit.success(), "{}", again.tail(40));
    runner.installs.store(0, Ordering::SeqCst);
    drive(&mut setup, &runner);
    assert_eq!(runner.installs.load(Ordering::SeqCst), 2);

    let stop = provider
        .invocation(Action::Stop, &machine, &BTreeMap::new())
        .unwrap()
        .unwrap();
    let stopped = runner.run(&stop).unwrap();
    assert!(stopped.exit.success(), "{}", stopped.tail(40));
    assert_eq!(status(&provider, &machine), MachineStatus::Stopped);
}
