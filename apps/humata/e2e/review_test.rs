//! Code review: a comment on a line the agent changed is sent as a review;
//! the scripted agent answers its thread and edits that line, and the thread
//! shows both, with the turn whose changes are the fix.

use std::time::{Duration, Instant};

mod common;

const SCRIPT: &str = r#"{"rules": [
    {"when": "my review", "steps": [
        {"step": "write_file", "path": "sum.py", "contents": "def total(a, b):\n    return a + b\n"},
        {"step": "say", "text": "Sure.\n\n[thread 1]\nRenamed it to total."}
    ]},
    {"when": "", "steps": [
        {"step": "write_file", "path": "sum.py", "contents": "def add(a, b):\n    return a + b\n"},
        {"step": "say", "text": "Added sum.py."}
    ]}
]}"#;

/// The comments printed under the one thread: (author, turn, text).
fn comments(threads: &str) -> Vec<(String, String, String)> {
    threads
        .lines()
        .filter_map(|line| {
            let mut fields = line.strip_prefix('\t')?.splitn(4, '\t');
            let (_, author, turn, text) = (
                fields.next()?,
                fields.next()?,
                fields.next()?,
                fields.next()?,
            );
            Some((author.to_owned(), turn.to_owned(), text.to_owned()))
        })
        .collect()
}

#[test]
fn a_comment_sent_as_a_review_gets_a_reply_and_a_fix() {
    let scratch = common::scratch();
    let root = scratch.path();
    let repository = root.join("app");
    common::git_repository(&repository, &[("README.md", "hello\n")]);
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
        "edits",
        "--backend-config",
        &script.display().to_string(),
    ]);
    let session = session.trim();
    humata.run(&["send", session, "add a sum", "--wait"]);
    let changes = humata.run(&["changes", session]);
    assert!(changes.contains("+def add(a, b):"), "{changes}");

    let thread = humata.run(&[
        "review",
        "comment",
        session,
        "app",
        "sum.py",
        "1",
        "call it total",
    ]);
    let thread = thread.trim().to_owned();
    assert_eq!(humata.run(&["review", "send", session]).trim(), "1");

    let deadline = Instant::now() + Duration::from_secs(20);
    let threads = loop {
        let threads = humata.run(&["review", "threads", session]);
        if comments(&threads).len() == 2 || Instant::now() > deadline {
            break threads;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    assert!(
        threads.starts_with(&format!("{thread}\tapp/sum.py:1-1\topen")),
        "{threads}"
    );
    let said = comments(&threads);
    assert_eq!(
        said,
        [
            ("you".to_owned(), "0".to_owned(), "call it total".to_owned()),
            (
                "agent".to_owned(),
                "2".to_owned(),
                "Renamed it to total.".to_owned()
            ),
        ],
        "{threads}"
    );
    let fix = humata.run(&["changes", session, "--turn", "2"]);
    assert!(
        fix.contains("-def add(a, b):") && fix.contains("+def total(a, b):"),
        "{fix}"
    );

    // Nothing left to send; the thread is resolved and the file staged.
    humata.fails(&["review", "send", session]);
    humata.run(&["review", "resolve", &thread]);
    humata.run(&["stage", session, "app", "sum.py"]);
    let staged = humata.run(&["changes", session, "--staged"]);
    assert!(staged.contains("+def total(a, b):"), "{staged}");
    assert_eq!(humata.run(&["changes", session, "--unstaged"]), "");
    assert!(common::clean(&repository));
}
