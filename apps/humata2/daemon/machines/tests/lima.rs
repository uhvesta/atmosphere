//! End to end with the built-in Lima provider: create the VM
//! `humata2-machines-e2e`, set it up with fake humatad2 files, check its
//! status, stop it and delete it. Needs `limactl` on PATH and network for
//! the image. Never touches a VM whose name doesn't start with `humata2-`.

use std::collections::BTreeMap;
use std::sync::Arc;

use humata2_machines::{
    parse_status, Action, Builds, InstalledFile, MachineName, MachineStatus, Payload, Platform,
    Progress, Provider, Runner, Setup, SetupError, SystemRunner,
};

const MACHINE: &str = "machines-e2e";

struct Fakes;

impl Builds for Fakes {
    fn payloads(&self, _platform: Platform) -> Option<Vec<Payload>> {
        Some(vec![
            // Stays running, so the service step sees it start.
            Payload::new(
                InstalledFile::Humatad,
                Arc::from(&b"#!/bin/sh\nexec sleep 2147483647\n"[..]),
            ),
            Payload::new(InstalledFile::Holder, Arc::from(&b"#!/bin/sh\nexit 0\n"[..])),
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
    let runner = SystemRunner::new(&());

    let mut used_setsid = false;
    let ready = loop {
        match setup.step(&runner, &Fakes) {
            Progress::Continue => {}
            Progress::RetryAfter { delay, .. } => std::thread::sleep(delay),
            Progress::Ready(ready) => break ready,
            // Some images don't allow lingering without an admin; setup
            // offers setsid then, and that is a pass too.
            Progress::Failed(failure)
                if matches!(failure.error, SetupError::LingerUnavailable { .. }) && !used_setsid =>
            {
                used_setsid = true;
                setup.use_setsid();
            }
            other => panic!("setup stopped: {other:?}"),
        }
    };

    assert_eq!(ready.link.machine(), &machine);
    assert_eq!(status(&provider, &machine), MachineStatus::Running);

    let stop = provider
        .invocation(Action::Stop, &machine, &BTreeMap::new())
        .unwrap()
        .unwrap();
    let stopped = runner.run(&stop).unwrap();
    assert!(stopped.exit.success(), "{}", stopped.tail(40));
    assert_eq!(status(&provider, &machine), MachineStatus::Stopped);
}
