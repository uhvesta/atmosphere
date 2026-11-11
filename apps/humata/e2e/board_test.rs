//! The board follows its sessions: a session on a subtask moves the subtask
//! and its task to In progress while it works, then to Waiting on you when
//! the agent asks for approval. The task's other subtask stays where it is.

use std::time::{Duration, Instant};

mod common;

const SCRIPT: &str = r#"{"rules": [
    {"when": "", "steps": [
        {"step": "wait_for", "path": "go"},
        {"step": "shell", "command": "make test", "output": "ok"},
        {"step": "say", "text": "Tested."}
    ]}
]}"#;

/// Each item's title and the status the board shows it in.
fn board(humata: &common::Humata, project: &str) -> Vec<(String, String)> {
    humata
        .run(&["item", "list", "--project", project])
        .lines()
        .map(|line| {
            let fields: Vec<&str> = line.split('\t').collect();
            let (Some(status), Some(title)) = (fields.get(2), fields.get(6)) else {
                panic!("{line}")
            };
            ((*title).to_owned(), (*status).to_owned())
        })
        .collect()
}

fn statuses(statuses: [&str; 3]) -> Vec<(String, String)> {
    ["login", "form", "copy"]
        .into_iter()
        .zip(statuses)
        .map(|(title, status)| (title.to_owned(), status.to_owned()))
        .collect()
}

/// Waits until the board shows `expected`.
fn until(humata: &common::Humata, project: &str, expected: &[(String, String)]) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let shown = board(humata, project);
        if shown == expected {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the board never showed {expected:?}; it shows {shown:?}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn a_session_on_a_subtask_moves_it_and_its_task() {
    let scratch = common::scratch();
    let root = scratch.path();
    let repository = root.join("repository");
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
    let project = project.trim();
    let task = humata.run(&["item", "add", "--project", project, "login"]);
    let task = task.trim();
    let subtask = humata.run(&[
        "item",
        "add",
        "--project",
        project,
        "--parent",
        task,
        "form",
    ]);
    let subtask = subtask.trim();
    humata.run(&[
        "item",
        "add",
        "--project",
        project,
        "--parent",
        task,
        "copy",
    ]);
    assert_eq!(board(&humata, project), statuses(["todo", "todo", "todo"]));

    let session = humata.run(&[
        "start",
        "--backend",
        "scripted",
        "--item",
        subtask,
        "--with-parents",
        "--backend-config",
        &script.display().to_string(),
    ]);
    let session = session.trim();
    humata.run(&["send", session, "test the form"]);
    until(
        &humata,
        project,
        &statuses(["in_progress", "in_progress", "todo"]),
    );

    let copy = humata.working_directory(session);
    std::fs::write(copy.join("go"), "").unwrap_or_else(|error| panic!("{error}"));
    until(
        &humata,
        project,
        &statuses(["waiting_on_you", "waiting_on_you", "todo"]),
    );
    let inbox = humata.run(&["inbox"]);
    let entries: Vec<Vec<&str>> = inbox
        .lines()
        .map(|line| line.split('\t').take(4).collect())
        .collect();
    assert_eq!(entries, [vec![session, "approval", "demo", "form"]]);
}
