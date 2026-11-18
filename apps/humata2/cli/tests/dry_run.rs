//! `--dry-run` on a command that changes something (CLI.md §4): it prints
//! the plan and sends no change to the daemon.

mod support;

use humata2_cli::{Environment, Rpc, exit, rpc};
use humata2_proto::humata::v1 as pb;
use support::{FakeConsole, FakeDaemon, run, run_as};

/// Runs `words` with `--dry-run` and says what went wrong, if anything: the
/// command changed something on the daemon, failed, or printed no plan.
async fn dry_run_failure(words: &[&str], change: Rpc) -> Option<String> {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    let mut args = vec!["--json", "--dry-run"];
    args.extend_from_slice(words);
    let code = run(&daemon, &console, &args).await;
    if daemon.called(change) {
        return Some(format!("{words:?} sent {}", change.path));
    }
    if code != exit::OK {
        return Some(format!("{words:?} exited {code}"));
    }
    if console.json()["outcome"] != "planned" {
        return Some(format!("{words:?} printed no plan"));
    }
    None
}

/// Every case that fails, so one run lists every broken command.
async fn failures(cases: &[(&[&str], Rpc)]) -> Vec<String> {
    let mut found = Vec::new();
    for (words, change) in cases {
        if let Some(failure) = dry_run_failure(words, *change).await {
            found.push(failure);
        }
    }
    found
}

fn assert_no_failures(found: Vec<String>) {
    assert!(found.is_empty(), "dry runs that changed something:\n{}", found.join("\n"));
}

const INBOX: &[(&[&str], Rpc)] = &[
    (&["inbox", "approve", "rq_1"], rpc::inbox::ANSWER_REQUEST),
    (&["inbox", "deny", "rq_1"], rpc::inbox::ANSWER_REQUEST),
    (&["inbox", "reply", "rq_1", "hi"], rpc::inbox::ANSWER_REQUEST),
];

const ACCESS: &[(&[&str], Rpc)] = &[
    (&["--channel", "ch_1", "access", "away", "wait", "--kinds", "channel_repo_add"], rpc::access::SET_AWAY_RULE),
    (&["access", "revoke", "gr_1"], rpc::access::REVOKE_GRANT),
    (&["access", "passphrase", "--generate"], rpc::access::SET_APPROVAL_PASSPHRASE),
    (
        &["access", "ask-ahead", "channel_repo_add", "session:se_1", "--reason", "why"],
        rpc::inbox::REQUEST_ACCESS,
    ),
];

const TOKENS: &[(&[&str], Rpc)] = &[
    (
        &["token", "create", "--name", "ci", "--capability", "channel_repo_add"],
        rpc::cli_auth::CREATE_SCRIPT_TOKEN,
    ),
    (&["token", "revoke", "tok_1"], rpc::cli_auth::REVOKE_SCRIPT_TOKEN),
];

const BOARDS: &[(&[&str], Rpc)] = &[
    (&["--channel", "ch_1", "board", "add", "Title"], rpc::board::CREATE_ITEM),
    (&["--channel", "ch_1", "board", "edit", "bi_1", "--title", "x"], rpc::board::UPDATE_ITEM),
    (&["--channel", "ch_1", "board", "move", "bi_1", "done"], rpc::board::MOVE_ITEM),
    (&["--channel", "ch_1", "board", "rm", "bi_1"], rpc::board::DELETE_ITEM),
    (&["view", "open", "vw_1"], rpc::view::SET_VIEW_STATE),
    (&["view", "later", "vw_1"], rpc::view::SET_VIEW_STATE),
    (&["view", "not-now", "vw_1"], rpc::view::SET_VIEW_STATE),
    (&["view", "revert", "vw_1"], rpc::view::SET_VIEW_STATE),
    (&["view", "keep", "vw_1"], rpc::view::SET_VIEW_STATE),
    (&["diff", "mark-reviewed", "se_1"], rpc::diff::MARK_REVIEWED),
    (&["comment", "add", "se_1", "acme/repo:src/a.rs:1-2", "hi"], rpc::comment::CREATE_COMMENT),
    (&["comment", "send", "se_1", "cm_1"], rpc::comment::SEND_COMMENTS),
    (&["comment", "rm", "cm_1"], rpc::comment::DROP_COMMENT),
];

const CHANNELS: &[(&[&str], Rpc)] = &[
    (&["channel", "create", "name"], rpc::channel::CREATE_CHANNEL),
    (&["channel", "rename", "ch_1", "new"], rpc::channel::RENAME_CHANNEL),
    (&["channel", "machine", "add", "ch_1", "ma_1"], rpc::channel::ADD_MACHINE),
    (
        &["channel", "settings", "ch_1", "set", "branch_prefix", "x"],
        rpc::settings::UPDATE_CHANNEL_SETTINGS,
    ),
];

const OPERATIONS: &[(&[&str], Rpc)] = &[
    (&["ops", "cancel", "op_1"], rpc::operation::CANCEL_OPERATION),
    (&["ops", "retry", "op_1"], rpc::operation::RETRY_OPERATION),
];

const SESSIONS: &[(&[&str], Rpc)] = &[
    (&["--channel", "ch_1", "session", "start", "--machine", "ma_1"], rpc::session::START_SESSION),
    (&["session", "send-now", "se_1", "q_1"], rpc::terminal::SEND_QUEUED_NOW),
    (&["session", "rename", "se_1", "title"], rpc::session::RENAME_SESSION),
    (&["session", "stop", "se_1"], rpc::session::STOP_SESSION),
    (&["session", "resume", "se_1"], rpc::session::RESUME_SESSION),
    (&["session", "restore", "se_1"], rpc::session::RESTORE_SESSION),
    (&["session", "rebuild", "se_1"], rpc::session::REBUILD_SESSION),
    (&["session", "repo", "add", "se_1", "acme/repo"], rpc::session::ADD_REPO_TO_SESSION),
    (&["session", "own-copy", "se_1"], rpc::session::GIVE_OWN_COPY),
    (&["--channel", "ch_1", "session", "continue", "se_1"], rpc::session::CONTINUE_IN_CHANNEL),
    (&["session", "note", "se_1", "hi"], rpc::session::SEND_NOTE),
    (&["session", "ask", "se_1", "hi"], rpc::session::ASK_SESSION),
    (&["session", "answer", "qu_1", "8080"], rpc::session::ANSWER_QUESTION),
    (&["session", "type", "se_1", "hi"], rpc::terminal::TYPE),
    (&["shell", "ma_1"], rpc::terminal::OPEN_MACHINE_SHELL),
];

const MACHINES: &[(&[&str], Rpc)] = &[
    (&["host", "add", "box.example"], rpc::machine::ADD_SSH_MACHINE),
    (&["host", "reconnect", "ma_1"], rpc::machine::RECONNECT_MACHINE),
    (&["host", "vm-tier", "ma_1", "on"], rpc::machine::SET_VM_TIER),
    (&["host", "clear-build-cache", "se_1", "target"], rpc::machine::CLEAR_BUILD_CACHE),
    (&["host", "trim-logs", "ma_1"], rpc::machine::TRIM_LOGS),
    (&["host", "stop", "ma_1"], rpc::machine::STOP_MACHINE_SESSIONS),
    (&["host", "start", "ma_1"], rpc::machine::START_MACHINE_SESSIONS),
    (&["host", "var", "rm", "ma_1", "NAME"], rpc::machine::REMOVE_HOST_VARIABLE),
    (
        &["host", "trust-key", "ma_1", "--fingerprint", "SHA256:abc"],
        rpc::machine::TRUST_NEW_HOST_KEY,
    ),
    (&["host", "script", "run", "rule", "ma_1"], rpc::machine::RUN_MACHINE_SCRIPT),
    (&["provider", "create", "kind", "name"], rpc::provider::CREATE_MACHINE),
    (&["provider", "start", "ma_1"], rpc::provider::RUN_PROVIDER_ACTION),
    (&["provider", "stop", "ma_1"], rpc::provider::RUN_PROVIDER_ACTION),
    (&["bridge", "close", "fw_1"], rpc::bridge::CLOSE_PORT_FORWARD),
    (&["agents", "add", "claude", "--launch", "x"], rpc::agents::ADD_AGENT),
    (&["pin", "session:se_1"], rpc::marks::SET_MARKS),
    (&["unpin", "session:se_1"], rpc::marks::SET_MARKS),
    (&["pinned", "reorder", "session:se_1"], rpc::marks::REORDER_PINNED),
];

const SETTINGS_AND_REPOS: &[(&[&str], Rpc)] = &[
    (&["settings", "set", "branch_prefix", "x"], rpc::settings::UPDATE_SETTINGS),
    (&["settings", "set", "security.contain_agents", "on"], rpc::settings::UPDATE_SETTINGS),
    (&["repos", "remove", "acme/repo"], rpc::repos::REMOVE_FROM_REPO_LIBRARY),
    (&["repos", "scope", "set", "--owner", "acme"], rpc::repos::UPDATE_REPO_SCOPE),
    (&["desk", "keep", "se_1"], rpc::desk::KEEP_DESK_FOLDER),
];

#[tokio::test]
async fn inbox_answers_plan_without_answering() {
    assert_no_failures(failures(INBOX).await);
}

#[tokio::test]
async fn approve_all_plans_without_answering_the_waiting_requests() {
    let daemon = FakeDaemon::new();
    daemon.reply(
        rpc::inbox::LIST_REQUESTS,
        &pb::ListRequestsResponse {
            requests: vec![pb::InboxRequest {
                id: Some(pb::RequestId { value: "rq_1".into() }),
                state: pb::RequestState::Pending as i32,
                ..Default::default()
            }],
            ..Default::default()
        },
    );
    let console = FakeConsole::pipe();
    let code = run(&daemon, &console, &["--json", "--dry-run", "inbox", "approve-all"]).await;
    assert_eq!(code, exit::OK);
    assert_eq!(console.json()["outcome"], "planned");
    assert!(!daemon.called(rpc::inbox::ANSWER_ALL));
}

#[tokio::test]
async fn access_changes_plan_without_changing_the_rules() {
    assert_no_failures(failures(ACCESS).await);
}

#[tokio::test]
async fn access_set_dry_run_changes_no_rule() {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    let code = run(
        &daemon,
        &console,
        &["--json", "--dry-run", "--channel", "ch_1", "access", "set", "channel_repo_add", "allow"],
    )
    .await;
    assert_eq!(code, exit::OK);
    assert_eq!(console.json()["outcome"], "planned");
    assert!(!daemon.called(rpc::access::SET_CHANNEL_RULE));
}

#[tokio::test]
async fn tokens_plan_without_creating_or_revoking() {
    assert_no_failures(failures(TOKENS).await);
}

#[tokio::test]
async fn review_start_dry_run_starts_no_review() {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    let code = run(
        &daemon,
        &console,
        &["--json", "--dry-run", "--channel", "ch_1", "--machine", "ma_1", "review", "start", "acme/repo#1"],
    )
    .await;
    assert_eq!(code, exit::OK);
    assert_eq!(console.json()["outcome"], "planned");
    assert!(!daemon.called(rpc::review::START_REVIEW));
}

#[tokio::test]
async fn review_allow_push_plans_without_changing_it() {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    let code = run(&daemon, &console, &["--json", "--dry-run", "review", "allow-push", "se_1", "on"]).await;
    assert_eq!(code, exit::OK);
    assert_eq!(console.json()["outcome"], "planned");
    assert!(!daemon.called(rpc::review::SET_PUSH_ALLOWED));
}

#[tokio::test]
async fn boards_and_comments_plan_without_changing_them() {
    assert_no_failures(failures(BOARDS).await);
}

#[tokio::test]
async fn channel_repo_add_dry_run_adds_nothing() {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    let code = run(
        &daemon,
        &console,
        &["--json", "--dry-run", "channel", "repo", "add", "ch_1", "acme/repo"],
    )
    .await;
    assert_eq!(code, exit::OK);
    assert_eq!(console.json()["outcome"], "planned");
    assert!(!daemon.called(rpc::channel::ADD_REPO));
}

#[tokio::test]
async fn channels_plan_without_changing_them() {
    assert_no_failures(failures(CHANNELS).await);
}

#[tokio::test]
async fn operations_plan_without_cancelling_or_retrying() {
    assert_no_failures(failures(OPERATIONS).await);
}

#[tokio::test]
async fn sessions_plan_without_changing_them() {
    assert_no_failures(failures(SESSIONS).await);
}

#[tokio::test]
async fn session_move_dry_run_moves_nothing() {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    let code = run(
        &daemon,
        &console,
        &["--json", "--dry-run", "session", "move", "se_1", "--to", "ma_1"],
    )
    .await;
    assert_eq!(code, exit::OK);
    assert_eq!(console.json()["outcome"], "planned");
    assert!(!daemon.called(rpc::session::MOVE_SESSION));
}

#[tokio::test]
async fn machines_and_providers_plan_without_changing_them() {
    assert_no_failures(failures(MACHINES).await);
}

#[tokio::test]
async fn settings_and_repos_plan_without_changing_them() {
    assert_no_failures(failures(SETTINGS_AND_REPOS).await);
}

#[tokio::test]
async fn host_var_set_dry_run_sets_nothing() {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe().with_stdin(b"value");
    let code = run(&daemon, &console, &["--json", "--dry-run", "host", "var", "set", "ma_1", "NAME"]).await;
    assert_eq!(code, exit::OK);
    assert_eq!(console.json()["outcome"], "planned");
    assert!(!daemon.called(rpc::machine::SET_HOST_VARIABLE));
}

#[tokio::test]
async fn agents_edit_dry_run_changes_no_agent() {
    let daemon = FakeDaemon::new();
    daemon.reply(
        rpc::agents::LIST_AGENTS,
        &pb::ListAgentsResponse {
            agents: vec![pb::AgentDefinition {
                name: "claude".into(),
                ..Default::default()
            }],
        },
    );
    let console = FakeConsole::pipe();
    let code = run(
        &daemon,
        &console,
        &["--json", "--dry-run", "agents", "edit", "claude", "--launch", "x"],
    )
    .await;
    assert_eq!(code, exit::OK);
    assert_eq!(console.json()["outcome"], "planned");
    assert!(!daemon.called(rpc::agents::UPDATE_AGENT));
}

const FILES: &[(&[&str], Rpc)] = &[
    (&["--channel", "ch_1", "files", "mkdir", "ma_1:shared/new"], rpc::files::CREATE_FOLDER),
    (&["--channel", "ch_1", "files", "mv", "ma_1:shared/a", "b"], rpc::files::RENAME),
    (&["--channel", "ch_1", "files", "rm", "ma_1:shared/a"], rpc::files::MOVE_TO_TRASH),
    (&["--channel", "ch_1", "cp", "ma_1:shared/a", "ma_1:shared/b"], rpc::transfer::START_TRANSFER),
    (&["--channel", "ch_1", "mv", "ma_1:shared/a", "ma_1:shared/b"], rpc::transfer::START_TRANSFER),
    (&["transfer", "pause", "tr_1"], rpc::transfer::PAUSE_TRANSFER),
    (&["transfer", "resume", "tr_1"], rpc::transfer::RESUME_TRANSFER),
    (&["transfer", "cancel", "tr_1"], rpc::transfer::CANCEL_TRANSFER),
];

#[tokio::test]
async fn files_and_transfers_plan_without_moving_anything() {
    assert_no_failures(failures(FILES).await);
}

#[tokio::test]
async fn cp_to_a_machine_dry_run_starts_no_transfer() {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    let code = run(
        &daemon,
        &console,
        &["--json", "--dry-run", "--channel", "ch_1", "cp", "ma_1:shared/a", "ma_2"],
    )
    .await;
    assert_eq!(code, exit::OK);
    assert_eq!(console.json()["outcome"], "planned");
    assert!(!daemon.called(rpc::transfer::START_TRANSFER));
}

#[tokio::test]
async fn files_put_dry_run_uploads_nothing() {
    let local = tempfile::NamedTempFile::new().expect("a temp file");
    std::fs::write(local.path(), b"hello").expect("writes the temp file");
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    let path = local.path().display().to_string();
    let code = run(
        &daemon,
        &console,
        &["--json", "--dry-run", "--channel", "ch_1", "files", "put", &path, "ma_1:shared/a"],
    )
    .await;
    assert_eq!(code, exit::OK);
    assert_eq!(console.json()["outcome"], "planned");
    assert!(!daemon.called(rpc::files::WRITE_FILE));
}

/// A restart report that `humata doctor restarts` can read.
const RESTART_REPORT: &str = r#"{
  "format": 1,
  "at_unix_ms": 1763287200000,
  "machine": "devbox",
  "version": "0.0.0",
  "os": "macOS 26.7",
  "store_schema_version": 3,
  "sessions": [
    {
      "kind": "agent", "session": "se_1", "tab": "tb_1", "agent": "claude", "was": "running",
      "status": "recoverable", "reason": "conversation_file_missing",
      "needs": "its conversation file is gone", "step": "check it can be resumed",
      "conversation_id": "c1",
      "files": [{"role": "conversation", "path": "/x/c1.jsonl", "bytes": null, "modified_unix_ms": null}]
    }
  ],
  "daemon_log": ["humatad2: starting", "humatad2: serving"]
}"#;

#[tokio::test]
async fn doctor_restarts_dry_run_writes_no_diagnosis() {
    let state = tempfile::tempdir().expect("a temp dir");
    let folder = state.path().join("restarts").join("2026-11-16-restart-at-10-00-00-utc");
    std::fs::create_dir_all(&folder).expect("the report folder");
    std::fs::write(folder.join("restart.json"), RESTART_REPORT).expect("the report");
    let written = tempfile::NamedTempFile::new().expect("a temp file");
    std::fs::write(written.path(), b"what happened").expect("the diagnosis text");
    let diagnosis = written.path().display().to_string();
    let state_path = state.path().display().to_string();

    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    let code = run(
        &daemon,
        &console,
        &["--json", "--dry-run", "doctor", "restarts", "--state", &state_path, "--diagnosis", &diagnosis],
    )
    .await;
    assert_eq!(code, exit::OK);
    assert_eq!(console.json()["outcome"], "planned");
    assert!(!folder.join("diagnosis.md").exists());
    assert!(!daemon.called(rpc::daemon::REPAIR_RESTART));
}

#[tokio::test]
async fn login_dry_run_asks_for_no_login_and_keeps_none() {
    let home = tempfile::tempdir().expect("a temp dir");
    let environment = Environment {
        home: Some(home.path().to_path_buf()),
        terminal_session: Some(7),
        ..Environment::default()
    };
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    let code = run_as(&environment, &daemon, &console, &["--json", "--dry-run", "login"]).await;
    assert_eq!(code, exit::OK);
    assert_eq!(console.json()["outcome"], "planned");
    assert!(!daemon.called(rpc::cli_auth::BEGIN_CLI_SESSION));
    assert_eq!(environment.with_saved_login().login, None);
}

#[tokio::test]
async fn logout_dry_run_ends_no_login_and_forgets_no_token() {
    let home = tempfile::tempdir().expect("a temp dir");
    let terminal = || Environment {
        home: Some(home.path().to_path_buf()),
        terminal_session: Some(7),
        ..Environment::default()
    };
    let granted = pb::CliSession {
        id: "cl_1".into(),
        token: "0123456789abcdef".into(),
        ..Default::default()
    };
    let logging_in = FakeDaemon::new();
    logging_in.reply(rpc::cli_auth::BEGIN_CLI_SESSION, &granted);
    let code = run_as(&terminal(), &logging_in, &FakeConsole::pipe(), &["--json", "login"]).await;
    assert_eq!(code, exit::OK);

    let environment = terminal().with_saved_login();
    assert!(environment.login.is_some());
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    let code = run_as(&environment, &daemon, &console, &["--json", "--dry-run", "logout"]).await;
    assert_eq!(code, exit::OK);
    assert_eq!(console.json()["outcome"], "planned");
    assert!(!daemon.called(rpc::cli_auth::END_CLI_SESSION));
    assert!(terminal().with_saved_login().login.is_some());
}

#[tokio::test]
async fn provider_login_dry_run_opens_no_stream() {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    let code = run(&daemon, &console, &["--dry-run", "provider", "login", "ma_1"]).await;
    assert_eq!(code, exit::OK);
    assert!(!daemon.called(rpc::provider::INTERACTIVE_LOGIN));
}

#[tokio::test]
async fn attach_dry_run_is_a_usage_refusal_and_opens_no_stream() {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    let code = run(&daemon, &console, &["--dry-run", "attach", "se_1"]).await;
    assert_eq!(code, exit::USAGE);
    assert!(!daemon.called(rpc::terminal::ATTACH));
}

#[tokio::test]
async fn hook_dry_run_is_a_usage_refusal_and_sends_nothing() {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe().with_stdin(br#"{"tool":"bash"}"#);
    let code = run(&daemon, &console, &["--dry-run", "hook", "claude", "permission"]).await;
    assert_eq!(code, exit::USAGE);
    assert!(!daemon.called(rpc::session::HOOK));
}
