//! A scripted agent edits a file in its copy of a project's repository: it
//! waits for a person to approve the write, then, once the session allows
//! edits, writes without asking but still asks to run a command. The
//! repository itself is left as it was.

use std::path::Path;

mod common;

const SCRIPT: &str = r#"{"rules": [
    {"when": "notes", "steps": [
        {"step": "think", "text": "The notes go in the repository."},
        {"step": "write_file", "path": "NOTES.md", "contents": "Ship M1.\n"},
        {"step": "say", "text": "Wrote the notes."}
    ]},
    {"when": "", "steps": [
        {"step": "write_file", "path": "TODO.md", "contents": "Test it.\n"},
        {"step": "shell", "command": "git commit -am todo", "output": "committed"},
        {"step": "say", "text": "Done."}
    ]}
]}"#;

/// The (kind, detail) of each event line `humata` printed.
fn events(output: &str) -> Vec<(String, String)> {
    output
        .lines()
        .filter_map(|line| {
            let mut fields = line.splitn(3, '\t');
            let (_, kind, detail) = (fields.next()?, fields.next()?, fields.next()?);
            Some((kind.to_owned(), detail.to_owned()))
        })
        .collect()
}

/// The request the output stopped at, and its permission kind: the last
/// one asked, with no answer and no idle after it. Other events, such as a
/// save of the copy, may still follow it.
fn waiting_on(events: &[(String, String)]) -> (String, String) {
    let at = events
        .iter()
        .rposition(|(kind, _)| kind == "permission_requested")
        .unwrap_or_else(|| panic!("no request: {events:?}"));
    let mut fields = events[at].1.split(' ');
    let request = fields.next().unwrap_or_default().to_owned();
    let permission = fields.next().unwrap_or_default().to_owned();
    assert!(
        !events[at..].iter().any(|(kind, detail)| kind == "idle"
            || (kind == "permission_completed" && detail.starts_with(&request))),
        "{events:?}"
    );
    (request, permission)
}

fn read(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

#[test]
fn an_agent_edits_a_file_through_an_approval() {
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
    let session = humata.run(&[
        "start",
        "--backend",
        "scripted",
        "--project",
        project,
        "--backend-config",
        &script.display().to_string(),
    ]);
    let session = session.trim();
    let copy = humata.working_directory(session);
    assert_ne!(copy, repository);

    // Asked every time: the write waits for a person.
    let asked = events(&humata.run(&["send", session, "write the notes", "--wait"]));
    let (request, permission) = waiting_on(&asked);
    assert_eq!(permission, "PERMISSION_KIND_WRITE");
    assert!(asked.iter().any(|(kind, _)| kind == "reasoning"));
    assert_eq!(read(&copy.join("NOTES.md")), None);

    let approved = humata.run(&["approve", session, &request, "--wait"]);
    assert_eq!(read(&copy.join("NOTES.md")).as_deref(), Some("Ship M1.\n"));
    assert!(events(&approved).contains(&("assistant".to_owned(), "Wrote the notes.".to_owned())));
    assert_eq!(
        events(&approved).last().map(|(kind, _)| kind.as_str()),
        Some("idle")
    );

    // Allowing edits: the write goes ahead, the command still asks.
    humata.run(&["mode", session, "edits"]);
    let asked = events(&humata.run(&["send", session, "and the todo", "--wait"]));
    assert_eq!(read(&copy.join("TODO.md")).as_deref(), Some("Test it.\n"));
    assert!(
        asked
            .iter()
            .any(|(kind, detail)| kind == "permission_completed" && detail.ends_with(" automatic")),
        "{asked:?}"
    );
    let (request, permission) = waiting_on(&asked);
    assert_eq!(permission, "PERMISSION_KIND_SHELL");

    let denied = events(&humata.run(&["approve", session, &request, "--deny", "--wait"]));
    assert!(
        denied
            .iter()
            .any(|(kind, detail)| kind == "tool_completed" && detail.contains(" failed "))
    );
    assert_eq!(denied.last().map(|(kind, _)| kind.as_str()), Some("idle"));

    let listed = humata.run(&["sessions", "--project", project]);
    let fields: Vec<&str> = listed
        .lines()
        .flat_map(|line| line.split('\t').take(1))
        .collect();
    assert_eq!(fields, vec![session]);
    let changes = humata.run(&["changes", session]);
    assert!(changes.contains("+Ship M1."), "{changes}");
    assert!(changes.contains("+Test it."), "{changes}");
    assert!(common::clean(&repository));
    assert_eq!(read(&repository.join("NOTES.md")), None);
    assert_eq!(
        humata.said(&["history", session]),
        common::pairs(&[
            ("user", "write the notes"),
            ("assistant", "Wrote the notes."),
            ("user", "and the todo"),
            ("assistant", "Done."),
        ])
    );
}
