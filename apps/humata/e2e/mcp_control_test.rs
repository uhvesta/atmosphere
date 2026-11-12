//! An agent outside Humata drives it through `humata mcp`: it creates a
//! project, starts a scripted session in it, sends it a message, approves
//! the write it asks for and reads the reply, all as MCP tool calls. Its
//! board changes are proposals, which wait until the person applies them.

mod common;

use std::io::{BufRead as _, BufReader, Write as _};
use std::process::{ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{Value, json};

const SCRIPT: &str = r#"{"rules": [
    {"when": "", "steps": [
        {"step": "write_file", "path": "NOTES.md", "contents": "Ship G.\n"},
        {"step": "say", "text": "Wrote the notes."}
    ]}
]}"#;

/// An MCP client of `humata mcp`, one request and answer at a time.
struct Mcp {
    _process: common::Running,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    next: u64,
}

impl Mcp {
    fn start(socket: &std::path::Path) -> Self {
        let mut child = Command::new(common::binary("HUMATA"))
            .arg("--socket")
            .arg(socket)
            .arg("mcp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap_or_else(|error| panic!("couldn't run humata mcp: {error}"));
        let (Some(input), Some(output)) = (child.stdin.take(), child.stdout.take()) else {
            panic!("humata mcp has no pipes")
        };
        Self {
            _process: common::Running(child),
            input,
            output: BufReader::new(output),
            next: 0,
        }
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        self.next += 1;
        let message =
            json!({"jsonrpc": "2.0", "id": self.next, "method": method, "params": params});
        writeln!(self.input, "{message}").unwrap_or_else(|error| panic!("{error}"));
        let mut line = String::new();
        self.output
            .read_line(&mut line)
            .unwrap_or_else(|error| panic!("{error}"));
        let answer: Value =
            serde_json::from_str(&line).unwrap_or_else(|error| panic!("{error}: {line}"));
        assert_eq!(answer["id"], json!(self.next), "{answer}");
        answer["result"].clone()
    }

    /// A tool's text, which must not be an error.
    fn call(&mut self, tool: &str, arguments: Value) -> String {
        let result = self.request("tools/call", json!({"name": tool, "arguments": arguments}));
        let text = result["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        assert_eq!(result["isError"], json!(false), "{tool}: {text}");
        text
    }

    /// The (kind, detail) of each event line a tool answered.
    fn events(&mut self, tool: &str, arguments: Value) -> Vec<(String, String)> {
        self.call(tool, arguments)
            .lines()
            .filter_map(|line| {
                let mut fields = line.splitn(3, '\t');
                let (_, kind, detail) = (fields.next()?, fields.next()?, fields.next()?);
                Some((kind.to_owned(), detail.to_owned()))
            })
            .collect()
    }
}

/// The first word after `prefix` in `text`.
fn after(text: &str, prefix: &str) -> String {
    let rest = text
        .split_once(prefix)
        .unwrap_or_else(|| panic!("no {prefix:?} in {text:?}"))
        .1;
    rest.split(|c: char| c.is_whitespace() || c == ';' || c == '.')
        .next()
        .unwrap_or_default()
        .to_owned()
}

#[test]
fn an_outside_agent_sets_up_and_drives_a_session_through_mcp() {
    let scratch = common::scratch();
    let root = scratch.path();
    let repository = root.join("repository");
    common::git_repository(&repository, &[("README.md", "hello\n")]);
    let (_daemon, humata) = common::start(root, &[]);
    let mut mcp = Mcp::start(&root.join("d.sock"));

    let initialized = mcp.request("initialize", json!({"protocolVersion": "2025-06-18"}));
    assert_eq!(initialized["protocolVersion"], json!("2025-06-18"));
    let tools = mcp.request("tools/list", json!({}));
    let destructive: Vec<&str> = tools["tools"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|tool| tool["annotations"]["destructiveHint"] == json!(true))
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    assert!(destructive.contains(&"archive_session"), "{destructive:?}");

    let created = mcp.call(
        "create_project",
        json!({"name": "demo", "repositories": [repository.display().to_string()]}),
    );
    let project = after(&created, "project ");
    assert!(mcp.call("list_projects", json!({})).contains(&project));

    let started = mcp.call(
        "start_session",
        json!({"backend": "scripted", "project_id": project, "backend_config": SCRIPT}),
    );
    let session = after(&started, "session ");

    // The write waits for approval, which the outside agent sees, with the
    // request's id, and so does the inbox.
    let asked = mcp.events(
        "send_message",
        json!({"session_id": session, "text": "write the notes"}),
    );
    let (kind, detail) = asked.last().unwrap_or_else(|| panic!("no events"));
    assert_eq!(kind, "permission_requested", "{asked:?}");
    let request = detail.split(' ').next().unwrap_or_default().to_owned();
    assert!(mcp.call("list_inbox", json!({})).contains(&session));

    let approved = mcp.events(
        "approve",
        json!({"session_id": session, "request_id": request}),
    );
    let reply = ("assistant".to_owned(), "Wrote the notes.".to_owned());
    assert!(approved.contains(&reply), "{approved:?}");
    assert_eq!(approved.last().map(|(kind, _)| kind.as_str()), Some("idle"));
    assert!(
        mcp.events("read_history", json!({"session_id": session}))
            .contains(&reply)
    );
    let copy = humata.working_directory(&session);
    assert_eq!(
        std::fs::read_to_string(copy.join("NOTES.md"))
            .ok()
            .as_deref(),
        Some("Ship G.\n")
    );

    // A tool the protocol refuses says why, as an error the agent reads.
    let refused = mcp.request(
        "tools/call",
        json!({"name": "send_message", "arguments": {"session_id": "nope", "text": "hi"}}),
    );
    assert_eq!(refused["isError"], json!(true));
}

#[test]
fn an_outside_agent_proposes_board_changes_that_wait_for_the_person() {
    let scratch = common::scratch();
    let root = scratch.path();
    let repository = root.join("repository");
    common::git_repository(&repository, &[("README.md", "hello\n")]);
    let (_daemon, humata) = common::start(root, &[]);
    let mut mcp = Mcp::start(&root.join("d.sock"));
    mcp.request("initialize", json!({"protocolVersion": "2025-06-18"}));
    let created = mcp.call(
        "create_project",
        json!({"name": "demo", "repositories": [repository.display().to_string()]}),
    );
    let project = after(&created, "project ");
    let task = humata
        .run(&["item", "add", "--project", &project, "Log in"])
        .trim()
        .to_owned();

    mcp.call(
        "propose_task",
        json!({"project_id": project, "title": "Dark mode", "spec": "Follow the system."}),
    );
    mcp.call(
        "propose_spec_edit",
        json!({"project_id": project, "item_id": task, "spec": "Email and password."}),
    );
    // Nothing reached the board.
    assert!(
        !mcp.call("board", json!({"project_id": project}))
            .contains("Dark mode")
    );
    assert_eq!(
        mcp.call("read_spec", json!({"project_id": project, "item_id": task})),
        "Log in has no spec yet."
    );

    // The person sees both waiting, and applies them.
    let listed = humata.run(&["proposal", "list", "--project", &project]);
    let pending: Vec<&str> = listed
        .lines()
        .filter(|line| line.contains("\tpending\t"))
        .filter_map(|line| line.split('\t').next())
        .collect();
    assert_eq!(pending.len(), 2, "{listed}");
    for proposal in &pending {
        humata.run(&["proposal", "apply", "", proposal]);
    }
    assert!(
        mcp.call("board", json!({"project_id": project}))
            .contains("Dark mode")
    );
    assert_eq!(
        mcp.call("read_spec", json!({"project_id": project, "item_id": task})),
        "Email and password."
    );
    // Each is decided once.
    humata.fails(&["proposal", "apply", "", pending[0]]);

    // A proposal that names no title is refused, and says why.
    let refused = mcp.request(
        "tools/call",
        json!({"name": "propose_task", "arguments": {"project_id": project, "title": " "}}),
    );
    assert_eq!(refused["isError"], json!(true));
}
