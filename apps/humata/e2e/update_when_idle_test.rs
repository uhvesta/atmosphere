//! A host whose humatad is another build than this Mac's is updated when
//! the daemon connects, but not while one of its sessions has a turn going:
//! that turn would be cut off. The update waits until the host is idle.
//! The daemon is made another build by a copy of it signed again under
//! another identifier, which changes its bytes but not what it does.

use std::path::Path;
use std::process::Command;

use common::Humata;
use ssh_host::{daemon_with_host, eventually, sshd};

mod common;
mod ssh_host;

const SCRIPT: &str = r#"{"rules": [
    {"when": "wait", "steps": [
        {"step": "wait_for", "path": "go"},
        {"step": "say", "text": "went"}
    ]},
    {"when": "", "steps": [{"step": "say", "text": "done"}]}
]}"#;

fn session_line(humata: &Humata, session: &str) -> String {
    humata
        .run(&["sessions"])
        .lines()
        .find(|line| line.starts_with(session))
        .map(str::to_owned)
        .unwrap_or_default()
}

fn git(path: &Path, arguments: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(arguments)
        .status()
        .unwrap_or_else(|error| panic!("{error}"));
    assert!(status.success(), "git {arguments:?}");
}

/// Whether the host's humatad is `build`, byte for byte.
fn runs(folder: &Path, build: &Path) -> bool {
    let ours = std::fs::read(build).unwrap_or_else(|error| panic!("{error}"));
    std::fs::read(folder.join("bin/humatad")).is_ok_and(|theirs| theirs == ours)
}

/// Starts the daemon again from `humatad`, with the state, socket and hosts
/// `common::start` gave the one before it.
fn restart(root: &Path, humatad: &Path) -> common::Running {
    let socket = root.join("d.sock");
    let _ = std::fs::remove_file(&socket);
    let daemon = Command::new(humatad)
        .args(["daemon", "--state"])
        .arg(root.join("daemon"))
        .arg("--cache")
        .arg(root.join("cache"))
        .arg("--socket")
        .arg(&socket)
        .arg("--hosts")
        .arg(root.join("hosts.json"))
        .spawn()
        .unwrap_or_else(|error| panic!("couldn't start humatad: {error}"));
    let daemon = common::Running(daemon);
    common::wait_for(&socket);
    daemon
}

#[test]
fn an_outdated_host_is_updated_once_its_turns_are_done() {
    let scratch = common::scratch();
    let root = scratch.path();
    let sshd = sshd(root);
    let code = root.join("code");
    common::git_repository(&code, &[("README.md", "hello\n")]);
    let origin = root.join("origin.git");
    git(
        root,
        &[
            "clone",
            "--quiet",
            "--bare",
            &code.display().to_string(),
            &origin.display().to_string(),
        ],
    );
    git(
        &code,
        &["remote", "add", "origin", &origin.display().to_string()],
    );
    let folder = root.join("far");
    let _host = ssh_host::DetachedHost(&folder);
    let (daemon, humata) = daemon_with_host(root, &sshd, "far", &folder);
    assert!(
        runs(&folder, &common::binary("HUMATAD")),
        "the install didn't put this build there"
    );
    let project = humata.run(&[
        "project",
        "add",
        "--name",
        "code",
        "--repository",
        &code.display().to_string(),
    ]);
    let script = root.join("script.json");
    std::fs::write(&script, SCRIPT).unwrap_or_else(|error| panic!("{error}"));
    let session = humata.run(&[
        "start",
        "--backend",
        "scripted",
        "--project",
        project.trim(),
        "--host",
        "far",
        "--approval",
        "everything",
        "--backend-config",
        &script.display().to_string(),
    ]);
    let session = session.trim();

    // A turn going on the host, then this Mac's daemon replaced by another
    // build, as an update of the app does.
    humata.run(&["send", session, "wait"]);
    eventually("the turn working", || {
        session_line(&humata, session).contains("WORKING")
    });
    drop(daemon);
    let newer = root.join("newer/humatad");
    std::fs::create_dir_all(root.join("newer")).unwrap_or_else(|error| panic!("{error}"));
    std::fs::copy(common::binary("HUMATAD"), &newer).unwrap_or_else(|error| panic!("{error}"));
    let signed = Command::new("codesign")
        .args(["--force", "--sign", "-", "--identifier", "humata.e2e.newer"])
        .arg(&newer)
        .status()
        .unwrap_or_else(|error| panic!("couldn't run codesign: {error}"));
    assert!(signed.success());
    let _daemon = restart(root, &newer);

    // It finds the host outdated but busy: the host is left as it is, and
    // the turn keeps going.
    eventually("the turn still working after reconnecting", || {
        session_line(&humata, session).contains("WORKING")
    });
    assert!(!runs(&folder, &newer), "the host was updated mid-turn");

    // Once the turn is done, the host is updated, and the session carries on.
    let go = humata.working_directory(session).join("go");
    std::fs::write(&go, "").unwrap_or_else(|error| panic!("{error}"));
    eventually("the host updated once idle", || {
        let line = session_line(&humata, session);
        !line.contains("WORKING") && runs(&folder, &newer)
    });
    let said = humata.said(&["history", session]);
    assert!(
        said.iter()
            .any(|(kind, text)| kind == "assistant" && text == "went"),
        "{said:?}"
    );
    eventually("the session answering on the new build", || {
        ssh_host::succeeds(&humata, &["send", session, "hello", "--wait"])
    });
}
