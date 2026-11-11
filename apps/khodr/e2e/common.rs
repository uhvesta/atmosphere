//! The real daemon and CLI, playing a session on a remote host and forking
//! it at an earlier event. Each test says how the daemon reaches the host.

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

const SCRIPT: &str = r#"{"rules": [
    {"when": "hello", "steps": [{"step": "say", "text": "hi there"}]},
    {"when": "", "steps": [{"step": "say", "text": "something else"}]}
]}"#;

pub fn binary(variable: &str) -> PathBuf {
    let path =
        std::env::var_os(variable).unwrap_or_else(|| panic!("set {variable} to the binary's path"));
    std::fs::canonicalize(&path).unwrap_or_else(|error| panic!("{variable}: {error}"))
}

/// Stops a process when the test ends, passed or not.
pub struct Running(pub Child);

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct Khodr {
    cli: PathBuf,
    socket: PathBuf,
}

impl Khodr {
    fn run(&self, arguments: &[&str]) -> String {
        let output = Command::new(&self.cli)
            .arg("--socket")
            .arg(&self.socket)
            .args(arguments)
            .output()
            .unwrap_or_else(|error| panic!("couldn't run khodr: {error}"));
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        assert!(
            output.status.success(),
            "khodr {arguments:?} failed: {}\n{stdout}",
            String::from_utf8_lossy(&output.stderr)
        );
        stdout
    }

    /// The (kind, detail) of each line `khodr` printed for events.
    fn said(&self, arguments: &[&str]) -> Vec<(String, String)> {
        self.run(arguments)
            .lines()
            .filter_map(|line| {
                let mut fields = line.splitn(3, '\t');
                let (_, kind, detail) = (fields.next()?, fields.next()?, fields.next()?);
                matches!(kind, "user" | "assistant").then(|| (kind.to_owned(), detail.to_owned()))
            })
            .collect()
    }

    fn event_id(&self, session: &str, text: &str) -> String {
        self.run(&["history", session])
            .lines()
            .find_map(|line| {
                let fields: Vec<&str> = line.splitn(3, '\t').collect();
                (fields.get(1) == Some(&"user") && fields.get(2) == Some(&text))
                    .then(|| fields[0].to_owned())
            })
            .unwrap_or_else(|| panic!("no message {text:?} in {session}"))
    }
}

pub fn wait_for(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "{} never appeared",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn pairs(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(kind, text)| ((*kind).to_owned(), (*text).to_owned()))
        .collect()
}

/// A scratch folder short enough for a socket path, which is limited to
/// about 100 bytes, so not under Bazel's long test folder.
pub fn scratch() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("khodr")
        .tempdir_in("/tmp")
        .unwrap_or_else(|error| panic!("{error}"))
}

/// Starts the daemon with one host, `remote`, reached by running `command`
/// (given the host's state folder), then plays a session there and forks it
/// before its second message.
pub fn fork_on_remote(root: &Path, command: Vec<String>) {
    let khodrd = binary("KHODRD");
    let socket = root.join("d.sock");
    let work = root.join("work");
    std::fs::create_dir_all(&work).unwrap_or_else(|error| panic!("{error}"));
    let script = root.join("script.json");
    std::fs::write(&script, SCRIPT).unwrap_or_else(|error| panic!("{error}"));
    let hosts = serde_json::json!({"hosts": {"remote": {"command": command}}});
    let hosts_file = root.join("hosts.json");
    std::fs::write(&hosts_file, hosts.to_string()).unwrap_or_else(|error| panic!("{error}"));

    let daemon = Command::new(&khodrd)
        .args(["daemon", "--state"])
        .arg(root.join("daemon"))
        .arg("--socket")
        .arg(&socket)
        .arg("--hosts")
        .arg(&hosts_file)
        .spawn()
        .unwrap_or_else(|error| panic!("couldn't start khodrd: {error}"));
    let _daemon = Running(daemon);
    wait_for(&socket);
    let khodr = Khodr {
        cli: binary("KHODR"),
        socket,
    };

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

    let before = khodr.event_id(session, "and then");
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
