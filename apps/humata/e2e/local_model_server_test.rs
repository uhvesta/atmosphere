//! A local-model session on this Mac runs on Humata's own model server,
//! as the app serves it: the model calls a tool, which waits for a person's
//! approval and writes the file, then it replies. The model here is
//! scripted; the server, humatad and humata are the real binaries.

use std::io::{BufRead as _, BufReader};
use std::process::{Command, Stdio};

mod common;

const SCRIPT: &str = r##"[
    {"call": "write_file", "arguments": {"path": "notes.md", "contents": "# Notes\n"}},
    {"answer": "I wrote notes.md."}
]"##;

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

#[test]
fn a_local_model_session_calls_a_tool_through_the_apps_server() {
    let scratch = common::scratch();
    let root = scratch.path();
    let work = root.join("work");
    std::fs::create_dir_all(&work).unwrap_or_else(|error| panic!("{error}"));
    let script = root.join("steps.json");
    std::fs::write(&script, SCRIPT).unwrap_or_else(|error| panic!("{error}"));

    let mut server = Command::new(common::binary("HUMATA_MODEL_SERVER"))
        .arg("--script")
        .arg(&script)
        .stdout(Stdio::piped())
        .spawn()
        .unwrap_or_else(|error| panic!("couldn't start the model server: {error}"));
    let stdout = server.stdout.take().unwrap_or_else(|| panic!("no stdout"));
    let _server = common::Running(server);
    let mut port = String::new();
    BufReader::new(stdout)
        .read_line(&mut port)
        .unwrap_or_else(|error| panic!("{error}"));
    let port = port.trim().to_owned();
    assert!(port.parse::<u16>().is_ok(), "the server printed {port:?}");

    let (_daemon, humata) = common::start(root, &[]);
    humata.run(&["model-server", &port]);
    let session = humata.run(&[
        "start",
        "--backend",
        "local",
        "--directory",
        &work.display().to_string(),
        "--model",
        "scripted",
    ]);
    let session = session.trim();

    let asked = events(&humata.run(&["send", session, "write the notes", "--wait"]));
    let (kind, detail) = asked.last().unwrap_or_else(|| panic!("no events"));
    assert_eq!(kind, "permission_requested", "{asked:?}");
    let request = detail.split(' ').next().unwrap_or_default().to_owned();
    assert!(!work.join("notes.md").exists());

    let approved = events(&humata.run(&["approve", session, &request, "--wait"]));
    assert_eq!(
        std::fs::read_to_string(work.join("notes.md"))
            .ok()
            .as_deref(),
        Some("# Notes\n")
    );
    assert!(
        approved.contains(&("assistant".to_owned(), "I wrote notes.md.".to_owned())),
        "{approved:?}"
    );
    assert_eq!(approved.last().map(|(kind, _)| kind.as_str()), Some("idle"));
}
