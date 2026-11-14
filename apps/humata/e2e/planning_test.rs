//! Planning with an agent: a planning session proposes three tasks with
//! specs, the person applies two and declines one, and the board has
//! exactly those two. A fork of the planning session onto one of them gets
//! a copy of the code to work in. A planning session with the code reads a
//! copy of it, changes nothing there whatever it tries, and still proposes.

mod common;

const SCRIPT: &str = r#"{"rules": [
    {"when": "plan", "steps": [
        {"step": "tool", "tool": "mcp__humata__propose_task",
         "arguments": {"title": "Login form", "spec": "Email and password."}},
        {"step": "tool", "tool": "mcp__humata__propose_task",
         "arguments": {"title": "Dark mode", "spec": "Follow the system."}},
        {"step": "tool", "tool": "mcp__humata__propose_task",
         "arguments": {"title": "Confetti", "spec": "On every save."}},
        {"step": "say", "text": "Three tasks."}
    ]},
    {"when": "", "steps": [{"step": "say", "text": "On it."}]}
]}"#;

/// Each proposal's id, title and state, as `humata proposal list` prints.
fn proposals(humata: &common::Humata, session: &str) -> Vec<(String, String, String)> {
    humata
        .run(&["proposal", "list", session])
        .lines()
        .map(|line| {
            let fields: Vec<&str> = line.split('\t').collect();
            let (Some(id), Some(state), Some(title)) =
                (fields.first(), fields.get(2), fields.get(3))
            else {
                panic!("{line}")
            };
            ((*id).to_owned(), (*title).to_owned(), (*state).to_owned())
        })
        .collect()
}

fn titles(humata: &common::Humata, project: &str) -> Vec<String> {
    humata
        .run(&["item", "list", "--project", project])
        .lines()
        .map(|line| {
            let fields: Vec<&str> = line.split('\t').collect();
            let Some(title) = fields.get(6) else {
                panic!("{line}")
            };
            (*title).to_owned()
        })
        .collect()
}

#[test]
fn applied_proposals_become_tasks_and_a_fork_works_on_one() {
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
        "--planning",
        "--backend-config",
        &script.display().to_string(),
    ]);
    let session = session.trim();
    humata.run(&["send", session, "plan the app", "--wait"]);

    let proposed = proposals(&humata, session);
    let shown: Vec<(&str, &str)> = proposed
        .iter()
        .map(|(_, title, state)| (title.as_str(), state.as_str()))
        .collect();
    assert_eq!(
        shown,
        [
            ("Login form", "pending"),
            ("Dark mode", "pending"),
            ("Confetti", "pending")
        ]
    );
    let [login, dark, confetti] = [0, 1, 2].map(|index| proposed[index].0.clone());

    let task = humata.run(&["proposal", "apply", session, &login]);
    let task = task.trim();
    humata.run(&["proposal", "apply", session, &dark, "--title", "Dark theme"]);
    humata.run(&["proposal", "decline", session, &confetti]);
    humata.fails(&["proposal", "apply", session, &confetti]);
    humata.fails(&["proposal", "apply", session, &login]);

    assert_eq!(titles(&humata, project), ["Login form", "Dark theme"]);
    let states: Vec<String> = proposals(&humata, session)
        .into_iter()
        .map(|(_, _, state)| state)
        .collect();
    assert_eq!(states, ["applied", "applied", "declined"]);

    let fork = humata.run(&["fork", session, "--item", task, "--name", "login"]);
    let fork = fork.trim();
    humata.run(&["send", fork, "build it", "--wait"]);
    let copy = humata.working_directory(fork);
    assert_eq!(
        std::fs::read_to_string(copy.join("README.md"))
            .ok()
            .as_deref(),
        Some("hello\n")
    );
    assert_ne!(copy, repository);
}

const WRITING_SCRIPT: &str = r#"{"rules": [
    {"when": "", "steps": [
        {"step": "write_file", "path": "README.md", "contents": "changed"},
        {"step": "write_file", "path": "new.txt", "contents": "new"},
        {"step": "tool", "tool": "mcp__humata__propose_task",
         "arguments": {"title": "Login form", "spec": "Email and password."}},
        {"step": "say", "text": "One task."}
    ]}
]}"#;

#[test]
fn a_planning_session_with_the_code_reads_a_copy_it_cant_change_and_its_proposal_becomes_a_task() {
    let scratch = common::scratch();
    let root = scratch.path();
    let repository = root.join("repository");
    common::git_repository(&repository, &[("README.md", "hello\n")]);
    let script = root.join("script.json");
    std::fs::write(&script, WRITING_SCRIPT).unwrap_or_else(|error| panic!("{error}"));
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
        "--planning",
        "--with-code",
        "--approval",
        "everything",
        "--backend-config",
        &script.display().to_string(),
    ]);
    let session = session.trim();
    humata.run(&["send", session, "plan the app", "--wait"]);

    let copy = humata.working_directory(session);
    assert_ne!(copy, repository);
    assert_eq!(
        std::fs::read_to_string(copy.join("README.md"))
            .ok()
            .as_deref(),
        Some("hello\n")
    );
    assert!(!copy.join("new.txt").exists());

    let proposed = proposals(&humata, session);
    let [(login, _, _)] = proposed.as_slice() else {
        panic!("one proposal: {proposed:?}")
    };
    humata.run(&["proposal", "apply", session, login]);
    assert_eq!(titles(&humata, project), ["Login form"]);
}
