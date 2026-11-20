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
    Payload, Platform, Purpose, RetryPolicy, SetupError, StackConfig, StackConfigError, StepEvent,
};

/// The same files for every platform; `version` changes their bytes.
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

/// [`Files`] that also name their release: what a tools bundle gives.
struct Released(u32);

impl Released {
    const BUNDLE: &'static [u8] = b"{\"platforms\":{}}";
}

impl Builds for Released {
    fn payloads(&self, _platform: Platform) -> Result<Vec<Payload>, BuildsError> {
        Ok(Files(self.0).payloads())
    }

    fn bundle_json(&self, _platform: Platform) -> Result<Option<Arc<[u8]>>, BuildsError> {
        Ok(Some(Arc::from(Self::BUNDLE)))
    }
}

/// [`Files`] of build 1 that also give the host's stack settings.
struct WithSettings(StackConfig);

impl Builds for WithSettings {
    fn payloads(&self, _platform: Platform) -> Result<Vec<Payload>, BuildsError> {
        Ok(Files(1).payloads())
    }

    fn stack_config(&self) -> Result<Option<StackConfig>, StackConfigError> {
        Ok(Some(self.0.clone()))
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
            peer_keys: self.peer_keys(),
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

    fn peer_keys(&self) -> PathBuf {
        self.folder.path().join("peer-keys")
    }

    /// The host's copy of box-1's enrollment key, if it keeps one.
    fn peer_key(&self) -> Option<Vec<u8>> {
        std::fs::read(self.peer_keys().join("box-1")).ok()
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
    assert_eq!(host.peer_key(), None, "the host still keeps an enrollment key");
    for file in InstalledFile::ALL {
        assert_eq!(machine.installed(file.file_name()), None, "{file} is still on the machine");
    }
    assert!(!machine.unit_installed(), "the service is still installed");
}

#[test]
fn adding_a_machine_needs_its_host_key_confirmed_and_installs_all_the_tools() {
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

fn mode(path: &std::path::Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[test]
fn an_add_writes_the_same_private_enrollment_key_on_the_machine_and_the_host() {
    let host = Host::new();
    let machine = FakeMachine::new();
    add(&host, &machine, &machine.runner(), &Files(1)).unwrap();

    let on_host = host.peer_key().expect("the host keeps its copy");
    assert_eq!(machine.enrollment_key(), Some(on_host.clone()));
    assert!(on_host.len() >= 64 && on_host.iter().all(u8::is_ascii_hexdigit));
    assert_eq!(mode(&host.peer_keys().join("box-1")), 0o600);
    assert_eq!(mode(&machine.enrollment_key_path()), 0o600);
    let key = String::from_utf8(on_host).unwrap();
    assert!(!machine.ssh_log().contains(&key), "the key went on a command line");

    // Another machine added gets another key.
    let other = Host::new();
    add(&other, &FakeMachine::new(), &machine.runner(), &Files(1)).unwrap();
    assert_ne!(other.peer_key().unwrap(), key.as_bytes());
}

#[test]
fn reconnecting_keeps_the_key_and_writes_one_only_when_the_host_has_none() {
    let host = Host::new();
    let machine = FakeMachine::new();
    add(&host, &machine, &machine.runner(), &Files(1)).unwrap();
    let first = host.peer_key().unwrap();
    let machines = host.machines();

    let runner = machine.runner();
    let (entry, _) = drive(&runner, &Files(1), |drive| machines.reconnect("box-1", drive));
    entry.unwrap();
    assert_eq!(host.peer_key(), Some(first.clone()));
    assert_eq!(runner.ran(Purpose::Enroll), 0);

    // A machine added before the host kept keys.
    std::fs::remove_file(host.peer_keys().join("box-1")).unwrap();
    let runner = machine.runner();
    let (entry, _) = drive(&runner, &Files(1), |drive| machines.reconnect("box-1", drive));
    entry.unwrap();
    let second = host.peer_key().expect("the reconnect wrote a key");
    assert_ne!(second, first);
    assert_eq!(machine.enrollment_key(), Some(second));
    // Only the key was written: the daemon kept running.
    assert_eq!(runner.ran(Purpose::Install), 0);
    assert_eq!(runner.ran(Purpose::Service), 0);
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
fn an_add_while_a_removal_is_on_its_way_is_refused_as_being_removed_until_it_finishes() {
    let host = Host::new();
    let machine = FakeMachine::new();
    add(&host, &machine, &machine.runner(), &Files(1)).unwrap();
    let machines = host.machines();

    // The removal is asked for and answered with, but hasn't run yet.
    machines.removing("box-1");
    assert!(matches!(
        machines.plan_add(&request(None)),
        Err(ManageError::BeingRemoved { .. })
    ));

    drive(&machine.runner(), &Files(1), |drive| machines.remove("box-1", drive)).0.unwrap();
    assert!(machines.plan_add(&request(None)).is_ok());
}

#[test]
fn an_add_while_a_removal_waits_for_its_machine_is_refused_as_being_removed() {
    let host = Host::new();
    let machine = FakeMachine::new();
    add(&host, &machine, &machine.runner(), &Files(1)).unwrap();
    let machines = host.machines();
    let runner = machine.runner().fault_at(0, Fault::MachineKilled);
    let cut = drive(&runner, &Files(1), |drive| machines.remove("box-1", drive)).0;
    assert!(matches!(cut, Err(ManageError::UninstallFailed { .. })));

    // A fresh Machines, as after a restart: the entry still says Removing.
    assert!(matches!(
        host.machines().plan_add(&request(None)),
        Err(ManageError::BeingRemoved { .. })
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
    // The daemon keeps running: nothing changed, so nothing restarts.
    assert_eq!(runner.ran(Purpose::Service), 0);
    assert_has(&machine, &Files(1));
}

#[test]
fn reconnecting_starts_the_service_again_only_when_it_stopped_and_leaves_a_new_build_to_migrate() {
    let host = Host::new();
    let machine = FakeMachine::new();
    add(&host, &machine, &machine.runner(), &Files(1)).unwrap();
    let machines = host.machines();

    machine.clear("active");
    let runner = machine.runner();
    let (entry, _) = drive(&runner, &Files(1), |drive| machines.reconnect("box-1", drive));
    assert_eq!(entry.unwrap().state, EntryState::Ready);
    assert_eq!(runner.ran(Purpose::Service), 1);

    // A newer build than the one installed: a reconnect doesn't upgrade a
    // machine that already runs humatad2 (UP-1); Migrate does.
    let runner = machine.runner();
    let (entry, _) = drive(&runner, &Files(2), |drive| machines.reconnect("box-1", drive));
    assert_eq!(entry.unwrap().state, EntryState::Ready);
    assert_eq!(runner.ran(Purpose::Install), 0);
    assert_eq!(runner.ran(Purpose::Service), 0);
    assert_has(&machine, &Files(1));
}

#[test]
fn a_new_build_is_left_alone_while_holders_run_and_nothing_waits_beside_it() {
    let host = Host::new();
    let machine = FakeMachine::new();
    add(&host, &machine, &machine.runner(), &Files(1)).unwrap();

    machine.set("holder");
    let machines = host.machines();
    let runner = machine.runner();
    let (entry, _) = drive(&runner, &Files(2), |drive| machines.reconnect("box-1", drive));
    // The machine stays usable on its build; Migrate brings the new one.
    assert_eq!(entry.unwrap().state, EntryState::Ready);
    assert_has(&machine, &Files(1));
    assert_eq!(machine.installed(".humata2_holder.staged"), None);
    assert_eq!(machine.installed(".humata.staged"), None);
    assert_eq!(runner.ran(Purpose::Install), 0);
}

#[test]
fn the_hosts_stack_settings_reach_a_machine_without_their_secrets_and_follow_the_file() {
    let host = Host::new();
    let machine = FakeMachine::new();
    let first = WithSettings(StackConfig::from_text("prefix = \"alice\"\ngh_token = \"ghp_abc\"\n"));
    add(&host, &machine, &machine.runner(), &first).unwrap();
    let settings = machine.home().join(".humata2/config/stack.toml");
    assert_eq!(std::fs::read(&settings).unwrap(), b"prefix = \"alice\"\n");

    // The file changes on the host; the tools are what they were.
    let machines = host.machines();
    let runner = machine.runner();
    let second = WithSettings(StackConfig::from_text("prefix = \"bob\"\n"));
    let (entry, _) = drive(&runner, &second, |drive| machines.reconnect("box-1", drive));

    assert_eq!(entry.unwrap().state, EntryState::Ready);
    assert_eq!(std::fs::read(&settings).unwrap(), b"prefix = \"bob\"\n");
    assert_has(&machine, &Files(1));
    // Settings alone don't restart the daemon.
    assert_eq!(runner.ran(Purpose::Service), 0);
}

#[test]
fn a_host_with_no_stack_settings_leaves_the_machines_own_copy_alone() {
    let host = Host::new();
    let machine = FakeMachine::new();
    let settings = machine.home().join(".humata2/config/stack.toml");
    std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
    std::fs::write(&settings, b"prefix = \"mine\"\n").unwrap();

    add(&host, &machine, &machine.runner(), &Files(1)).unwrap();

    assert_eq!(std::fs::read(&settings).unwrap(), b"prefix = \"mine\"\n");
}

#[test]
fn a_file_the_machine_lacks_is_put_back_and_the_others_are_not_touched() {
    let host = Host::new();
    let machine = FakeMachine::new();
    add(&host, &machine, &machine.runner(), &Files(1)).unwrap();
    std::fs::remove_file(machine.bin().join("humata2_holder")).unwrap();
    std::fs::remove_file(machine.bin().join("humata2_fusefs")).unwrap();
    let old = Files(1).payloads();
    let new = Files(2).payloads();

    let machines = host.machines();
    let runner = machine.runner();
    let (entry, _) = drive(&runner, &Files(2), |drive| machines.reconnect("box-1", drive));

    assert_eq!(entry.unwrap().state, EntryState::Ready);
    // One install per missing file: the holder and the FUSEFS helper.
    assert_eq!(runner.ran(Purpose::Install), 2);
    assert_eq!(machine.installed("humata2_holder").as_deref(), Some(&new[1].bytes()[..]));
    // The FUSEFS helper is one of the files a machine can lack, and is put back too.
    assert_eq!(machine.installed("humata2_fusefs").as_deref(), Some(&new[4].bytes()[..]));
    assert_eq!(machine.installed("humatad2").as_deref(), Some(&old[0].bytes()[..]));
    assert_eq!(machine.installed("humata").as_deref(), Some(&old[2].bytes()[..]));
    assert_eq!(machine.installed("stack").as_deref(), Some(&old[3].bytes()[..]));
}

#[test]
fn a_first_install_writes_the_releases_bundle_json_after_the_tools() {
    let host = Host::new();
    let machine = FakeMachine::new();

    add(&host, &machine, &machine.runner(), &Released(1)).unwrap();

    assert_has(&machine, &Files(1));
    assert_eq!(machine.installed("bundle.json").as_deref(), Some(Released::BUNDLE));
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

/// LD-12: forgetting a lost machine drops it from this host without
/// reaching it: no entry, no pinned host key and no enrollment key are left,
/// so adding it again is a new machine. Humata stays on the lost machine.
#[test]
fn forgetting_an_unreachable_machine_drops_it_here_without_reaching_it() {
    let host = Host::new();
    let machine = FakeMachine::new();
    add(&host, &machine, &machine.runner(), &Files(1)).unwrap();
    machine.set("down");
    let calls = machine.ssh_calls();

    host.machines().forget("box-1").unwrap();

    assert_eq!(host.machines().list().unwrap(), []);
    assert!(!host.pinned(), "the host key is still pinned");
    assert_eq!(host.peer_key(), None, "the host still keeps an enrollment key");
    assert_eq!(machine.ssh_calls(), calls, "forgetting reached the machine");
    assert!(machine.unit_installed(), "the lost machine was changed");
    assert!(matches!(host.machines().forget("box-1"), Err(ManageError::NotFound { .. })));
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

// ---- Machines made through a provider (MP-3, MP-7, MP-8, MP-19, MP-21) ----

mod provider_machines {
    use std::collections::BTreeMap;

    use humata2_machines::{Action, IdentityError, MachineStatus, OpStep, Provider, parse_status};

    use super::*;

    const HOST_KEY: &str = fake_machine::HOST_KEY;

    impl Host {
        fn dir(&self) -> &std::path::Path {
            self.folder.path()
        }

        fn made(&self) -> bool {
            self.dir().join("created").exists()
        }

        fn write_identity(&self, host_key: &str) {
            let identity = format!(
                r#"{{"name":"box-1","ssh":{{"host":"box-1.internal","user":"me","port":22,"host_key":"{host_key}"}},"os":"linux","arch":"x86_64"}}"#
            );
            std::fs::write(self.dir().join("identity.json"), identity).unwrap();
        }

        /// A provider reaching its machine from an identity it prints.
        fn identity_provider(&self) -> Provider {
            let dir = self.dir().display();
            parse(&format!(
                "[providers.fake]\n\
                 params = [{{ name = \"size\", prompt = \"Size\", default = \"small\" }}, {{ name = \"zone\", prompt = \"Zone\" }}]\n\
                 create = 'touch {dir}/created'\n\
                 identity = 'cat {dir}/identity.json'\n\
                 start = 'touch {dir}/started'\n\
                 status = 'echo stopped'\n\
                 delete = 'rm -f {dir}/created'\n"
            ))
        }

        /// A provider whose connect command runs the remote command over
        /// the fake ssh, checking the key in a file of the test's own.
        fn connect_provider(&self) -> Provider {
            let dir = self.dir().display();
            std::fs::write(self.dir().join("connect_known_hosts"), format!("humata2.box-1 {HOST_KEY}\n")).unwrap();
            parse(&format!(
                "[providers.viaconnect]\n\
                 params = [{{ name = \"zone\", prompt = \"Zone\", default = \"a\" }}]\n\
                 create = 'touch {dir}/created'\n\
                 connect = 'ssh -F /dev/null -o UserKnownHostsFile={dir}/connect_known_hosts -o HostKeyAlias=humata2.box-1 -- box-1 {{cmd}}'\n\
                 delete = 'rm -f {dir}/created'\n"
            ))
        }
    }

    fn parse(text: &str) -> Provider {
        Provider::parse(text).unwrap().pop().unwrap()
    }

    fn zone() -> BTreeMap<String, String> {
        BTreeMap::from([("zone".to_owned(), "fsn1".to_owned())])
    }

    fn create(
        host: &Host,
        provider: &Provider,
        values: BTreeMap<String, String>,
        runner: &FakeSshRunner,
    ) -> (Result<humata2_machines::HostEntry, ManageError>, Vec<StepEvent>) {
        let machines = host.machines();
        let plan = match machines.plan_create(provider, "box-1", values) {
            Ok(plan) => plan,
            Err(error) => return (Err(error), Vec::new()),
        };
        drive(runner, &Files(1), |drive| machines.create(plan, drive))
    }

    fn create_ok(host: &Host, provider: &Provider, machine: &FakeMachine) -> humata2_machines::HostEntry {
        create(host, provider, zone(), &machine.runner()).0.unwrap()
    }

    #[test]
    fn a_machine_made_by_an_identity_provider_is_listed_with_its_provider_and_reconnects_through_it() {
        let host = Host::new();
        let machine = FakeMachine::new();
        host.write_identity(HOST_KEY);
        let provider = host.identity_provider();

        let (entry, events) = create(&host, &provider, zone(), &machine.runner());
        let entry = entry.unwrap();

        assert_eq!(entry.state, EntryState::Ready);
        assert_eq!(entry.provider.as_ref().map(|p| p.id.as_str()), Some("fake"));
        assert_eq!(host.machines().list().unwrap(), [entry]);
        assert!(host.made());
        assert_has(&machine, &Files(1));
        assert!(host.pinned(), "the identity's host key is pinned with no prompt");
        assert!(host.peer_key().is_some());
        for step in [OpStep::Create, OpStep::Identity, OpStep::Connect, OpStep::Install] {
            assert!(events.contains(&StepEvent::Done(step)), "{step:?}");
        }
        // The stored link survives a restart: a reconnect and the peer link use it.
        let machines = host.machines();
        let (reconnected, _) = drive(&machine.runner(), &Files(1), |drive| machines.reconnect("box-1", drive));
        assert_eq!(reconnected.unwrap().state, EntryState::Ready);
        machines.peer_link("box-1").unwrap();
    }

    #[test]
    fn a_machine_made_by_a_connect_provider_is_reached_through_its_command_and_removed_through_it() {
        let host = Host::new();
        let machine = FakeMachine::new();
        let provider = host.connect_provider();

        let entry = create(&host, &provider, BTreeMap::new(), &machine.runner()).0.unwrap();

        assert_eq!(entry.provider.as_ref().map(|p| p.id.as_str()), Some("viaconnect"));
        assert_has(&machine, &Files(1));
        let machines = host.machines();
        machines.peer_link("box-1").unwrap();
        let (removed, _) = drive(&machine.runner(), &Files(1), |drive| machines.remove("box-1", drive));
        removed.unwrap();
        assert_clean(&host, &machine);
    }

    #[test]
    fn params_that_are_undeclared_or_missing_are_refused_by_name_before_anything_runs() {
        let host = Host::new();
        let machine = FakeMachine::new();
        let provider = host.identity_provider();
        let runner = machine.runner();

        let missing = create(&host, &provider, BTreeMap::new(), &runner).0;
        let mut extra = zone();
        extra.insert("colour".to_owned(), "red".to_owned());
        let undeclared = create(&host, &provider, extra, &runner).0;

        match missing {
            Err(ManageError::Params {
                source: humata2_machines::ValuesError::Missing { param, .. },
            }) => assert_eq!(param, "zone"),
            other => panic!("expected a missing param, got {other:?}"),
        }
        match undeclared {
            Err(ManageError::Params {
                source: humata2_machines::ValuesError::Undeclared { param, .. },
            }) => assert_eq!(param, "colour"),
            other => panic!("expected an undeclared param, got {other:?}"),
        }
        assert_eq!(runner.calls(), 0);
        assert!(!host.made());
        assert_eq!(host.machines().list().unwrap(), []);
    }

    #[test]
    fn a_failed_step_leaves_no_half_machine_but_keeps_what_create_made_for_delete() {
        for connect in [false, true] {
            let host = Host::new();
            let machine = FakeMachine::new();
            host.write_identity(HOST_KEY);
            let provider = if connect {
                host.connect_provider()
            } else {
                host.identity_provider()
            };
            machine.set_uname("FreeBSD amd64");
            let values = if connect { BTreeMap::new() } else { zone() };

            let (result, _) = create(&host, &provider, values.clone(), &machine.runner());

            assert!(matches!(result, Err(ManageError::Step { .. })), "{result:?}");
            assert_clean(&host, &machine);
            assert!(host.made(), "what create made is left for Delete what was made");

            let machines = host.machines();
            let (deleted, _) = drive(&machine.runner(), &Files(1), |drive| {
                machines.delete_made(&provider, "box-1", &values, drive)
            });
            deleted.unwrap();
            assert!(!host.made());
        }
    }

    #[test]
    fn a_bad_identity_field_is_named_and_leaves_nothing_listed() {
        let host = Host::new();
        let machine = FakeMachine::new();
        host.write_identity("not-a-key");
        let provider = host.identity_provider();

        let (result, _) = create(&host, &provider, zone(), &machine.runner());

        match result {
            Err(ManageError::Step {
                step,
                source: SetupError::Identity(IdentityError::Field { field, .. }),
                ..
            }) => {
                assert_eq!(step, OpStep::Identity);
                assert_eq!(field, "ssh.host_key");
            }
            other => panic!("expected a bad identity field, got {other:?}"),
        }
        assert_clean(&host, &machine);
    }

    #[test]
    fn a_login_the_connect_needs_fails_the_step_and_is_undone() {
        let host = Host::new();
        let machine = FakeMachine::new();
        let dir = host.dir().display();
        let provider = parse(&format!(
            "[providers.asks]\ncreate = 'true'\nconnect = 'echo \"Password:\" >&2; exit 5; : {{cmd}}'\ndelete = 'rm -f {dir}/created'\n"
        ));

        let (result, _) = create(&host, &provider, BTreeMap::new(), &machine.runner());

        assert!(matches!(result, Err(ManageError::ProviderLoginNeeded { .. })), "{result:?}");
        assert_clean(&host, &machine);
    }

    #[test]
    fn a_create_cancelled_after_any_step_is_undone() {
        for after in [OpStep::Create, OpStep::Identity, OpStep::Connect, OpStep::Check, OpStep::Install] {
            let host = Host::new();
            let machine = FakeMachine::new();
            host.write_identity(HOST_KEY);
            let provider = host.identity_provider();
            let machines = host.machines();
            let plan = machines.plan_create(&provider, "box-1", zone()).unwrap();
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

            let result = machines.create(plan, &mut drive);

            assert!(matches!(result, Err(ManageError::Cancelled { .. })), "after {after:?}: {result:?}");
            assert_clean(&host, &machine);
        }
    }

    /// How many commands a whole create runs.
    fn create_calls() -> usize {
        let host = Host::new();
        let machine = FakeMachine::new();
        host.write_identity(HOST_KEY);
        let runner = machine.runner();
        create(&host, &host.identity_provider(), zone(), &runner).0.unwrap();
        runner.calls()
    }

    #[test]
    fn a_create_cut_short_at_any_step_is_undone_after_a_restart() {
        let steps = create_calls();
        assert!(steps >= 5, "a create runs create, identity, connect, check, install and start");
        for fault in [Fault::HostKilled, Fault::MachineKilled] {
            for at in 0..steps {
                let host = Host::new();
                let machine = FakeMachine::new();
                host.write_identity(HOST_KEY);
                let provider = host.identity_provider();
                let runner = machine.runner().fault_at(at, fault);

                let result = panic::catch_unwind(AssertUnwindSafe(|| create(&host, &provider, zone(), &runner).0));
                match fault {
                    Fault::HostKilled => assert!(result.is_err(), "call {at} wasn't reached"),
                    Fault::MachineKilled => assert!(result.unwrap().is_err(), "call {at}"),
                }

                machine.clear("down");
                recover(&host, &machine);

                assert_clean(&host, &machine);
                // The VM create made is not Humata's to undo on its own. A
                // host killed at the first call never ran `create` at all.
                let create_ran = !(at == 0 && matches!(fault, Fault::HostKilled));
                assert_eq!(host.made(), create_ran, "call {at}");
            }
        }
    }

    #[test]
    fn a_provider_action_runs_its_command_for_its_machine_and_an_undefined_one_is_refused() {
        let host = Host::new();
        let machine = FakeMachine::new();
        host.write_identity(HOST_KEY);
        let provider = host.identity_provider();
        create_ok(&host, &provider, &machine);
        let machines = host.machines();

        let (status, _) = drive(&machine.runner(), &Files(1), |drive| {
            machines.run_action(&provider, "box-1", Action::Status, drive)
        });
        let (stop, _) = drive(&machine.runner(), &Files(1), |drive| {
            machines.run_action(&provider, "box-1", Action::Stop, drive)
        });
        let (start, _) = drive(&machine.runner(), &Files(1), |drive| {
            machines.run_action(&provider, "box-1", Action::Start, drive)
        });

        assert_eq!(parse_status(&status.unwrap().stdout), MachineStatus::Stopped);
        assert!(matches!(
            stop,
            Err(ManageError::ActionUndefined { action: Action::Stop, .. })
        ));
        start.unwrap();
        assert!(host.dir().join("started").exists());
        // A machine another provider made isn't this provider's to run.
        let other = host.connect_provider();
        let (wrong, _) = drive(&machine.runner(), &Files(1), |drive| {
            machines.run_action(&other, "box-1", Action::Start, drive)
        });
        assert!(matches!(wrong, Err(ManageError::NotProviderMachine { .. })));
    }

    #[test]
    fn deleting_a_provider_machine_removes_it_then_runs_the_providers_delete() {
        let host = Host::new();
        let machine = FakeMachine::new();
        host.write_identity(HOST_KEY);
        let provider = host.identity_provider();
        create_ok(&host, &provider, &machine);
        let machines = host.machines();

        let (deleted, events) = drive(&machine.runner(), &Files(1), |drive| {
            machines.delete_machine(&provider, "box-1", drive)
        });

        deleted.unwrap();
        assert_clean(&host, &machine);
        assert!(!host.made(), "the provider's delete ran");
        assert!(events.contains(&StepEvent::Done(OpStep::Provider)));
    }
}
