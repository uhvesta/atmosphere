//! What an add's check step reads from the machine, over a fake `ssh` (no
//! network, never the real ~/.ssh): the versions of `git` and `gh` beside
//! the platform and the free disk, handed to the daemon as the step
//! finishes so the Add machine dialog can show them (HO-1, frame 56).
//!
//! Waiting for enabling: this file lives in tests/pending/ (outside the
//! BUILD glob `tests/*.rs`) until the crate has
//!
//! - `Facts::git_version(&self) -> &str`: the number in `git --version`,
//!   such as `2.47.1`; empty when it printed none;
//! - `Facts::gh_version(&self) -> &str`: the number in `gh --version`,
//!   such as `2.83.0`; empty when there is no `gh` or it printed none;
//! - `StepEvent::Checked(Facts)` (boxed or not): what the check step read,
//!   pushed before that step's `StepEvent::Done(OpStep::Check)`.
//!
//! Then move it to `tests/check_facts.rs`; the glob makes it
//! `//apps/humata2/daemon/machines:check_facts_test`. It uses
//! `FakeMachine::set_tool` (testing/fake_machine.rs), which is there today.

#[path = "../testing/fake_machine.rs"]
mod fake_machine;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use fake_machine::FakeMachine;
use humata2_machines::{
    AddSsh, Arch, Builds, BuildsError, Drive, EntryState, Facts, InstalledFile, Machines, MachinesConfig,
    ManageError, OpStep, Os, Payload, Platform, RetryPolicy, StepEvent,
};

/// The same files for every platform.
struct Files;

impl Builds for Files {
    fn payloads(&self, _platform: Platform) -> Result<Vec<Payload>, BuildsError> {
        Ok(InstalledFile::ALL
            .iter()
            .map(|&file| {
                let body = format!("#!/bin/sh\n# {file}\nexit 0\n");
                Payload::new(file, Arc::from(body.into_bytes()))
            })
            .collect())
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

    fn machines(&self) -> Machines {
        Machines::open(MachinesConfig {
            hosts_file: self.folder.path().join("hosts.json"),
            known_hosts: self.folder.path().join("known_hosts"),
            ssh_config: self.folder.path().join("ssh_config"),
            peer_keys: self.folder.path().join("peer-keys"),
            retry: RetryPolicy {
                attempts: 2,
                first_delay: Duration::ZERO,
                max_delay: Duration::ZERO,
            },
        })
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

/// Adds the machine as the dialog does: a first try that reports the host
/// key, then the add that trusts it. Returns how the second ended and every
/// event it pushed, in order.
fn add(host: &Host, machine: &FakeMachine) -> (Result<EntryState, ManageError>, Vec<StepEvent>) {
    let runner = machine.runner();
    let machines = host.machines();
    let run = |request: &AddSsh| {
        let mut events = Vec::new();
        let mut push = |event| events.push(event);
        let result = machines.plan_add(request).and_then(|plan| {
            let mut drive = Drive {
                runner: &runner,
                builds: &Files,
                wait: &|_| {},
                events: &mut push,
                cancelled: &|| false,
            };
            machines.add(request, plan, &mut drive)
        });
        (result, events)
    };
    let fingerprint = match run(&request(None)).0 {
        Err(ManageError::HostKeyUnconfirmed { fingerprint, .. }) => fingerprint,
        other => panic!("expected the host key to need confirming, got {other:?}"),
    };
    let (result, events) = run(&request(Some(fingerprint)));
    (result.map(|entry| entry.state), events)
}

/// A tool that prints `output` when asked for its version, as `gh` does for
/// `--version` and `version`, and fails for anything else. The fake `git`
/// prints its version whatever it is asked, as the fake machine's own does.
fn says_its_version(output: &str) -> String {
    format!("case \"$1\" in\n  --version|version) cat <<'VERSION'\n{output}\nVERSION\n  ;;\n  *) exit 1 ;;\nesac")
}

/// What the check step read, and that it was pushed before the step was done.
fn checked(events: &[StepEvent]) -> &Facts {
    let at = events
        .iter()
        .position(|event| matches!(event, StepEvent::Checked(_)))
        .unwrap_or_else(|| panic!("the add never said what its check read: {events:?}"));
    let done = events
        .iter()
        .position(|event| *event == StepEvent::Done(OpStep::Check))
        .unwrap_or_else(|| panic!("the check step never finished: {events:?}"));
    assert!(at < done, "the facts come before the check step is done: {events:?}");
    match &events[at] {
        StepEvent::Checked(facts) => facts,
        _ => unreachable!("found by its position"),
    }
}

#[test]
fn the_check_reads_the_versions_of_git_and_gh_beside_the_platform_and_the_free_disk() {
    let host = Host::new();
    let machine = FakeMachine::new();
    machine.set_tool("git", "echo 'git version 2.47.1'");
    machine.set_tool(
        "gh",
        &says_its_version("gh version 2.83.0 (2025-11-04)\nhttps://github.com/cli/cli/releases/tag/v2.83.0"),
    );

    let (result, events) = add(&host, &machine);

    assert_eq!(result.unwrap(), EntryState::Ready);
    let facts = checked(&events);
    assert_eq!(facts.git_version(), "2.47.1");
    assert_eq!(facts.gh_version(), "2.83.0");
    assert_eq!(
        facts.platform,
        Platform {
            os: Os::Linux,
            arch: Arch::X86_64
        }
    );
    assert!(facts.disk_free_kib.is_some_and(|kib| kib > 0), "{:?}", facts.disk_free_kib);
}

#[test]
fn a_git_that_names_its_build_after_the_number_gives_the_number_alone() {
    let host = Host::new();
    let machine = FakeMachine::new();
    machine.set_tool("git", "echo 'git version 2.39.5 (Apple Git-154)'");

    let (result, events) = add(&host, &machine);

    assert_eq!(result.unwrap(), EntryState::Ready);
    assert_eq!(checked(&events).git_version(), "2.39.5");
}

#[test]
fn a_gh_that_cannot_say_its_version_leaves_it_empty_and_the_add_goes_on() {
    let host = Host::new();
    let machine = FakeMachine::new();
    machine.set_tool("git", "echo 'git version 2.47.1'");
    // It is on PATH and fails whatever it is asked, printing nothing.
    machine.set_tool("gh", "exit 127");

    let (result, events) = add(&host, &machine);

    assert_eq!(result.unwrap(), EntryState::Ready, "gh is not needed to add a machine");
    let facts = checked(&events);
    assert_eq!(facts.gh_version(), "");
    assert_eq!(facts.git_version(), "2.47.1");
}

#[test]
fn a_machine_without_gh_has_no_gh_version() {
    // The fake machine's PATH ends in /usr/bin:/bin. Where this computer
    // keeps a gh there, the machine has one too and this says nothing; the
    // test above covers a gh that gives no version.
    if ["/usr/bin/gh", "/bin/gh"].iter().any(|path| Path::new(path).exists()) {
        return;
    }
    let host = Host::new();
    let machine = FakeMachine::new();

    let (result, events) = add(&host, &machine);

    assert_eq!(result.unwrap(), EntryState::Ready, "gh is not needed to add a machine");
    assert_eq!(checked(&events).gh_version(), "");
}
