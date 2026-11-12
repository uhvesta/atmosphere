//! Agents ask before they change the board: a session working on a task
//! proposes a subtask and a new title and spec for its task, and the board
//! is unchanged until the person applies them. The board's journal then
//! shows both as the session's, and reverting the edit keeps a change made
//! since to another field.

mod common;

/// The rule that has the agent propose, under `task`.
fn script(task: &str) -> String {
    format!(
        r#"{{"rules": [
    {{"when": "go", "steps": [
        {{"step": "tool", "tool": "mcp__humata__propose_task",
         "arguments": {{"title": "Form", "spec": "Its fields.", "parent_id": "{task}"}}}},
        {{"step": "tool", "tool": "mcp__humata__propose_spec_edit",
         "arguments": {{"item_id": "{task}", "title": "Sign in", "spec": "With email."}}}},
        {{"step": "say", "text": "Proposed."}}
    ]}},
    {{"when": "", "steps": [{{"step": "say", "text": "On it."}}]}}
]}}"#
    )
}

/// Each item's title and priority, as `humata item list` prints them.
fn items(humata: &common::Humata, project: &str) -> Vec<(String, String)> {
    humata
        .run(&["item", "list", "--project", project])
        .lines()
        .map(|line| {
            let fields: Vec<&str> = line.split('\t').collect();
            let (Some(priority), Some(title)) = (fields.get(4), fields.get(6)) else {
                panic!("{line}")
            };
            ((*title).to_owned(), (*priority).to_owned())
        })
        .collect()
}

/// Each board change's id, item, before and author, newest first.
fn journal(humata: &common::Humata, project: &str) -> Vec<[String; 4]> {
    humata
        .run(&["board", "log", "--project", project])
        .lines()
        .map(|line| {
            let fields: Vec<&str> = line.split('\t').collect();
            let [Some(id), Some(item), Some(before), Some(by)] =
                [0, 1, 3, 5].map(|index| fields.get(index))
            else {
                panic!("{line}")
            };
            [*id, *item, *before, *by].map(str::to_owned)
        })
        .collect()
}

#[test]
fn an_agents_changes_wait_for_the_person_and_are_journaled_as_its_own() {
    let scratch = common::scratch();
    let root = scratch.path();
    let repository = root.join("repository");
    common::git_repository(&repository, &[("README.md", "hello\n")]);
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
    let task = humata.run(&["item", "add", "--project", project, "Login"]);
    let task = task.trim();
    let script_path = root.join("script.json");
    std::fs::write(&script_path, script(task)).unwrap_or_else(|error| panic!("{error}"));
    let session = humata.run(&[
        "start",
        "--backend",
        "scripted",
        "--item",
        task,
        "--backend-config",
        &script_path.display().to_string(),
    ]);
    let session = session.trim();
    humata.run(&["send", session, "go", "--wait"]);

    // Proposed, but nothing on the board changed.
    assert_eq!(
        items(&humata, project),
        [("Login".to_owned(), "none".to_owned())]
    );
    let proposals = humata.run(&["proposal", "list", session]);
    let ids: Vec<&str> = proposals
        .lines()
        .filter_map(|line| line.split('\t').next())
        .collect();
    assert_eq!(ids.len(), 2, "{proposals}");
    for id in &ids {
        humata.run(&["proposal", "apply", session, id]);
    }
    assert_eq!(
        items(&humata, project),
        [
            ("Sign in".to_owned(), "none".to_owned()),
            ("Form".to_owned(), "none".to_owned())
        ]
    );

    // The journal has both as the session's.
    let by_session = format!("session:{session}");
    let changes = journal(&humata, project);
    assert!(
        changes[..2].iter().all(|change| change[3] == by_session),
        "{changes:?}"
    );

    // Raised from the command line, then the session's edit reverted: the
    // title goes back, the priority set since stays.
    humata.run(&["item", "edit", task, "--priority", "high"]);
    let changes = journal(&humata, project);
    assert_eq!(changes[0][3], "command_line");
    let edit = changes
        .iter()
        .find(|change| change[1] == task && change[2] != "-" && change[3] == by_session)
        .unwrap_or_else(|| panic!("no edit by the session in {changes:?}"));
    humata.run(&["board", "revert", &edit[0]]);
    assert_eq!(
        items(&humata, project),
        [
            ("Login".to_owned(), "high".to_owned()),
            ("Form".to_owned(), "none".to_owned())
        ]
    );
    assert_eq!(journal(&humata, project)[0][3], "command_line");
}
