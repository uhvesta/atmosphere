//! Setting a machine up step by step against a fake runner: what each step
//! runs, how it retries, and what a failure offers.

mod support;

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use humata2_machines::{
    BuildsError, Exit, IdentityError, InstallProblem, InstalledFile, Link, MachineName, Progress, Provider,
    Purpose, Remedy, RetryPolicy, ServiceKind, Setup, SetupError, SshAccess, SshTarget, Step,
    ValuesError,
};
use support::{payloads, CheckFacts, FakeBuilds, FakeRunner};

const KNOWN_HOSTS: &str = "/humata2-test/known_hosts";

fn name(value: &str) -> MachineName {
    MachineName::new(value).unwrap()
}

fn lima(machine: &str) -> Setup {
    Setup::new(
        Provider::lima().unwrap(),
        name(machine),
        BTreeMap::new(),
        Path::new(KNOWN_HOSTS),
    )
    .unwrap()
}

fn cloud(with_delete: bool) -> Provider {
    let delete = if with_delete {
        "delete = \"cloudctl delete {name}\"\n"
    } else {
        ""
    };
    let text = format!(
        "[providers.cloud]\ncreate = \"cloudctl create {{name}}\"\nidentity = \"cloudctl identity {{name}}\"\n{delete}"
    );
    Provider::parse(&text).unwrap().pop().unwrap()
}

/// Steps until setup stops, as the daemon would without sleeping; returns
/// where it stopped and the delays it was asked to wait.
fn drive(setup: &mut Setup, runner: &FakeRunner, builds: &FakeBuilds) -> (Progress, Vec<Duration>) {
    let mut delays = Vec::new();
    for _ in 0..100 {
        match setup.step(runner, builds) {
            Progress::Continue => {}
            Progress::RetryAfter { delay, .. } => delays.push(delay),
            other => return (other, delays),
        }
    }
    panic!("setup didn't stop");
}

/// One successful install per file the build ships, so new shipped files
/// need no per-test edits.
fn answer_installs(runner: &FakeRunner) {
    for _ in payloads() {
        runner.ok(Purpose::Install, "");
    }
}

fn answer_up_to_service(runner: &FakeRunner, facts: &CheckFacts) {
    runner
        .ok(Purpose::Create, "")
        .ok(Purpose::Probe, "Linux x86_64\n")
        .ok(Purpose::Check, &facts.stdout());
    answer_installs(runner);
}

#[test]
fn a_lima_machine_is_created_checked_installed_and_started() {
    let runner = FakeRunner::new();
    answer_up_to_service(&runner, &CheckFacts::default());
    runner.ok(Purpose::Service, "");
    let mut setup = lima("box-1");

    let (progress, delays) = drive(&mut setup, &runner, &FakeBuilds::files());

    let Progress::Ready(ready) = progress else {
        panic!("{progress:?}");
    };
    assert!(delays.is_empty());
    assert_eq!(ready.service, ServiceKind::SystemdUser);
    assert!(ready.link.is_connect());
    assert_eq!(ready.link.machine(), &name("box-1"));
    let sent: Vec<Vec<u8>> = runner
        .ran(Purpose::Install)
        .into_iter()
        .map(|invocation| invocation.stdin.unwrap().to_vec())
        .collect();
    let built: Vec<Vec<u8>> = payloads().iter().map(|payload| payload.bytes().to_vec()).collect();
    assert_eq!(sent, built);
}

#[test]
fn files_already_installed_are_not_sent_again() {
    let runner = FakeRunner::new();
    let facts = CheckFacts {
        installed: payloads()
            .iter()
            .map(|payload| (payload.file(), payload.sha256().to_owned()))
            .collect(),
        ..CheckFacts::default()
    };
    runner
        .ok(Purpose::Create, "")
        .ok(Purpose::Probe, "Linux x86_64\n")
        .ok(Purpose::Check, &facts.stdout())
        .ok(Purpose::Service, "");
    let mut setup = lima("box-1");

    let (progress, _) = drive(&mut setup, &runner, &FakeBuilds::files());

    assert!(matches!(progress, Progress::Ready(_)), "{progress:?}");
    assert!(runner.ran(Purpose::Install).is_empty());
}

#[test]
fn only_the_file_that_differs_is_sent() {
    let runner = FakeRunner::new();
    let facts = CheckFacts {
        installed: payloads()[1..]
            .iter()
            .map(|payload| (payload.file(), payload.sha256().to_owned()))
            .collect(),
        ..CheckFacts::default()
    };
    runner
        .ok(Purpose::Create, "")
        .ok(Purpose::Probe, "Linux aarch64\n")
        .ok(Purpose::Check, &facts.stdout())
        .ok(Purpose::Install, "")
        .ok(Purpose::Service, "");
    let mut setup = lima("box-1");

    let (progress, _) = drive(&mut setup, &runner, &FakeBuilds::files());

    assert!(matches!(progress, Progress::Ready(_)), "{progress:?}");
    let installs = runner.ran(Purpose::Install);
    assert_eq!(installs.len(), 1);
    assert_eq!(
        installs[0].stdin.as_deref(),
        Some(&payloads()[0].bytes()[..])
    );
}

#[test]
fn connect_retries_with_growing_waits_while_the_machine_boots() {
    let runner = FakeRunner::new();
    runner
        .ok(Purpose::Create, "")
        .answer(Purpose::Probe, Exit::Code(255), "", "Connection refused")
        .answer(Purpose::Probe, Exit::Code(255), "", "Connection refused")
        .ok(Purpose::Probe, "Linux x86_64\n")
        .ok(Purpose::Check, &CheckFacts::default().stdout())
        .ok(Purpose::Service, "");
    answer_installs(&runner);
    let mut setup = lima("box-1");

    let (progress, delays) = drive(&mut setup, &runner, &FakeBuilds::files());

    assert!(matches!(progress, Progress::Ready(_)), "{progress:?}");
    assert_eq!(delays.len(), 2);
    assert!(delays[1] > delays[0]);
}

#[test]
fn a_machine_that_never_answers_is_unreachable_and_can_be_deleted() {
    let runner = FakeRunner::new();
    runner.ok(Purpose::Create, "");
    for _ in 0..3 {
        runner.answer(Purpose::Probe, Exit::Code(255), "", "No route to host");
    }
    let mut setup = lima("box-1").with_retry(RetryPolicy {
        attempts: 3,
        first_delay: Duration::from_secs(1),
        max_delay: Duration::from_secs(4),
    });

    let (progress, delays) = drive(&mut setup, &runner, &FakeBuilds::files());

    let Progress::Failed(failure) = progress else {
        panic!("{progress:?}");
    };
    assert_eq!(delays.len(), 2);
    assert_eq!(failure.step, Step::Connect);
    assert!(matches!(failure.error, SetupError::Unreachable { attempts: 3, .. }));
    assert!(failure.output_tail.contains("No route to host"));
    assert!(failure.remedies.contains(&Remedy::TryAgain));
    assert!(failure.remedies.contains(&Remedy::OpenInteractively));
    assert!(failure.remedies.contains(&Remedy::DeleteWhatWasMade));
    let delete = setup.delete_what_was_made().expect("a delete command");
    assert_eq!(delete.purpose, Purpose::Delete);
}

#[test]
fn a_password_prompt_asks_for_an_interactive_login() {
    let runner = FakeRunner::new();
    runner
        .ok(Purpose::Create, "")
        .answer(Purpose::Probe, Exit::Code(255), "", "dev@10.0.0.5's Password: ");
    let mut setup = lima("box-1");

    let (progress, delays) = drive(&mut setup, &runner, &FakeBuilds::files());

    let Progress::NeedsLogin(login) = progress else {
        panic!("{progress:?}");
    };
    assert!(delays.is_empty());
    assert_eq!(
        login.reason,
        humata2_machines::LoginReason::Prompted(humata2_machines::Prompt {
            kind: humata2_machines::PromptKind::Password,
            line: "dev@10.0.0.5's Password:".to_owned(),
        })
    );
    assert_eq!(setup.current(), Step::Connect);
}

#[test]
fn a_connect_that_hangs_asks_for_an_interactive_login() {
    let runner = FakeRunner::new();
    runner
        .ok(Purpose::Create, "")
        .answer(Purpose::Probe, Exit::TimedOut, "", "");
    let mut setup = lima("box-1");

    let (progress, _) = drive(&mut setup, &runner, &FakeBuilds::files());

    let Progress::NeedsLogin(login) = progress else {
        panic!("{progress:?}");
    };
    assert_eq!(login.reason, humata2_machines::LoginReason::Hung);
}

#[test]
fn a_failed_create_offers_to_fix_the_provider_and_delete_what_was_made() {
    let runner = FakeRunner::new();
    runner.answer(Purpose::Create, Exit::Code(1), "", "quota exceeded");
    let mut setup = lima("box-1");

    let (progress, _) = drive(&mut setup, &runner, &FakeBuilds::files());

    let Progress::Failed(failure) = progress else {
        panic!("{progress:?}");
    };
    assert_eq!(failure.step, Step::Create);
    assert!(matches!(failure.error, SetupError::CommandFailed { printed: true, .. }));
    assert!(failure.output_tail.contains("quota exceeded"));
    assert!(failure.remedies.contains(&Remedy::FixProvider));
    assert!(failure.remedies.contains(&Remedy::DeleteWhatWasMade));
    assert_eq!(setup.current(), Step::Create);
}

#[test]
fn a_failed_step_that_printed_nothing_carries_no_output_and_does_not_promise_any() {
    let runner = FakeRunner::new();
    runner.answer(Purpose::Create, Exit::Code(255), "", "");
    let mut setup = lima("box-1");

    let (progress, _) = drive(&mut setup, &runner, &FakeBuilds::files());

    let Progress::Failed(failure) = progress else {
        panic!("{progress:?}");
    };
    assert!(failure.output_tail.is_empty(), "{:?}", failure.output_tail);
    assert!(matches!(failure.error, SetupError::CommandFailed { printed: false, .. }));
    assert!(!failure.error.to_string().contains("below"), "{}", failure.error);
}

#[test]
fn an_install_that_stops_quietly_promises_no_output() {
    let runner = FakeRunner::new();
    runner
        .ok(Purpose::Create, "")
        .ok(Purpose::Probe, "Linux x86_64\n")
        .ok(Purpose::Check, &CheckFacts::default().stdout());
    runner.answer(Purpose::Install, Exit::Code(9), "", "");
    let mut setup = lima("box-1");

    let (progress, _) = drive(&mut setup, &runner, &FakeBuilds::files());

    let Progress::Failed(failure) = progress else {
        panic!("{progress:?}");
    };
    assert!(failure.output_tail.is_empty(), "{:?}", failure.output_tail);
    assert!(!failure.error.to_string().contains("below"), "{}", failure.error);
}

#[test]
fn without_a_delete_command_nothing_offers_to_delete() {
    let runner = FakeRunner::new();
    runner.answer(Purpose::Create, Exit::Code(1), "", "");
    let mut setup = Setup::new(
        cloud(false),
        name("box-1"),
        BTreeMap::new(),
        Path::new(KNOWN_HOSTS),
    )
    .unwrap();

    let (progress, _) = drive(&mut setup, &runner, &FakeBuilds::files());

    let Progress::Failed(failure) = progress else {
        panic!("{progress:?}");
    };
    assert!(!failure.remedies.contains(&Remedy::DeleteWhatWasMade));
    assert!(setup.delete_what_was_made().is_none());
}

#[test]
fn a_try_again_after_a_failure_runs_the_same_step() {
    let runner = FakeRunner::new();
    answer_up_to_service(&runner, &CheckFacts::default());
    runner
        .answer(Purpose::Service, Exit::Code(1), "", "Failed to connect to bus")
        .ok(Purpose::Service, "");
    let mut setup = lima("box-1");

    let (first, _) = drive(&mut setup, &runner, &FakeBuilds::files());
    let (second, _) = drive(&mut setup, &runner, &FakeBuilds::files());

    assert!(matches!(first, Progress::Failed(ref failure) if failure.step == Step::Service));
    assert!(matches!(second, Progress::Ready(_)), "{second:?}");
    assert_eq!(runner.ran(Purpose::Install).len(), payloads().len());
}

#[test]
fn an_identity_machine_is_reached_over_ssh_with_humatas_host_keys() {
    let runner = FakeRunner::new();
    runner
        .ok(Purpose::Create, "")
        .ok(
            Purpose::Identity,
            r#"{"name":"box-1","ssh":{"host":"10.0.0.5","port":2222,"user":"dev","identity_file":"/keys/box-1"},"os":"linux","arch":"x86_64"}"#,
        )
        .ok(Purpose::Probe, "Linux x86_64\n")
        .ok(Purpose::Check, &CheckFacts::default().stdout())
        .ok(Purpose::Service, "");
    answer_installs(&runner);
    let mut setup =
        Setup::new(cloud(true), name("box-1"), BTreeMap::new(), Path::new(KNOWN_HOSTS)).unwrap();

    let (progress, _) = drive(&mut setup, &runner, &FakeBuilds::files());

    assert!(matches!(progress, Progress::Ready(_)), "{progress:?}");
    let probe = &runner.ran(Purpose::Probe)[0];
    assert_eq!(probe.program, "ssh");
    for wanted in [
        "BatchMode=yes",
        "StrictHostKeyChecking=yes",
        "HostKeyAlias=humata2.box-1",
        "GlobalKnownHostsFile=/dev/null",
    ] {
        assert!(probe.args.iter().any(|arg| arg == wanted), "{wanted} in {:?}", probe.args);
    }
    assert!(probe.args.iter().any(|arg| arg.contains(KNOWN_HOSTS)));
    assert!(probe.args.windows(2).any(|pair| pair == ["-p", "2222"]));
}

#[test]
fn identity_output_with_a_key_is_refused_and_not_shown() {
    let runner = FakeRunner::new();
    runner.ok(Purpose::Create, "").ok(
        Purpose::Identity,
        "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAA\n-----END OPENSSH PRIVATE KEY-----\n",
    );
    let mut setup =
        Setup::new(cloud(true), name("box-1"), BTreeMap::new(), Path::new(KNOWN_HOSTS)).unwrap();

    let (progress, _) = drive(&mut setup, &runner, &FakeBuilds::files());

    let Progress::Failed(failure) = progress else {
        panic!("{progress:?}");
    };
    assert!(matches!(failure.error, SetupError::Identity(IdentityError::KeyMaterial)));
    assert!(failure.output_tail.is_empty());
    let shown = format!("{} {:?}", failure.error, failure);
    assert!(!shown.contains("b3BlbnNzaC1rZXktdjEAAAAA"));
}

#[test]
fn an_identity_whose_platform_disagrees_with_the_machine_fails_the_check() {
    let runner = FakeRunner::new();
    runner
        .ok(Purpose::Create, "")
        .ok(
            Purpose::Identity,
            r#"{"name":"box-1","ssh":{"host":"10.0.0.5","user":"dev"},"os":"linux","arch":"aarch64"}"#,
        )
        .ok(Purpose::Probe, "Linux x86_64\n")
        .ok(Purpose::Check, &CheckFacts::default().stdout());
    let mut setup =
        Setup::new(cloud(true), name("box-1"), BTreeMap::new(), Path::new(KNOWN_HOSTS)).unwrap();

    let (progress, _) = drive(&mut setup, &runner, &FakeBuilds::files());

    let Progress::Failed(failure) = progress else {
        panic!("{progress:?}");
    };
    assert!(matches!(failure.error, SetupError::IdentityMismatch { .. }));
    assert!(failure.remedies.contains(&Remedy::FixProvider));
}

#[test]
fn the_check_refuses_machines_setup_cant_use() {
    let cases = [
        (
            CheckFacts {
                os: "Darwin",
                arch: "arm64",
                ..CheckFacts::default()
            },
            "unsupported",
        ),
        (
            CheckFacts {
                git: false,
                ..CheckFacts::default()
            },
            "git",
        ),
        (
            CheckFacts {
                disk_free_kib: 512 * 1024,
                ..CheckFacts::default()
            },
            "disk",
        ),
    ];
    for (facts, case) in cases {
        let runner = FakeRunner::new();
        runner
            .ok(Purpose::Create, "")
            .ok(Purpose::Probe, "Linux x86_64\n")
            .ok(Purpose::Check, &facts.stdout());
        let mut setup = lima("box-1");

        let (progress, _) = drive(&mut setup, &runner, &FakeBuilds::files());

        let Progress::Failed(failure) = progress else {
            panic!("{case}: {progress:?}");
        };
        assert_eq!(failure.step, Step::Check, "{case}");
        let expected = match case {
            "unsupported" => matches!(failure.error, SetupError::Unsupported { .. }),
            "git" => matches!(failure.error, SetupError::GitMissing { .. }),
            _ => matches!(failure.error, SetupError::DiskLow { .. }),
        };
        assert!(expected, "{case}: {:?}", failure.error);
    }
}

#[test]
fn without_systemd_setsid_or_crontab_setup_falls_back_to_detached_daemonize() {
    let runner = FakeRunner::new();
    let facts = CheckFacts {
        systemd: false,
        cron: false,
        ..CheckFacts::default()
    };
    answer_up_to_service(&runner, &facts);
    runner.ok(Purpose::Service, "");
    let mut setup = lima("box-1");

    let (progress, _) = drive(&mut setup, &runner, &FakeBuilds::files());

    let Progress::Ready(ready) = progress else {
        panic!("expected detached fallback to succeed, got {progress:?}");
    };
    assert_eq!(ready.service, ServiceKind::Detached);
    let service_scripts = runner.ran(Purpose::Service);
    assert_eq!(service_scripts.len(), 1);
    let script_text = service_scripts[0].args.join(" ");
    assert!(
        script_text.contains("serve --role machine --daemonize"),
        "expected detached script to invoke --daemonize, got: {script_text}"
    );
}

#[test]
fn without_lingering_and_without_setsid_or_crontab_detached_remedy_is_offered() {
    let runner = FakeRunner::new();
    let facts = CheckFacts {
        systemd: true,
        cron: false,
        ..CheckFacts::default()
    };
    answer_up_to_service(&runner, &facts);
    runner
        .answer(Purpose::Service, Exit::Code(75), "", "lingering can't be turned on")
        .ok(Purpose::Service, "");
    let mut setup = lima("box-1");

    let (progress, _) = drive(&mut setup, &runner, &FakeBuilds::files());

    let Progress::Failed(failure) = progress else {
        panic!("{progress:?}");
    };
    assert!(matches!(failure.error, SetupError::LingerUnavailable { .. }));
    assert!(failure.remedies.contains(&Remedy::StartWithoutSystemd));

    setup.use_without_systemd();
    let (progress, _) = drive(&mut setup, &runner, &FakeBuilds::files());

    let Progress::Ready(ready) = progress else {
        panic!("{progress:?}");
    };
    assert_eq!(ready.service, ServiceKind::Detached);
    let scripts = runner.ran(Purpose::Service);
    assert_eq!(scripts.len(), 2);
    assert!(scripts[1].args.join(" ").contains("serve --role machine --daemonize"));
}

#[test]
fn reconnecting_a_detached_machine_starts_daemonize_again_when_not_running() {
    let runner = FakeRunner::new();
    let files = payloads();
    let installed: Vec<(InstalledFile, String)> = files
        .iter()
        .map(|p| (p.file(), p.sha256().to_owned()))
        .collect();
    let facts = CheckFacts {
        systemd: false,
        cron: false,
        installed,
        ..CheckFacts::default()
    };
    // First reconnect: humatad2 is NOT running (e.g. container restarted), binaries are intact.
    runner
        .ok(Purpose::Probe, "Linux x86_64\n")
        .ok(Purpose::Check, &format!("{}humata2.running=no\n", facts.stdout()))
        .ok(Purpose::Service, "");
    let mut setup = ssh_machine(Path::new(KNOWN_HOSTS));

    let (progress, _) = drive(&mut setup, &runner, &FakeBuilds::files());

    let Progress::Ready(ready) = progress else {
        panic!("{progress:?}");
    };
    assert_eq!(ready.service, ServiceKind::Detached);
    assert!(runner.ran(Purpose::Install).is_empty(), "binaries already match");
    let service_scripts = runner.ran(Purpose::Service);
    assert_eq!(service_scripts.len(), 1);
    assert!(service_scripts[0].args.join(" ").contains("serve --role machine --daemonize"));

    // Second reconnect: humatad2 IS already running detached -> no restart needed.
    let runner2 = FakeRunner::new();
    runner2
        .ok(Purpose::Probe, "Linux x86_64\n")
        .ok(Purpose::Check, &format!("{}humata2.running=detached\n", facts.stdout()));
    let mut setup2 = ssh_machine(Path::new(KNOWN_HOSTS));

    let (progress2, _) = drive(&mut setup2, &runner2, &FakeBuilds::files());
    let Progress::Ready(ready2) = progress2 else {
        panic!("{progress2:?}");
    };
    assert_eq!(ready2.service, ServiceKind::Detached);
    assert!(runner2.ran(Purpose::Service).is_empty(), "already running detached");
}

#[test]
fn a_platform_without_a_build_fails_the_check() {
    let runner = FakeRunner::new();
    runner
        .ok(Purpose::Create, "")
        .ok(Purpose::Probe, "Linux x86_64\n")
        .ok(Purpose::Check, &CheckFacts::default().stdout());
    let mut setup = lima("box-1");

    let (progress, _) = drive(&mut setup, &runner, &FakeBuilds::none());

    assert!(
        matches!(progress, Progress::Failed(ref failure) if matches!(failure.error, SetupError::Builds(BuildsError::NoBuild { .. }))),
        "{progress:?}"
    );
}

#[test]
fn a_changed_copy_is_reported_as_a_checksum_mismatch() {
    let runner = FakeRunner::new();
    runner
        .ok(Purpose::Create, "")
        .ok(Purpose::Probe, "Linux x86_64\n")
        .ok(Purpose::Check, &CheckFacts::default().stdout())
        .answer(Purpose::Install, Exit::Code(65), "", "SHA-256 0000, expected 1111");
    let mut setup = lima("box-1");

    let (progress, _) = drive(&mut setup, &runner, &FakeBuilds::files());

    let Progress::Failed(failure) = progress else {
        panic!("{progress:?}");
    };
    assert_eq!(failure.step, Step::Install);
    assert!(matches!(
        failure.error,
        SetupError::Install {
            file: InstalledFile::Humatad,
            problem: InstallProblem::ChecksumMismatch,
            ..
        }
    ));
}

#[test]
fn without_lingering_an_admin_command_or_setsid_is_offered() {
    let runner = FakeRunner::new();
    answer_up_to_service(&runner, &CheckFacts::default());
    runner
        .answer(Purpose::Service, Exit::Code(75), "", "lingering can't be turned on")
        .ok(Purpose::Service, "");
    let mut setup = lima("box-1");

    let (progress, _) = drive(&mut setup, &runner, &FakeBuilds::files());

    let Progress::Failed(failure) = progress else {
        panic!("{progress:?}");
    };
    let SetupError::LingerUnavailable { ref admin_command, ref user, .. } = failure.error else {
        panic!("{:?}", failure.error);
    };
    assert_eq!(user, "dev");
    assert!(admin_command.contains("dev"));
    assert!(failure
        .remedies
        .contains(&Remedy::RunAsAdmin { command: admin_command.clone() }));
    assert!(failure.remedies.contains(&Remedy::StartWithoutSystemd));

    setup.use_setsid();
    let (progress, _) = drive(&mut setup, &runner, &FakeBuilds::files());

    let Progress::Ready(ready) = progress else {
        panic!("{progress:?}");
    };
    assert_eq!(ready.service, ServiceKind::Setsid);
    let scripts = runner.ran(Purpose::Service);
    assert_ne!(scripts[0].args, scripts[1].args);
}

#[test]
fn values_the_provider_doesnt_declare_are_refused() {
    let values = BTreeMap::from([("colour".to_owned(), "blue".to_owned())]);

    let result = Setup::new(
        Provider::lima().unwrap(),
        name("box-1"),
        values,
        Path::new(KNOWN_HOSTS),
    );

    assert!(matches!(result, Err(ValuesError::Undeclared { .. })));
}

const HOST_KEY: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl";
const HOST_KEY_FINGERPRINT: &str = "SHA256:+DiY3wvvV6TuJJhbpZisF/zLDA0zPMSvHdkr4UvCOqU";
const UNKNOWN_KEY: &str = "Host key verification failed.\r\n";

/// A machine typed in as an SSH host (MP-10), checked against `known_hosts`.
fn ssh_machine(known_hosts: &Path) -> Setup {
    let target = SshTarget {
        host: "10.0.0.7".to_owned(),
        access: SshAccess::Direct {
            port: 22,
            user: "dev".to_owned(),
            identity_file: None,
        },
        host_key: None,
    };
    Setup::existing(Link::ssh(name("box-1"), target, known_hosts), known_hosts)
        .with_retry(RetryPolicy {
            attempts: 1,
            first_delay: Duration::ZERO,
            max_delay: Duration::ZERO,
        })
}

#[test]
fn a_host_key_from_the_identity_is_pinned_before_connecting() {
    let folder = tempfile::tempdir().unwrap();
    let known_hosts = folder.path().join("known_hosts");
    let runner = FakeRunner::new();
    runner.ok(Purpose::Create, "").ok(
        Purpose::Identity,
        &format!(
            r#"{{"name":"box-1","ssh":{{"host":"10.0.0.5","user":"dev","host_key":"{HOST_KEY}"}},"os":"linux","arch":"x86_64"}}"#
        ),
    );
    let mut setup = Setup::new(cloud(true), name("box-1"), BTreeMap::new(), &known_hosts).unwrap();

    setup.step(&runner, &FakeBuilds::files());
    setup.step(&runner, &FakeBuilds::files());

    assert_eq!(setup.current(), Step::Connect);
    let pinned = std::fs::read_to_string(&known_hosts).unwrap();
    assert_eq!(pinned.trim(), format!("humata2.box-1 {HOST_KEY}"));
}

#[test]
fn an_unknown_host_key_is_scanned_shown_and_trusted() {
    let folder = tempfile::tempdir().unwrap();
    let known_hosts = folder.path().join("known_hosts");
    let runner = FakeRunner::new();
    runner
        .answer(Purpose::Probe, Exit::Code(255), "", UNKNOWN_KEY)
        .answer(
            Purpose::ScanHostKey,
            Exit::Code(255),
            &format!("humata2.box-1 {HOST_KEY}\n"),
            "dev@10.0.0.7: Permission denied (publickey).\n",
        )
        .ok(Purpose::Probe, "Linux x86_64\n");
    let mut setup = ssh_machine(&known_hosts);

    let Progress::Failed(failure) = setup.step(&runner, &FakeBuilds::files()) else {
        panic!("an unknown key was accepted");
    };
    assert!(matches!(failure.error, SetupError::HostKeyUnknown { .. }));
    assert!(failure.remedies.contains(&Remedy::ConfirmHostKey));

    let key = setup.scan_host_key(&runner).unwrap();
    assert_eq!(key.fingerprint(), HOST_KEY_FINGERPRINT);
    // The scan offers the unverified machine nothing to log in with.
    let scan = &runner.ran(Purpose::ScanHostKey)[0];
    for refused in ["PubkeyAuthentication=no", "PasswordAuthentication=no", "KbdInteractiveAuthentication=no"] {
        assert!(scan.args.iter().any(|arg| arg == refused), "{refused} in {:?}", scan.args);
    }
    assert!(!scan.args.iter().any(|arg| arg.contains("known_hosts")));

    setup.trust_host_key(&key).unwrap();
    assert!(matches!(setup.step(&runner, &FakeBuilds::files()), Progress::Continue));
    assert_eq!(setup.current(), Step::Check);
    assert!(std::fs::read_to_string(&known_hosts).unwrap().contains(HOST_KEY));
}

#[test]
fn a_scan_that_learns_no_key_says_what_ssh_said() {
    let folder = tempfile::tempdir().unwrap();
    let runner = FakeRunner::new();
    runner.answer(
        Purpose::ScanHostKey,
        Exit::Code(255),
        "",
        "ssh: connect to host 10.0.0.7 port 22: Connection refused\n",
    );
    let setup = ssh_machine(&folder.path().join("known_hosts"));

    let failure = setup.scan_host_key(&runner).unwrap_err();

    assert!(matches!(failure.error, SetupError::HostKeyScanFailed { .. }));
    assert!(failure.output_tail.contains("Connection refused"));
}

#[test]
fn a_changed_host_key_shows_both_fingerprints_and_offers_no_login() {
    let folder = tempfile::tempdir().unwrap();
    let known_hosts = folder.path().join("known_hosts");
    let runner = FakeRunner::new();
    runner
        .answer(
            Purpose::ScanHostKey,
            Exit::Code(255),
            &format!("humata2.box-1 {HOST_KEY}\n"),
            "",
        )
        .answer(
            Purpose::Probe,
            Exit::Code(255),
            "",
            "@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@\r\n\
             @    WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED!     @\r\n\
             The fingerprint for the ED25519 key sent by the remote host is\r\n\
             SHA256:n3wK3yn3wK3yn3wK3yn3wK3yn3wK3yn3wK3yn3wK3yA.\r\n\
             Host key verification failed.\r\n",
        );
    let mut setup = ssh_machine(&known_hosts);
    let key = setup.scan_host_key(&runner).unwrap();
    setup.trust_host_key(&key).unwrap();

    let Progress::Failed(failure) = setup.step(&runner, &FakeBuilds::files()) else {
        panic!("a changed key was accepted");
    };

    let SetupError::HostKeyChanged {
        expected_fingerprint,
        presented_fingerprint,
        ..
    } = &failure.error
    else {
        panic!("{:?}", failure.error);
    };
    assert_eq!(expected_fingerprint.as_deref(), Some(HOST_KEY_FINGERPRINT));
    assert_eq!(
        presented_fingerprint.as_deref(),
        Some("SHA256:n3wK3yn3wK3yn3wK3yn3wK3yn3wK3yn3wK3yn3wK3yA")
    );
    assert!(failure.remedies.contains(&Remedy::ConfirmHostKey));
    assert!(!failure.remedies.contains(&Remedy::OpenInteractively));
}

#[test]
fn a_provider_connect_that_refuses_a_host_key_is_left_to_its_tool() {
    let runner = FakeRunner::new();
    runner
        .ok(Purpose::Create, "")
        .answer(Purpose::Probe, Exit::Code(255), "", UNKNOWN_KEY);
    let mut setup = lima("box-1");

    let (progress, _) = drive(&mut setup, &runner, &FakeBuilds::files());

    let Progress::Failed(failure) = progress else {
        panic!("{progress:?}");
    };
    assert!(matches!(failure.error, SetupError::ConnectHostKey { .. }));
    assert!(!failure.remedies.contains(&Remedy::ConfirmHostKey));
    let scan = setup.scan_host_key(&runner).unwrap_err();
    assert!(matches!(scan.error, SetupError::ConnectHostKey { .. }));
    assert!(runner.ran(Purpose::ScanHostKey).is_empty());
}

#[test]
fn without_a_user_manager_an_admin_command_or_setsid_is_offered() {
    let runner = FakeRunner::new();
    answer_up_to_service(&runner, &CheckFacts::default());
    runner.answer(Purpose::Service, Exit::Code(76), "", "no user manager");
    let mut setup = lima("box-1");

    let (progress, _) = drive(&mut setup, &runner, &FakeBuilds::files());

    let Progress::Failed(failure) = progress else {
        panic!("{progress:?}");
    };
    let SetupError::UserManagerUnavailable { ref admin_command, .. } = failure.error else {
        panic!("{:?}", failure.error);
    };
    assert!(failure
        .remedies
        .contains(&Remedy::RunAsAdmin { command: admin_command.clone() }));
    assert!(failure.remedies.contains(&Remedy::StartWithoutSystemd));
}
