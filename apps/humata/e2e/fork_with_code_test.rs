//! Session trees: a session edits files over two turns, and a fork before
//! the second has the files and the history as they were after the first.
//! Neither touches the repository or its history.

use std::path::Path;
use std::process::Command;

mod common;

const SCRIPT: &str = r#"{"rules": [
    {"when": "first", "steps": [
        {"step": "write_file", "path": "README.md", "contents": "first turn\n"},
        {"step": "write_file", "path": "notes/one.md", "contents": "one\n"},
        {"step": "say", "text": "Did the first."}
    ]},
    {"when": "second", "steps": [
        {"step": "write_file", "path": "README.md", "contents": "second turn\n"},
        {"step": "write_file", "path": "notes/two.md", "contents": "two\n"},
        {"step": "say", "text": "Did the second."}
    ]},
    {"when": "", "steps": [
        {"step": "write_file", "path": "notes/other.md", "contents": "other\n"},
        {"step": "say", "text": "Went another way."}
    ]}
]}"#;

fn read(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

fn head(path: &Path) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap_or_else(|error| panic!("{error}"));
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

/// The id of the event of `kind` whose detail is `detail`.
fn event_id(history: &str, kind: &str, detail: &str) -> String {
    history
        .lines()
        .find_map(|line| {
            let mut fields = line.splitn(3, '\t');
            let (id, found, said) = (fields.next()?, fields.next()?, fields.next()?);
            (found == kind && said == detail).then(|| id.to_owned())
        })
        .unwrap_or_else(|| panic!("no {kind} {detail:?} in {history}"))
}

#[test]
fn a_fork_has_the_files_and_history_as_they_were_where_it_starts() {
    let scratch = common::scratch();
    let root = scratch.path();
    let repository = root.join("repository");
    common::git_repository(&repository, &[("README.md", "hello\n")]);
    let started_at = head(&repository);
    let script = root.join("script.json");
    std::fs::write(&script, SCRIPT).unwrap_or_else(|error| panic!("{error}"));
    let (_daemon, humata) = common::start(root, &[]);

    let project = humata.run(&[
        "project",
        "add",
        "--name",
        "demo",
        "--repository",
        &repository.display().to_string(),
    ]);
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
    humata.run(&["send", session, "the first", "--wait"]);
    humata.run(&["send", session, "the second", "--wait"]);
    let original = humata.working_directory(session);
    assert_eq!(
        read(&original.join("README.md")).as_deref(),
        Some("second turn\n")
    );

    let history = humata.run(&["history", session]);
    let second = event_id(&history, "user", "the second");
    let fork = humata.run(&["fork", session, "--before", &second, "--name", "other way"]);
    let fork = fork.trim();
    let copy = humata.working_directory(fork);
    assert_ne!(copy, original);

    // The files stop where the history does.
    assert_eq!(
        humata.said(&["history", fork]),
        common::pairs(&[("user", "the first"), ("assistant", "Did the first.")])
    );
    assert_eq!(
        read(&copy.join("README.md")).as_deref(),
        Some("first turn\n")
    );
    assert_eq!(read(&copy.join("notes/one.md")).as_deref(), Some("one\n"));
    assert_eq!(read(&copy.join("notes/two.md")), None);

    // Each goes its own way from there.
    humata.run(&["send", fork, "something else", "--wait"]);
    assert_eq!(
        read(&copy.join("notes/other.md")).as_deref(),
        Some("other\n")
    );
    assert_eq!(read(&original.join("notes/other.md")), None);
    assert_eq!(
        read(&original.join("notes/two.md")).as_deref(),
        Some("two\n")
    );
    let changes = humata.run(&["changes", fork]);
    assert!(changes.contains("+other"), "{changes}");
    assert!(!changes.contains("+two"), "{changes}");

    // No commits: the repository and both copies are still on its commit.
    assert_eq!(head(&repository), started_at);
    assert_eq!(head(&original), started_at);
    assert_eq!(head(&copy), started_at);
    assert!(common::clean(&repository));

    // Archived, the fork's copy goes; unarchived, it comes back as it was.
    humata.run(&["archive", fork]);
    assert!(!copy.exists());
    humata.run(&["archive", fork, "--undo"]);
    humata.run(&["send", fork, "and again", "--wait"]);
    assert_eq!(
        read(&copy.join("notes/other.md")).as_deref(),
        Some("other\n")
    );
    assert_eq!(
        read(&copy.join("README.md")).as_deref(),
        Some("first turn\n")
    );
}
