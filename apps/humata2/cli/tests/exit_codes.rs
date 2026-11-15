//! Exit codes and `error.kind` (CLI.md §7, D26): what a script branches on.

mod support;

use humata2_cli::{exit, rpc};
use humata2_proto::humata::v1 as pb;
use pb::error::Kind;
use support::{FakeConsole, FakeDaemon, run, run_interrupted};

/// What CLI.md §7 and D26 give each kind. Every kind is listed, with no
/// catch-all, so a kind added to the proto fails to compile here until
/// the spec's code for it is written down.
fn expected(kind: &Kind) -> (&'static str, i32) {
    match kind {
        Kind::NotFound(_) => ("not_found", 3),
        Kind::MachineOffline(_) => ("machine_offline", 8),
        Kind::HostUnreachable(_) => ("host_unreachable", 8),
        Kind::Denied(_) => ("denied", 4),
        Kind::UserOnly(_) => ("user_only", 5),
        Kind::PresenceRequired(_) => ("presence_required", 5),
        Kind::IdentityMismatch(_) => ("identity_mismatch", 5),
        Kind::PeerUnproven(_) => ("peer_unproven", 4),
        Kind::RevisionConflict(_) => ("revision_conflict", 6),
        Kind::PlanStale(_) => ("plan_stale", 6),
        Kind::NameTaken(_) => ("name_taken", 6),
        Kind::InvalidField(_) => ("invalid_field", 2),
        Kind::UnpushedWork(_) => ("unpushed_work", 7),
        Kind::SessionBusy(_) => ("session_busy", 7),
        Kind::ParentReadOnly(_) => ("parent_read_only", 4),
        Kind::MainCloneReadOnly(_) => ("main_clone_read_only", 4),
        Kind::OutsideHumataFolders(_) => ("outside_humata_folders", 2),
        Kind::DiskFull(_) => ("disk_full", 10),
        Kind::RateLimited(_) => ("rate_limited", 10),
        Kind::GithubCli(_) => ("github_cli", 9),
        Kind::GitFailed(_) => ("git_failed", 9),
        Kind::PushRefused(_) => ("push_refused", 4),
        Kind::SshFailed(_) => ("ssh_failed", 8),
        Kind::HostKeyChanged(_) => ("host_key_changed", 7),
        Kind::ChecksumMismatch(_) => ("checksum_mismatch", 11),
        Kind::ResumeFailed(_) => ("resume_failed", 7),
        Kind::AgentLacksFeature(_) => ("agent_lacks_feature", 7),
        Kind::ProviderStepFailed(_) => ("provider_step_failed", 9),
        Kind::ProviderIdentityInvalid(_) => ("provider_identity_invalid", 7),
        Kind::LoginNeedsUser(_) => ("login_needs_user", 7),
        Kind::PeerVersionUnsupported(_) => ("peer_version_unsupported", 7),
        Kind::OffsetTrimmed(_) => ("offset_trimmed", 11),
        Kind::NotCancellable(_) => ("not_cancellable", 7),
        Kind::AlreadyAnswered(_) => ("already_answered", 6),
        Kind::RequestWithdrawn(_) => ("request_withdrawn", 7),
        Kind::UserAway(_) => ("user_away", 7),
        Kind::NotInChannel(_) => ("not_in_channel", 4),
        Kind::PrClosed(_) => ("pr_closed", 7),
        Kind::ReviewPartlyPosted(_) => ("review_partly_posted", 9),
        Kind::SkillNameClash(_) => ("skill_name_clash", 7),
        Kind::LingerUnavailable(_) => ("linger_unavailable", 7),
        Kind::DaemonAlreadyRunning(_) => ("daemon_already_running", 7),
        Kind::DaemonNotRunning(_) => ("daemon_not_running", 7),
        Kind::JournalEntryRejected(_) => ("journal_entry_rejected", 7),
        Kind::RepoNotVisible(_) => ("repo_not_visible", 7),
        Kind::StepFailed(_) => ("step_failed", 9),
        Kind::PassphraseLocked(_) => ("passphrase_locked", 10),
        Kind::Internal(_) => ("internal", 70),
    }
}

fn every_kind() -> Vec<Kind> {
    vec![
        Kind::NotFound(Default::default()),
        Kind::MachineOffline(Default::default()),
        Kind::HostUnreachable(Default::default()),
        Kind::Denied(Default::default()),
        Kind::UserOnly(Default::default()),
        Kind::PresenceRequired(Default::default()),
        Kind::IdentityMismatch(Default::default()),
        Kind::PeerUnproven(Default::default()),
        Kind::RevisionConflict(Default::default()),
        Kind::PlanStale(Default::default()),
        Kind::NameTaken(Default::default()),
        Kind::InvalidField(Default::default()),
        Kind::UnpushedWork(Default::default()),
        Kind::SessionBusy(Default::default()),
        Kind::ParentReadOnly(Default::default()),
        Kind::MainCloneReadOnly(Default::default()),
        Kind::OutsideHumataFolders(Default::default()),
        Kind::DiskFull(Default::default()),
        Kind::RateLimited(Default::default()),
        Kind::GithubCli(Default::default()),
        Kind::GitFailed(Default::default()),
        Kind::PushRefused(Default::default()),
        Kind::SshFailed(Default::default()),
        Kind::HostKeyChanged(Default::default()),
        Kind::ChecksumMismatch(Default::default()),
        Kind::ResumeFailed(Default::default()),
        Kind::AgentLacksFeature(Default::default()),
        Kind::ProviderStepFailed(Default::default()),
        Kind::ProviderIdentityInvalid(Default::default()),
        Kind::LoginNeedsUser(Default::default()),
        Kind::PeerVersionUnsupported(Default::default()),
        Kind::OffsetTrimmed(Default::default()),
        Kind::NotCancellable(Default::default()),
        Kind::AlreadyAnswered(Default::default()),
        Kind::RequestWithdrawn(Default::default()),
        Kind::UserAway(Default::default()),
        Kind::NotInChannel(Default::default()),
        Kind::PrClosed(Default::default()),
        Kind::ReviewPartlyPosted(Default::default()),
        Kind::SkillNameClash(Default::default()),
        Kind::LingerUnavailable(Default::default()),
        Kind::DaemonAlreadyRunning(Default::default()),
        Kind::DaemonNotRunning(Default::default()),
        Kind::JournalEntryRejected(Default::default()),
        Kind::RepoNotVisible(Default::default()),
        Kind::StepFailed(Default::default()),
        Kind::PassphraseLocked(Default::default()),
        Kind::Internal(Default::default()),
    ]
}

#[tokio::test]
async fn each_error_kind_exits_with_its_code_and_name() {
    for kind in every_kind() {
        let (name, code) = expected(&kind);
        // WhoAmI carries no presence proof, so even presence_required is
        // reported rather than answered with a proof.
        let daemon = FakeDaemon::new();
        daemon.refuse(rpc::daemon::WHO_AM_I, kind);
        let console = FakeConsole::pipe();
        let exit = run(&daemon, &console, &["--json", "whoami"]).await;
        assert_eq!(exit, code, "exit code for {name}");
        let printed = console.json();
        assert_eq!(printed["ok"], false, "{name}");
        assert_eq!(printed["error"]["kind"], name);
    }
}

#[tokio::test]
async fn each_error_kind_exits_with_its_code_without_json_too() {
    for kind in every_kind() {
        let (name, code) = expected(&kind);
        let daemon = FakeDaemon::new();
        daemon.refuse(rpc::daemon::WHO_AM_I, kind);
        let console = FakeConsole::pipe();
        assert_eq!(run(&daemon, &console, &["whoami"]).await, code, "{name}");
        assert!(console.stdout().is_empty(), "{name}: errors go to stderr");
        assert!(!console.stderr().is_empty(), "{name}");
    }
}

#[tokio::test]
async fn a_status_without_a_humata_error_exits_by_its_grpc_code() {
    let cases = [
        // UNIMPLEMENTED: a service this humatad hasn't built yet.
        (12, "unimplemented", exit::PRECONDITION),
        // UNAVAILABLE: the daemon went away mid-call.
        (14, "unavailable", exit::UNREACHABLE),
        (16, "unauthenticated", exit::NEEDS_USER),
        (13, "internal", exit::INTERNAL),
    ];
    for (code, name, exit) in cases {
        let daemon = FakeDaemon::new();
        daemon.status(rpc::daemon::WHO_AM_I, code);
        let console = FakeConsole::pipe();
        assert_eq!(run(&daemon, &console, &["--json", "whoami"]).await, exit, "{name}");
        assert_eq!(console.json()["error"]["kind"], name);
    }
}

#[tokio::test]
async fn a_bad_command_line_is_a_usage_error() {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    assert_eq!(run(&daemon, &console, &["inbox", "approve"]).await, exit::USAGE);
    assert!(!console.stderr().is_empty());
    assert!(!daemon.called(rpc::daemon::WHO_AM_I));
}

#[tokio::test]
async fn a_bad_command_line_with_json_is_an_error_object() {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    assert_eq!(run(&daemon, &console, &["--json", "no-such-command"]).await, exit::USAGE);
    let printed = console.json();
    assert_eq!(printed["ok"], false);
    assert_eq!(printed["error"]["kind"], "invalid_field");
}

#[tokio::test]
async fn help_exits_zero_on_stdout() {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    assert_eq!(run(&daemon, &console, &["--help"]).await, exit::OK);
    assert!(!console.stdout().is_empty());
}

#[tokio::test]
async fn a_name_that_matches_nothing_is_not_found() {
    let daemon = FakeDaemon::new();
    daemon.reply(
        rpc::session::LIST_SESSIONS,
        &pb::ListSessionsResponse {
            sessions: vec![pb::Session {
                id: support::session("se_1"),
                title: "fix login".into(),
                ..Default::default()
            }],
            ..Default::default()
        },
    );
    let console = FakeConsole::pipe();
    let exit = run(&daemon, &console, &["--json", "session", "stop", "tidy docs"]).await;
    assert_eq!(exit, exit::NOT_FOUND);
    assert_eq!(console.json()["error"]["kind"], "not_found");
    assert!(!daemon.called(rpc::session::STOP_SESSION));
}

#[tokio::test]
async fn a_name_that_matches_two_is_a_usage_error_listing_both() {
    let daemon = FakeDaemon::new();
    let named = |id: &str| pb::Session {
        id: support::session(id),
        title: "Fix login".into(),
        ..Default::default()
    };
    daemon.reply(
        rpc::session::LIST_SESSIONS,
        &pb::ListSessionsResponse {
            sessions: vec![named("se_1"), named("se_2")],
            ..Default::default()
        },
    );
    let console = FakeConsole::pipe();
    let exit = run(&daemon, &console, &["--json", "session", "stop", "fix login"]).await;
    assert_eq!(exit, exit::USAGE);
    let matches = &console.json()["error"]["details"]["matches"];
    assert_eq!(matches.as_array().map(Vec::len), Some(2));
}

#[tokio::test]
async fn ctrl_c_while_waiting_exits_130() {
    let daemon = FakeDaemon::new();
    daemon.hang(rpc::daemon::WHO_AM_I);
    let console = FakeConsole::pipe();
    let exit = run_interrupted(&daemon, &console, &["--json", "whoami"]).await;
    assert_eq!(exit, exit::INTERRUPTED);
    assert_eq!(console.json()["error"]["kind"], "interrupted");
}
