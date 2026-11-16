//! `humata doctor restarts` (PER-30, D242) over report folders on disk and a
//! fake daemon: what it reads, what it repairs and what it writes.

mod support;

use std::path::{Path, PathBuf};

use humata2_cli::{exit, rpc};
use humata2_proto::humata::v1 as pb;
use pb::repair_restart_request::Target;
use serde_json::Value as Json;
use support::{FakeConsole, FakeDaemon, run};

const NEWEST: &str = "2026-11-16-restart-at-10-00-00-utc";
const OLDER: &str = "2026-11-15-restart-at-09-00-00-utc";

const RESTART: &str = r#"{
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
    },
    {"kind": "shell", "session": null, "tab": "tb_2", "agent": "shell", "was": "shell", "status": "restarted", "files": []}
  ],
  "daemon_log": ["humatad2: starting", "humatad2: serving"]
}"#;

const OPERATIONS: &str = concat!(
    r#"{"id":"op_1","kind":"start_session","step":"clone","state":{"pending":{"waits_for":"git"}}}"#,
    "\n",
    r#"{"id":"op_2","kind":"fork","step":"copy","state":"finished"}"#,
    "\n",
    r#"{"id":"op_1","kind":"start_session","step":"clone","state":{"stuck":{"reason":"the folder is gone"}}}"#,
    "\n",
    // Cut short by a power cut.
    r#"{"id":"op_3","kind":"mo"#,
);

/// A state folder with an older report and a newer one; the newer has
/// operations when `operations` says so.
fn state(operations: bool) -> tempfile::TempDir {
    let state = tempfile::tempdir().unwrap();
    for name in [OLDER, NEWEST] {
        let folder = state.path().join("restarts").join(name);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("restart.json"), RESTART).unwrap();
    }
    if operations {
        let folder = state.path().join("restarts").join(NEWEST);
        std::fs::write(folder.join("operations.jsonl"), OPERATIONS).unwrap();
    }
    state
}

fn newest(state: &Path) -> PathBuf {
    state.join("restarts").join(NEWEST)
}

async fn doctor(daemon: &FakeDaemon, state: &Path, extra: &[&str]) -> (i32, FakeConsole) {
    let console = FakeConsole::pipe();
    let state = state.display().to_string();
    let mut words = vec!["--json", "doctor", "restarts", "--state", state.as_str()];
    words.extend_from_slice(extra);
    let exit = run(daemon, &console, &words).await;
    (exit, console)
}

fn ids(list: &Json) -> Vec<&str> {
    list.as_array()
        .unwrap()
        .iter()
        .map(|item| item["id"].as_str().unwrap())
        .collect()
}

#[tokio::test]
async fn explain_reads_the_newest_report_and_says_what_wasnt_brought_back() {
    let state = state(true);
    let daemon = FakeDaemon::new();

    let (code, console) = doctor(&daemon, state.path(), &["--explain"]).await;

    assert_eq!(code, exit::OK, "{}", console.stdout());
    let result = &console.json()["result"];
    assert_eq!(result["report"], newest(state.path()).display().to_string());
    assert_eq!(result["brought_back"], 1);
    let missing = result["not_brought_back"].as_array().unwrap();
    assert_eq!(missing.len(), 1);
    assert_eq!(missing[0]["session"], "se_1");
    assert_eq!(missing[0]["reason"], "conversation_file_missing");
}

#[tokio::test]
async fn explain_lists_the_latest_state_of_each_operation_and_skips_a_torn_line() {
    let state = state(true);
    let daemon = FakeDaemon::new();

    let (_, console) = doctor(&daemon, state.path(), &["--explain"]).await;

    let result = &console.json()["result"];
    assert_eq!(ids(&result["operations"]), ["op_1", "op_2"]);
    assert_eq!(result["operations"][0]["state"], "stuck");
    assert_eq!(result["operations"][0]["detail"], "the folder is gone");
    assert_eq!(result["operations"][1]["state"], "finished");
    assert_eq!(ids(&result["stuck"]), ["op_1"]);
}

#[tokio::test]
async fn a_report_with_no_operations_file_is_still_read() {
    let state = state(false);
    let daemon = FakeDaemon::new();

    let (code, console) = doctor(&daemon, state.path(), &["--explain"]).await;

    assert_eq!(code, exit::OK);
    assert_eq!(console.json()["result"]["operations"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn explain_offers_a_safe_repair_and_a_lossy_one_and_changes_nothing() {
    let state = state(true);
    let daemon = FakeDaemon::new();

    let (_, console) = doctor(&daemon, state.path(), &["--explain"]).await;

    let repairs = &console.json()["result"]["repairs"];
    assert_eq!(ids(repairs), ["resume:se_1", "start-fresh:se_1"]);
    assert_eq!(repairs[0]["safe"], true);
    assert_eq!(repairs[1]["safe"], false);
    assert!(!daemon.called(rpc::daemon::REPAIR_RESTART));
    assert!(!daemon.called(rpc::session::RESUME_SESSION));
}

#[tokio::test]
async fn explain_writes_diagnosis_md_in_the_report_folder_it_read() {
    let state = state(true);
    let daemon = FakeDaemon::new();

    doctor(&daemon, state.path(), &["--explain"]).await;

    let written = std::fs::read_to_string(newest(state.path()).join("diagnosis.md")).unwrap();
    assert!(written.contains("se_1"));
    assert!(written.contains("op_1"));
    assert!(!state.path().join("restarts").join(OLDER).join("diagnosis.md").exists());
}

#[tokio::test]
async fn an_older_report_can_be_named() {
    let state = state(false);
    let daemon = FakeDaemon::new();

    let (_, console) = doctor(&daemon, state.path(), &["--explain", OLDER]).await;

    let result = &console.json()["result"];
    assert_eq!(result["report"], state.path().join("restarts").join(OLDER).display().to_string());
}

#[tokio::test]
async fn the_safe_repair_runs_and_the_lossy_one_waits_for_allow() {
    let state = state(true);
    let daemon = FakeDaemon::new();

    let (code, console) = doctor(&daemon, state.path(), &[]).await;

    assert_eq!(code, exit::OK, "{}", console.stdout());
    let sent: pb::RepairRestartRequest = daemon.only(rpc::daemon::REPAIR_RESTART);
    assert_eq!(
        sent.target,
        Some(Target::Session(pb::SessionId {
            value: "se_1".into()
        }))
    );
    assert!(!daemon.called(rpc::session::RESUME_SESSION));
    let applied = &console.json()["result"]["applied"];
    assert_eq!(ids(applied), ["resume:se_1"]);
    assert_eq!(applied[0]["ok"], true);
}

#[tokio::test]
async fn a_lossy_repair_runs_only_when_it_is_allowed() {
    let state = state(true);
    let daemon = FakeDaemon::new();

    let (code, _) = doctor(&daemon, state.path(), &["--allow", "start-fresh:se_1"]).await;

    assert_eq!(code, exit::OK);
    let sent: pb::ResumeSessionRequest = daemon.only(rpc::session::RESUME_SESSION);
    assert_eq!(sent.id.unwrap().value, "se_1");
    assert_eq!(
        sent.mode,
        pb::resume_session_request::Mode::StartFreshWithSummary as i32
    );
}

#[tokio::test]
async fn allowing_a_repair_the_report_doesnt_offer_is_a_usage_error_and_runs_nothing() {
    let state = state(true);
    let daemon = FakeDaemon::new();

    let (code, _) = doctor(&daemon, state.path(), &["--allow", "start-fresh:se_9"]).await;

    assert_eq!(code, exit::USAGE);
    assert!(!daemon.called(rpc::daemon::REPAIR_RESTART));
    assert!(!daemon.called(rpc::session::RESUME_SESSION));
}

#[tokio::test]
async fn a_repair_the_daemon_refuses_is_reported_and_the_others_still_run() {
    let state = state(true);
    let daemon = FakeDaemon::new();
    daemon.status(rpc::daemon::REPAIR_RESTART, 9);

    let (code, console) = doctor(&daemon, state.path(), &["--allow", "start-fresh:se_1"]).await;

    assert_eq!(code, exit::OK);
    let applied = &console.json()["result"]["applied"];
    assert_eq!(applied[0]["id"], "resume:se_1");
    assert_eq!(applied[0]["ok"], false);
    assert_eq!(applied[1]["id"], "start-fresh:se_1");
    assert_eq!(applied[1]["ok"], true);
}

#[tokio::test]
async fn a_diagnosis_from_stdin_leads_diagnosis_md() {
    let state = state(true);
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe().with_stdin(b"The conversation file was deleted by hand.\n");
    let folder = state.path().display().to_string();

    let code = run(
        &daemon,
        &console,
        &["--json", "doctor", "restarts", "--explain", "--state", &folder, "--diagnosis", "-"],
    )
    .await;

    assert_eq!(code, exit::OK);
    let written = std::fs::read_to_string(newest(state.path()).join("diagnosis.md")).unwrap();
    let said = written.find("deleted by hand").unwrap();
    assert!(said < written.find("se_1").unwrap());
}

#[tokio::test]
async fn a_state_folder_with_no_reports_is_not_found() {
    let state = tempfile::tempdir().unwrap();
    let daemon = FakeDaemon::new();

    let (code, _) = doctor(&daemon, state.path(), &["--explain"]).await;

    assert_eq!(code, exit::NOT_FOUND);
}
