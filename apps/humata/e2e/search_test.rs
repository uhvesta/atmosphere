//! Search across sessions: a command a session in one project ran is found
//! by an agent in another project, through its humata tools, at the turn it
//! ran in; `humata history` has the event the hit names; and `humata
//! search` finds it too. On this Mac an agent reaches every project, as the
//! person does; it narrows by naming one.

mod common;

use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};

const SCRIPT: &str = r#"{"rules": [
    {"when": "run the tests", "steps": [
        {"step": "shell", "command": "cargo test --workspace", "output": "test result: ok. 12 passed"},
        {"step": "say", "text": "All 12 pass."}
    ]},
    {"when": "", "steps": [{"step": "say", "text": "On it."}]}
]}"#;

fn project(humata: &common::Humata, name: &str, repository: &Path) -> String {
    humata
        .run(&[
            "project",
            "add",
            "--name",
            name,
            "--repository",
            &repository.display().to_string(),
        ])
        .trim()
        .to_owned()
}

fn session(humata: &common::Humata, project: &str, script: &Path) -> String {
    humata
        .run(&[
            "start",
            "--backend",
            "scripted",
            "--project",
            project,
            "--approval",
            "everything",
            "--backend-config",
            &script.display().to_string(),
        ])
        .trim()
        .to_owned()
}

/// What `search_sessions` answers `arguments` with, called as the agent of
/// `session` calls it: through `humatad mcp`, which Humata starts for it.
fn search_as(root: &Path, session: &str, arguments: serde_json::Value) -> String {
    let mut mcp = Command::new(common::binary("HUMATAD"))
        .args(["mcp", "--socket"])
        .arg(root.join("d.sock"))
        .env("HUMATA_SESSION", session)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap_or_else(|error| panic!("couldn't run humatad mcp: {error}"));
    let messages = [
        serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {"protocolVersion": "2025-06-18", "capabilities": {},
                       "clientInfo": {"name": "test", "version": "1"}}}),
        serde_json::json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
        serde_json::json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
            "params": {"name": "search_sessions", "arguments": arguments}}),
    ];
    {
        let Some(mut stdin) = mcp.stdin.take() else {
            panic!("humatad mcp has no stdin")
        };
        for message in messages {
            writeln!(stdin, "{message}").unwrap_or_else(|error| panic!("{error}"));
        }
    }
    let output = mcp
        .wait_with_output()
        .unwrap_or_else(|error| panic!("{error}"));
    let answer = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .find(|answer| answer["id"] == 2)
        .unwrap_or_else(|| panic!("no answer to the search"));
    assert_eq!(answer["result"]["isError"], false, "{answer}");
    answer["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

#[test]
fn an_agent_in_one_project_finds_a_command_run_in_another() {
    let scratch = common::scratch();
    let root = scratch.path();
    let script = root.join("script.json");
    std::fs::write(&script, SCRIPT).unwrap_or_else(|error| panic!("{error}"));
    let [first, second] = ["first", "second"].map(|name| {
        let repository = root.join(name);
        common::git_repository(&repository, &[("README.md", "hello\n")]);
        repository
    });
    let (_daemon, humata) = common::start(root, &[]);
    let api = project(&humata, "api", &first);
    let web = project(&humata, "web", &second);
    let ran = session(&humata, &api, &script);
    humata.run(&["send", &ran, "hello", "--wait"]);
    humata.run(&["send", &ran, "run the tests", "--wait"]);

    let asking = session(&humata, &web, &script);
    let found = search_as(
        root,
        &asking,
        serde_json::json!({"query": "cargo test", "everywhere": true}),
    );
    // Hits come under the session they're in, which is named once.
    let line = found
        .lines()
        .skip_while(|line| !line.contains(&ran))
        .skip(1)
        .take_while(|line| line.starts_with(' '))
        .find(|line| line.contains("a tool call"))
        .unwrap_or_else(|| panic!("the command wasn't found under {ran}: {found}"));
    assert!(line.contains("turn 2"), "{line}");
    let event = line
        .split("event ")
        .nth(1)
        .and_then(|rest| rest.split(':').next())
        .unwrap_or_else(|| panic!("no event in {line}"));
    let history = humata.run(&["history", &ran]);
    assert!(
        history
            .lines()
            .any(|entry| entry.starts_with(event) && entry.contains("cargo test --workspace")),
        "{event} isn't the command in\n{history}"
    );

    // By default only the asking session's project is searched.
    let own = search_as(root, &asking, serde_json::json!({"query": "cargo test"}));
    assert!(!own.contains(&ran), "{own}");

    // Only the project asked about, and a time before it ran finds nothing.
    let elsewhere = search_as(
        root,
        &asking,
        serde_json::json!({"query": "cargo test", "project_id": web}),
    );
    assert!(!elsewhere.contains(&ran), "{elsewhere}");
    let earlier = search_as(
        root,
        &asking,
        serde_json::json!({"query": "cargo yesterday", "everywhere": true}),
    );
    assert!(!earlier.contains(&ran), "{earlier}");

    // The person's search finds it, and narrows to a project.
    let hits = humata.run(&["search", "cargo", "test"]);
    assert!(
        hits.lines()
            .any(|hit| hit.starts_with("tool_call\t") && hit.contains(&ran) && hit.contains(event)),
        "{hits}"
    );
    let elsewhere = humata.run(&["search", "cargo", "test", "--project", &web]);
    assert!(!elsewhere.contains(&ran), "{elsewhere}");
}
