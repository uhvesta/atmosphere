//! `humata session …` (CLI.md §5.2): the requests each command sends.

mod support;

use humata2_cli::{exit, rpc};
use humata2_proto::humata::v1 as pb;
use support::{FakeConsole, FakeDaemon, run};

fn tab(id: &str, kind: pb::tab::Kind) -> pb::Tab {
    pb::Tab {
        id: Some(pb::TabId { value: id.into() }),
        kind: kind as i32,
        ..Default::default()
    }
}

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
async fn start_needs_a_machine_when_the_daemon_names_none_either() {
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
async fn attach_without_a_tab_attaches_when_the_session_has_an_agent_tab() {
    let daemon = FakeDaemon::new();
    daemon.reply(
        rpc::terminal::LIST_TABS,
        &pb::ListTabsResponse {
            tabs: vec![
                tab("tb_shell", pb::tab::Kind::Shell),
                tab("tb_agent", pb::tab::Kind::Agent),
            ],
        },
    );
    let console = FakeConsole::pipe();
    let exit = run(&daemon, &console, &["attach", "se_1"]).await;
    assert_eq!(exit, exit::OK);
    let looked_up: pb::ListTabsRequest = daemon.only(rpc::terminal::LIST_TABS);
    assert_eq!(looked_up.session, support::session("se_1"));
    assert!(daemon.called(rpc::terminal::ATTACH));
}

/// No agent tab and two shells: nothing to pick, so attach stops before it attaches.
#[tokio::test]
async fn attach_without_a_tab_stops_when_the_tab_is_ambiguous() {
    let daemon = FakeDaemon::new();
    daemon.reply(
        rpc::terminal::LIST_TABS,
        &pb::ListTabsResponse {
            tabs: vec![
                tab("tb_a", pb::tab::Kind::Shell),
                tab("tb_b", pb::tab::Kind::Shell),
            ],
        },
    );
    let console = FakeConsole::pipe();
    let exit = run(&daemon, &console, &["attach", "se_1"]).await;
    assert_eq!(exit, exit::PRECONDITION);
    assert!(!daemon.called(rpc::terminal::ATTACH));
}

/// `session tabs` prints the daemon's tab list as JSON, ids included.
#[tokio::test]
async fn session_tabs_prints_the_tabs() {
    let daemon = FakeDaemon::new();
    daemon.reply(
        rpc::terminal::LIST_TABS,
        &pb::ListTabsResponse {
            tabs: vec![
                tab("tb_agent", pb::tab::Kind::Agent),
                tab("tb_shell", pb::tab::Kind::Shell),
            ],
        },
    );
    let console = FakeConsole::pipe();
    let exit = run(&daemon, &console, &["--json", "session", "tabs", "se_1"]).await;
    assert_eq!(exit, exit::OK);
    let printed = console.json();
    assert_eq!(printed["result"]["tabs"][0]["id"], "tb_agent");
    assert_eq!(printed["result"]["tabs"][1]["id"], "tb_shell");
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

/// FK-4: a typed branch goes to the daemon as typed; none leaves it to the daemon.
#[tokio::test]
async fn fork_sends_a_typed_branch() {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    let exit = run(
        &daemon,
        &console,
        &["--json", "session", "fork", "se_1", "--kind", "code-only", "--branch", "me/try-b"],
    )
    .await;
    assert_eq!(exit, exit::OK);
    let sent: pb::ForkSessionRequest = daemon.only(rpc::session::FORK_SESSION);
    assert_eq!(sent.branch, "me/try-b");
}

/// The fork's agent and turn go to the daemon as given; none leaves them to it.
#[tokio::test]
async fn fork_sends_the_agent_and_the_turn() {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    let exit = run(
        &daemon,
        &console,
        &["--json", "session", "fork", "se_1", "--kind", "code-only", "--agent", "codex", "--before-turn", "3"],
    )
    .await;
    assert_eq!(exit, exit::OK);
    let sent: pb::ForkSessionRequest = daemon.only(rpc::session::FORK_SESSION);
    assert_eq!(sent.agent, Some(pb::AgentId { value: "codex".to_owned() }));
    assert_eq!(sent.before_turn, 3);
}

#[tokio::test]
async fn fork_without_an_agent_or_turn_leaves_both_to_the_daemon() {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    let exit = run(&daemon, &console, &["--json", "session", "fork", "se_1", "--kind", "code-only"]).await;
    assert_eq!(exit, exit::OK);
    let sent: pb::ForkSessionRequest = daemon.only(rpc::session::FORK_SESSION);
    assert_eq!(sent.agent, None);
    assert_eq!(sent.before_turn, 0);
}

/// The copy mode reads as `--copy` with short names, and `--kind` keeps working.
#[tokio::test]
async fn fork_copy_takes_each_mode() {
    for (arguments, expected) in [
        (["--copy", "code-and-context"], pb::ForkKind::CodeAndContext),
        (["--copy", "code"], pb::ForkKind::CodeOnly),
        (["--copy", "context"], pb::ForkKind::ContextOnly),
        (["--kind", "context-only"], pb::ForkKind::ContextOnly),
    ] {
        let daemon = FakeDaemon::new();
        let console = FakeConsole::pipe();
        let mut line = vec!["--json", "session", "fork", "se_1"];
        line.extend(arguments);
        assert_eq!(run(&daemon, &console, &line).await, exit::OK, "{arguments:?}");
        let sent: pb::ForkSessionRequest = daemon.only(rpc::session::FORK_SESSION);
        assert_eq!(sent.kind, expected as i32, "{arguments:?}");
    }
}

/// A turn before the second can't be forked before.
#[tokio::test]
async fn fork_before_turn_one_is_refused_and_nothing_forks() {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    let exit = run(
        &daemon,
        &console,
        &["--json", "session", "fork", "se_1", "--kind", "code-only", "--before-turn", "1"],
    )
    .await;
    assert_eq!(exit, exit::USAGE);
    assert!(!daemon.called(rpc::session::FORK_SESSION));
}

/// CV-8: the session, the target channel and the turn go to the daemon as given.
#[tokio::test]
async fn continue_sends_the_session_channel_and_turn() {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    let exit = run(
        &daemon,
        &console,
        &["--json", "--channel", "ch_2", "session", "continue", "se_1", "--before-turn", "4"],
    )
    .await;
    assert_eq!(exit, exit::OK);
    let sent: pb::ContinueInChannelRequest = daemon.only(rpc::session::CONTINUE_IN_CHANNEL);
    assert_eq!(sent.session, support::session("se_1"));
    assert_eq!(sent.channel, support::channel("ch_2"));
    assert_eq!(sent.before_turn, 4);
    assert!(sent.idempotency_key.is_some_and(|key| !key.value.is_empty()));
}

/// CV-8: no turn means the whole conversation, from now.
#[tokio::test]
async fn continue_without_a_turn_takes_the_whole_conversation() {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    let exit = run(&daemon, &console, &["--json", "session", "continue", "se_1", "--channel", "ch_2"]).await;
    assert_eq!(exit, exit::OK);
    let sent: pb::ContinueInChannelRequest = daemon.only(rpc::session::CONTINUE_IN_CHANNEL);
    assert_eq!(sent.before_turn, 0);
}

/// CV-8: without a target channel nothing is sent; there is no default.
#[tokio::test]
async fn continue_needs_a_channel() {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    let exit = run(&daemon, &console, &["--json", "session", "continue", "se_1"]).await;
    assert_eq!(exit, exit::USAGE);
    assert!(!daemon.called(rpc::session::CONTINUE_IN_CHANNEL));
}

/// CV-2: turn 1 is the first prompt, so nothing comes before it.
#[tokio::test]
async fn continue_before_the_first_turn_is_a_usage_error() {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    let exit = run(
        &daemon,
        &console,
        &["--json", "--channel", "ch_2", "session", "continue", "se_1", "--before-turn", "1"],
    )
    .await;
    assert_eq!(exit, exit::USAGE);
    assert!(!daemon.called(rpc::session::CONTINUE_IN_CHANNEL));
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
    assert_eq!(sent[0].text, "run the tests\n");
    assert_eq!(sent[1].when, pb::type_request::When::Now as i32);
}

async fn typed_text(args: &[&str]) -> String {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    let mut all = vec!["--json", "session", "type", "se_1"];
    all.extend_from_slice(args);
    assert_eq!(run(&daemon, &console, &all).await, exit::OK);
    daemon.only::<pb::TypeRequest>(rpc::terminal::TYPE).text
}

#[tokio::test]
async fn type_presses_enter_after_the_text_by_default() {
    assert_eq!(typed_text(&["run the tests"]).await, "run the tests\n");
    // Empty text is Enter alone.
    assert_eq!(typed_text(&[""]).await, "\n");
}

#[tokio::test]
async fn type_never_presses_enter_twice() {
    assert_eq!(typed_text(&["run the tests\n"]).await, "run the tests\n");
    assert_eq!(typed_text(&["run the tests\r"]).await, "run the tests\r");
}

#[tokio::test]
async fn type_with_no_submit_types_the_text_as_it_is() {
    assert_eq!(typed_text(&["run the tests", "--no-submit"]).await, "run the tests");
    assert_eq!(typed_text(&["a\n", "--no-submit"]).await, "a\n");
}

#[tokio::test]
async fn type_says_whether_the_daemon_submitted_it() {
    for (submitted, agent_busy) in [(true, false), (false, false), (true, true)] {
        let daemon = FakeDaemon::new();
        daemon.reply(
            rpc::terminal::TYPE,
            &pb::TypeResponse {
                typed: true,
                submitted,
                agent_busy,
                ..Default::default()
            },
        );
        let console = FakeConsole::pipe();
        let exit = run(&daemon, &console, &["--json", "session", "type", "se_1", "hi"]).await;
        assert_eq!(exit, exit::OK);
        let printed = console.json();
        assert_eq!(printed["result"]["typed"], true);
        assert_eq!(printed["result"]["submitted"], submitted);
        assert_eq!(printed["result"]["agent_busy"], agent_busy);
        assert!(printed["result"]["queued"].is_null());
    }
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
