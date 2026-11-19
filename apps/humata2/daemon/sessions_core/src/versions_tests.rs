//! The CLI's version is asked again when it may have changed, and a kind
//! that wraps its CLI asks the CLI itself (CV-3).

use std::path::{Path, PathBuf};

use humata2_revive::CliVersions;

use crate::Agents;

/// `testdata/fake_cli.sh` linked beside a `version` file the test changes.
///
/// A link, not a copy: a copy is written by this process, and while any
/// test thread's spawn holds a forked child that inherited the copy's
/// write descriptor, running the copy directly fails with "text file busy"
/// (ETXTBSY) and the CLI reports no version. The script finds the
/// `version` file from the path it was run by, so the link still reads it.
fn fake_cli_in(dir: &Path) -> PathBuf {
    let from = std::env::var_os("HUMATA2_FAKE_CLI")
        .map(PathBuf::from)
        .expect("set HUMATA2_FAKE_CLI to the sessions testdata/fake_cli.sh");
    let from = std::fs::canonicalize(from).unwrap();
    let script = dir.join("fake_cli.sh");
    std::os::unix::fs::symlink(from, &script).unwrap();
    script
}

fn agents(kind: serde_json::Value) -> Agents {
    Agents::from_json(serde_json::json!([kind]).to_string().as_bytes()).unwrap()
}

#[tokio::test]
async fn the_version_asked_for_follows_the_cli_when_it_is_upgraded() {
    let dir = tempfile::tempdir().unwrap();
    let cli = fake_cli_in(dir.path());
    let agents = agents(serde_json::json!({
        "name": "v",
        "program": cli,
        "start_args": [],
        "resume_args": [],
        "conversation_file": "x",
        "conversation_format": "codex-rollout",
    }));
    std::fs::write(dir.path().join("version"), "codex-cli 1.0.0\n").unwrap();
    assert_eq!(agents.current("v").await.as_deref(), Some("codex-cli 1.0.0"));
    std::fs::write(dir.path().join("version"), "codex-cli 2.0.0\n").unwrap();
    assert_eq!(agents.current("v").await.as_deref(), Some("codex-cli 2.0.0"));
}

#[tokio::test]
async fn an_agent_nobody_knows_has_no_version() {
    let dir = tempfile::tempdir().unwrap();
    let cli = fake_cli_in(dir.path());
    let agents = agents(serde_json::json!({
        "name": "v", "program": cli, "start_args": [], "resume_args": [],
        "conversation_file": "x", "conversation_format": "codex-rollout",
    }));
    assert_eq!(agents.current("other").await, None);
}

#[tokio::test]
async fn a_kind_that_wraps_its_cli_records_the_cli_version() {
    let dir = tempfile::tempdir().unwrap();
    let cli = fake_cli_in(dir.path());
    let cli = cli.to_str().unwrap();
    // `program wrapper-args... --flags {placeholders}`, the way a kind runs
    // its CLI through a shell, `env` or `npx`.
    for (program, leading) in [("/bin/sh", vec![cli]), ("/usr/bin/env", vec!["/bin/sh", cli])] {
        let mut start: Vec<&str> = leading.clone();
        start.extend(["--session-id", "{conversation}"]);
        let mut resume: Vec<&str> = leading;
        resume.extend(["--resume", "{conversation}"]);
        let agents = agents(serde_json::json!({
            "name": "w",
            "program": program,
            "start_args": start,
            "resume_args": resume,
            "conversation_file": "x",
            "conversation_format": "copilot-session-state",
        }));
        assert_eq!(
            agents.current("w").await.as_deref(),
            Some("GitHub Copilot CLI 1.0.112-0."),
            "{program}"
        );
    }
}

#[tokio::test]
async fn arguments_the_start_and_resume_do_not_share_are_not_the_wrappers() {
    let dir = tempfile::tempdir().unwrap();
    let cli = fake_cli_in(dir.path());
    // Run directly, the CLI's first arguments differ between a start and a
    // resume, so nothing is put in front of the version arguments.
    let agents = agents(serde_json::json!({
        "name": "d",
        "program": cli,
        "start_args": ["new", "{conversation}"],
        "resume_args": ["resume", "{conversation}"],
        "conversation_file": "x",
        "conversation_format": "codex-rollout",
    }));
    assert_eq!(agents.current("d").await.as_deref(), Some("codex-cli 0.169.0"));
}

#[tokio::test]
async fn the_agents_file_overrides_the_version_command_of_a_wrapped_kind() {
    let dir = tempfile::tempdir().unwrap();
    let cli = fake_cli_in(dir.path());
    let cli = cli.to_str().unwrap();
    let agents = agents(serde_json::json!({
        "name": "o",
        "program": "/bin/sh",
        "start_args": [cli, "--session-id", "{conversation}"],
        "resume_args": [cli, "--resume", "{conversation}"],
        "conversation_file": "x",
        "conversation_format": "codex-rollout",
        "version_args": [cli, "--no-auto-update", "--version"],
    }));
    assert_eq!(agents.current("o").await.as_deref(), Some("GitHub Copilot CLI 1.0.112-0."));
}
