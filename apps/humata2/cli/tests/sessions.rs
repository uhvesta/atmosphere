//! `humata session …` (CLI.md §5.2): the requests each command sends.

mod support;

use humata2_cli::{exit, rpc};
use humata2_proto::humata::v1 as pb;
use support::{FakeConsole, FakeDaemon, run};

fn repo(owner: &str, name: &str) -> pb::RepoRef {
    pb::RepoRef {
        owner: owner.into(),
        name: name.into(),
    }
}

#[tokio::test]
async fn start_sends_everything_it_was_given() {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    let exit = run(
        &daemon,
        &console,
        &[
            "--json", "--channel", "ch_1", "--machine", "ma_1", "session", "start", "--agent",
            "claude", "--model", "opus", "--repo", "acme/web", "--repo", "acme/api", "--title",
            "Fix login", "--message", "Start with the tests",
        ],
    )
    .await;
    assert_eq!(exit, exit::OK);
    let sent: pb::StartSessionRequest = daemon.only(rpc::session::START_SESSION);
    assert_eq!(sent.channel, support::channel("ch_1"));
    assert_eq!(sent.machine, support::machine("ma_1"));
    assert_eq!(sent.agent, Some(pb::AgentId { value: "claude".into() }));
    assert_eq!(sent.model, "opus");
    assert_eq!(sent.title, "Fix login");
    assert_eq!(sent.first_message, "Start with the tests");
    let repos: Vec<_> = sent.repos.iter().map(|choice| (choice.repo.clone(), choice.include)).collect();
    assert_eq!(
        repos,
        vec![(Some(repo("acme", "web")), true), (Some(repo("acme", "api")), true)]
    );
    assert!(sent.idempotency_key.is_some_and(|key| !key.value.is_empty()));
}

#[tokio::test]
async fn start_needs_a_machine() {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    let exit = run(&daemon, &console, &["--json", "--channel", "ch_1", "session", "start"]).await;
    assert_eq!(exit, exit::USAGE);
    assert!(!daemon.called(rpc::session::START_SESSION));
}

#[tokio::test]
async fn a_repo_must_be_owner_and_name() {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    let exit = run(
        &daemon,
        &console,
        &["--json", "--channel", "ch_1", "--machine", "ma_1", "session", "start", "--repo", "web"],
    )
    .await;
    assert_eq!(exit, exit::USAGE);
    assert!(!daemon.called(rpc::session::START_SESSION));
}

#[tokio::test]
async fn key_reuses_the_given_idempotency_key() {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    run(&daemon, &console, &["--json", "--key", "retry-1", "session", "stop", "se_1"]).await;
    let sent: pb::StopSessionRequest = daemon.only(rpc::session::STOP_SESSION);
    assert_eq!(sent.idempotency_key, Some(pb::IdempotencyKey { value: "retry-1".into() }));
}

#[tokio::test]
async fn a_session_is_found_by_its_title() {
    let daemon = FakeDaemon::new();
    daemon.reply(
        rpc::session::LIST_SESSIONS,
        &pb::ListSessionsResponse {
            sessions: vec![
                pb::Session {
                    id: support::session("se_1"),
                    title: "Fix login".into(),
                    ..Default::default()
                },
                pb::Session {
                    id: support::session("se_2"),
                    title: "Tidy docs".into(),
                    ..Default::default()
                },
            ],
            page: None,
        },
    );
    let console = FakeConsole::pipe();
    assert_eq!(run(&daemon, &console, &["--json", "session", "stop", "tidy docs"]).await, exit::OK);
    let sent: pb::StopSessionRequest = daemon.only(rpc::session::STOP_SESSION);
    assert_eq!(sent.id, support::session("se_2"));
}

#[tokio::test]
async fn fork_dry_run_previews_without_forking() {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    let exit = run(
        &daemon,
        &console,
        &["--json", "--dry-run", "session", "fork", "se_1", "--kind", "code-only"],
    )
    .await;
    assert_eq!(exit, exit::OK);
    assert_eq!(console.json()["outcome"], "planned");
    let previewed: pb::ForkSessionRequest = daemon.only(rpc::session::PREVIEW_FORK);
    assert_eq!(previewed.kind, pb::ForkKind::CodeOnly as i32);
    assert!(!daemon.called(rpc::session::FORK_SESSION));
}

#[tokio::test]
async fn fork_forks_the_parent() {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    let exit = run(
        &daemon,
        &console,
        &["--json", "session", "fork", "se_1", "--kind", "context-only", "--title", "Try B"],
    )
    .await;
    assert_eq!(exit, exit::OK);
    let sent: pb::ForkSessionRequest = daemon.only(rpc::session::FORK_SESSION);
    assert_eq!(sent.parent, support::session("se_1"));
    assert_eq!(sent.kind, pb::ForkKind::ContextOnly as i32);
    assert_eq!(sent.title, "Try B");
}

#[tokio::test]
async fn archive_push_first_archives_by_the_plan() {
    let daemon = FakeDaemon::new();
    daemon.reply(
        rpc::session::PLAN_ARCHIVE,
        &pb::ArchivePlan {
            token: Some(pb::PlanToken { value: "plan-a".into() }),
            ..Default::default()
        },
    );
    let console = FakeConsole::pipe();
    let exit = run(
        &daemon,
        &console,
        &["--json", "--yes", "session", "archive", "se_1", "--push-first"],
    )
    .await;
    assert_eq!(exit, exit::OK);
    let sent: pb::ArchiveSessionRequest = daemon.only(rpc::session::ARCHIVE_SESSION);
    assert_eq!(sent.choice, pb::ArchiveChoice::CommitPushPrThenArchive as i32);
    assert_eq!(sent.plan, Some(pb::PlanToken { value: "plan-a".into() }));
}

#[tokio::test]
async fn push_first_and_delete_work_cant_go_together() {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    let exit = run(
        &daemon,
        &console,
        &["--json", "session", "archive", "se_1", "--push-first", "--delete-work"],
    )
    .await;
    assert_eq!(exit, exit::USAGE);
    assert!(!daemon.called(rpc::session::PLAN_ARCHIVE));
}

#[tokio::test]
async fn push_without_repos_pushes_every_repo_the_plan_lists() {
    let plan = pb::PushPlan {
        id: support::session("se_1"),
        repos: vec![
            pb::push_plan::Repo {
                repo: Some(repo("acme", "web")),
                ..Default::default()
            },
            pb::push_plan::Repo {
                repo: Some(repo("acme", "api")),
                ..Default::default()
            },
        ],
        token: Some(pb::PlanToken { value: "plan-p".into() }),
    };
    let daemon = FakeDaemon::new();
    daemon.reply(rpc::session::PLAN_PUSH, &plan);
    daemon.reply(rpc::session::PLAN_PUSH, &plan);
    let console = FakeConsole::pipe();
    let exit = run(
        &daemon,
        &console,
        &["--json", "--yes", "session", "push", "se_1", "--pr", "--message", "Fix login"],
    )
    .await;
    assert_eq!(exit, exit::OK);
    let sent: pb::PushAndOpenPullRequestsRequest =
        daemon.only(rpc::session::PUSH_AND_OPEN_PULL_REQUESTS);
    assert_eq!(sent.plan, Some(pb::PlanToken { value: "plan-p".into() }));
    let pushed: Vec<_> = sent.repos.iter().map(|choice| choice.repo.clone()).collect();
    assert_eq!(pushed, vec![Some(repo("acme", "web")), Some(repo("acme", "api"))]);
    assert!(sent.repos.iter().all(|choice| choice.open_pull_request));
    assert!(sent.repos.iter().all(|choice| choice.commit_message == "Fix login"));
}

#[tokio::test]
async fn push_with_a_plan_pushes_the_named_repos_without_planning() {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    let exit = run(
        &daemon,
        &console,
        &["--json", "--plan", "plan-p", "session", "push", "se_1", "--repo", "acme/web"],
    )
    .await;
    assert_eq!(exit, exit::OK);
    assert!(!daemon.called(rpc::session::PLAN_PUSH));
    let sent: pb::PushAndOpenPullRequestsRequest =
        daemon.only(rpc::session::PUSH_AND_OPEN_PULL_REQUESTS);
    assert_eq!(sent.repos.len(), 1);
    assert_eq!(sent.repos[0].repo, Some(repo("acme", "web")));
    assert!(!sent.repos[0].open_pull_request);
}

#[tokio::test]
async fn type_at_turn_end_waits_for_the_turn() {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    run(
        &daemon,
        &console,
        &["--json", "session", "type", "se_1", "run the tests", "--at-turn-end"],
    )
    .await;
    run(&daemon, &console, &["--json", "session", "type", "se_1", "now please"]).await;
    let sent: Vec<pb::TypeRequest> = daemon.sent(rpc::terminal::TYPE);
    assert_eq!(sent[0].when, pb::type_request::When::TurnEnds as i32);
    assert_eq!(sent[0].text, "run the tests");
    assert_eq!(sent[1].when, pb::type_request::When::Now as i32);
}

#[tokio::test]
async fn ask_with_json_doesnt_hold_the_question_open() {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    let exit = run(
        &daemon,
        &console,
        &["--json", "session", "ask", "se_2", "Which port does the API use?"],
    )
    .await;
    assert_eq!(exit, exit::OK);
    let sent: pb::AskSessionRequest = daemon.only(rpc::session::ASK_SESSION);
    assert_eq!(sent.to, support::session("se_2"));
    assert_eq!(sent.question, "Which port does the API use?");
    assert_eq!(sent.timeout_seconds, 0);
}

#[tokio::test]
async fn resume_fresh_starts_over_with_a_summary() {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    run(&daemon, &console, &["--json", "session", "resume", "se_1", "--fresh"]).await;
    let sent: pb::ResumeSessionRequest = daemon.only(rpc::session::RESUME_SESSION);
    assert_eq!(
        sent.mode,
        pb::resume_session_request::Mode::StartFreshWithSummary as i32
    );
}
