//! Moving a session between this Mac and a host over SSH while both answer:
//! it goes with every turn and its files, lists once, where it went, and
//! comes back the same way; one in a turn stays put.

use std::path::Path;
use std::process::Command;

use common::Humata;
use ssh_host::{daemon_with_host, eventually, sshd};

mod common;
mod ssh_host;

const SCRIPT: &str = r#"{"rules": [
    {"when": "hold", "steps": [{"step": "wait_for", "path": "never"}]},
    {"when": "", "steps": [{"step": "say", "text": "done"}]}
]}"#;

fn lines_of(humata: &Humata, session: &str) -> Vec<String> {
    humata
        .run(&["sessions"])
        .lines()
        .filter(|line| line.starts_with(session))
        .map(str::to_owned)
        .collect()
}

fn turns(humata: &Humata, session: &str) -> usize {
    humata
        .said(&["history", session])
        .iter()
        .filter(|(kind, _)| kind == "assistant")
        .count()
}

fn git(path: &Path, arguments: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(arguments)
        .status()
        .unwrap_or_else(|error| panic!("{error}"));
    assert!(status.success(), "git {arguments:?}");
}

#[test]
fn a_session_moves_to_a_host_and_back_with_every_turn() {
    let scratch = common::scratch();
    let root = scratch.path();
    let sshd = sshd(root);
    let code = root.join("code");
    common::git_repository(&code, &[("README.md", "hello\n")]);
    let origin = root.join("origin.git");
    git(
        root,
        &[
            "clone",
            "--quiet",
            "--bare",
            &code.display().to_string(),
            &origin.display().to_string(),
        ],
    );
    git(
        &code,
        &["remote", "add", "origin", &origin.display().to_string()],
    );
    let folder = root.join("far");
    let _host = ssh_host::DetachedHost(&folder);
    let (_daemon, humata) = daemon_with_host(root, &sshd, "far", &folder);
    let project = humata.run(&[
        "project",
        "add",
        "--name",
        "code",
        "--repository",
        &code.display().to_string(),
    ]);
    let script = root.join("script.json");
    std::fs::write(&script, SCRIPT).unwrap_or_else(|error| panic!("{error}"));
    let session = humata.run(&[
        "start",
        "--backend",
        "scripted",
        "--project",
        project.trim(),
        "--approval",
        "everything",
        "--backend-config",
        &script.display().to_string(),
    ]);
    let session = session.trim();

    // Two turns on this Mac, with a file between them.
    humata.run(&["send", session, "one", "--wait"]);
    std::fs::write(
        humata.working_directory(session).join("notes.txt"),
        "kept\n",
    )
    .unwrap_or_else(|error| panic!("{error}"));
    humata.run(&["send", session, "two", "--wait"]);

    // To the host: once, there, with both turns and the file.
    humata.run(&["recover", "session", session, "--host", "far"]);
    let lines = lines_of(&humata, session);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].contains("far"), "{lines:?}");
    assert_eq!(turns(&humata, session), 2);
    let there = humata.working_directory(session);
    assert!(there.starts_with(&folder), "{}", there.display());
    assert_eq!(
        std::fs::read_to_string(there.join("notes.txt"))
            .ok()
            .as_deref(),
        Some("kept\n")
    );

    // A turn there, then straight back: the turn comes too, though the
    // recovery copy may not have it yet.
    humata.run(&["send", session, "three", "--wait"]);
    humata.run(&["recover", "session", session, "--host", ""]);
    let lines = lines_of(&humata, session);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(!lines[0].contains("far"), "{lines:?}");
    assert_eq!(turns(&humata, session), 3);
    let here = humata.working_directory(session);
    assert!(!here.starts_with(&folder), "{}", here.display());
    assert!(here.join("notes.txt").is_file());

    // In a turn, it stays where it is.
    humata.run(&["send", session, "hold"]);
    eventually("the held turn working", || {
        lines_of(&humata, session)
            .first()
            .is_some_and(|line| line.contains("WORKING"))
    });
    humata.fails(&["recover", "session", session, "--host", "far"]);
    let lines = lines_of(&humata, session);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(!lines[0].contains("far"), "{lines:?}");
    humata.run(&["abort", session]);
}
