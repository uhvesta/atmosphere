//! Setting a machine up step by step against a fake runner: what each step
//! runs, how it retries, and what a failure offers.

mod support;

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use humata2_machines::{
    Exit, IdentityError, InstallProblem, InstalledFile, MachineName, Progress, Provider, Purpose,
    Remedy, RetryPolicy, ServiceKind, Setup, SetupError, Step, ValuesError,
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

fn answer_up_to_service(runner: &FakeRunner, facts: &CheckFacts) {
    runner
        .ok(Purpose::Create, "")
        .ok(Purpose::Probe, "Linux x86_64\n")
        .ok(Purpose::Check, &facts.stdout())
        .ok(Purpose::Install, "")
        .ok(Purpose::Install, "");
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
    let holder = payloads().pop().unwrap();
    let facts = CheckFacts {
        installed: vec![(InstalledFile::Holder, holder.sha256().to_owned())],
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
        .ok(Purpose::Install, "")
        .ok(Purpose::Install, "")
        .ok(Purpose::Service, "");
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
    assert_eq!(login.reason, humata2_machines::LoginReason::Prompted);
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
    assert!(matches!(failure.error, SetupError::CommandFailed { .. }));
    assert!(failure.output_tail.contains("quota exceeded"));
    assert!(failure.remedies.contains(&Remedy::FixProvider));
    assert!(failure.remedies.contains(&Remedy::DeleteWhatWasMade));
    assert_eq!(setup.current(), Step::Create);
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
    assert_eq!(runner.ran(Purpose::Install).len(), 2);
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
        .ok(Purpose::Install, "")
        .ok(Purpose::Install, "")
        .ok(Purpose::Service, "");
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
        (
            CheckFacts {
                systemd: false,
                cron: false,
                ..CheckFacts::default()
            },
            "service manager",
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
            "disk" => matches!(failure.error, SetupError::DiskLow { .. }),
            _ => matches!(failure.error, SetupError::NoServiceManager { .. }),
        };
        assert!(expected, "{case}: {:?}", failure.error);
    }
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
        matches!(progress, Progress::Failed(ref failure) if matches!(failure.error, SetupError::NoBuild { .. })),
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
