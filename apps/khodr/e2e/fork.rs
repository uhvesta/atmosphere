//! A session on a remote host, forked at an earlier event. Each test says
//! how the daemon reaches the host.

use std::ffi::OsStr;
use std::path::Path;

use crate::common::{Khodr, pairs, start};

const SCRIPT: &str = r#"{"rules": [
    {"when": "hello", "steps": [{"step": "say", "text": "hi there"}]},
    {"when": "", "steps": [{"step": "say", "text": "something else"}]}
]}"#;

fn event_id(khodr: &Khodr, session: &str, text: &str) -> String {
    khodr
        .run(&["history", session])
        .lines()
        .find_map(|line| {
            let fields: Vec<&str> = line.splitn(3, '\t').collect();
            (fields.get(1) == Some(&"user") && fields.get(2) == Some(&text))
                .then(|| fields[0].to_owned())
        })
        .unwrap_or_else(|| panic!("no message {text:?} in {session}"))
}

/// Starts the daemon with one host, `remote`, reached by running `command`
/// (given the host's state folder), then plays a session there and forks it
/// before its second message.
pub fn fork_on_remote(root: &Path, command: Vec<String>) {
    let work = root.join("work");
    std::fs::create_dir_all(&work).unwrap_or_else(|error| panic!("{error}"));
    let script = root.join("script.json");
    std::fs::write(&script, SCRIPT).unwrap_or_else(|error| panic!("{error}"));
    let hosts = serde_json::json!({"hosts": {"remote": {"command": command}}});
    let hosts_file = root.join("hosts.json");
    std::fs::write(&hosts_file, hosts.to_string()).unwrap_or_else(|error| panic!("{error}"));

    let (_daemon, khodr) = start(root, &[OsStr::new("--hosts"), hosts_file.as_os_str()]);

    let work = work.display().to_string();
    let script = script.display().to_string();
    let session = khodr.run(&[
        "start",
        "--backend",
        "scripted",
        "--directory",
        &work,
        "--host",
        "remote",
        "--backend-config",
        &script,
    ]);
    let session = session.trim();
    assert_eq!(
        khodr.said(&["send", session, "hello", "--wait"]),
        pairs(&[("user", "hello"), ("assistant", "hi there")])
    );
    assert_eq!(
        khodr.said(&["send", session, "and then", "--wait"]),
        pairs(&[("user", "and then"), ("assistant", "something else")])
    );

    let before = event_id(&khodr, session, "and then");
    let fork = khodr.run(&["fork", session, "--before", &before, "--name", "fork"]);
    let fork = fork.trim();
    assert_eq!(
        khodr.said(&["send", fork, "hello again", "--wait"]),
        pairs(&[("user", "hello again"), ("assistant", "hi there")])
    );

    assert_eq!(
        khodr.said(&["history", fork]),
        pairs(&[
            ("user", "hello"),
            ("assistant", "hi there"),
            ("user", "hello again"),
            ("assistant", "hi there")
        ])
    );
    assert_eq!(
        khodr.said(&["history", session]),
        pairs(&[
            ("user", "hello"),
            ("assistant", "hi there"),
            ("user", "and then"),
            ("assistant", "something else")
        ])
    );

    let sessions = khodr.run(&["sessions"]);
    let fork_line = sessions
        .lines()
        .find(|line| line.starts_with(fork))
        .unwrap_or_else(|| panic!("{sessions}"));
    let fields: Vec<&str> = fork_line.split('\t').collect();
    assert_eq!(fields.get(1), Some(&"remote"));
    assert_eq!(fields.get(3), Some(&session));
}
